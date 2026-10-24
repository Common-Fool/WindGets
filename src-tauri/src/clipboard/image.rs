//! Картинки буфера через WIC: разобрать, сжать миниатюру, закодировать PNG.
//!
//! WIC стоит в Windows и знает PNG и BMP со всеми их вариантами — палитрами,
//! масками, прозрачностью V5. Своего декодера и deflate ради этого не берём.
//! Это COM: квартиру поднимает поток буфера один раз ([`enter`]), сюда из
//! потока команды не ходят.

use base64::Engine;
use sha2::{Digest, Sha256};
use windows::core::{Interface, GUID};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA,
    IWICBitmapSource, IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapEncoderNoCache,
    WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::StructuredStorage::{CreateStreamOnHGlobal, GetHGlobalFromStream};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, STREAM_SEEK_SET};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};

use super::board::Raw;

/// Длинная сторона миниатюры в точках: две колонки сетки при масштабе 130 %
/// и плотности экрана 1,5 — с запасом.
const THUMB: u32 = 160;

/// Заголовок файла BMP: WIC разбирает BMP только целым файлом.
const FILE_HEADER: usize = 14;

/// Картинка, готовая в историю.
pub struct Decoded {
    pub png: Vec<u8>,
    pub hash: [u8; 32],
    pub width: u32,
    pub height: u32,
    pub thumb: String,
}

/// Поднимает квартиру COM у потока буфера. Отказ не беда: значит, она уже
/// есть, а без неї первый же вызов WIC скажет об этом сам.
pub fn enter() {
    // SAFETY: зовётся один раз в начале потока, до любого объекта COM
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
}

fn factory() -> Result<IWICImagingFactory, String> {
    unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
        .map_err(|error| format!("WIC не поднялся: {error}"))
}

/// Разбирает картинку буфера и готовит её для истории.
///
/// PNG остаётся как пришёл — его байты и есть картинка без потерь. DIB
/// кодируется в PNG: хранить несжатые точки снимка 4K — это 33 МБ на запись.
pub fn picture(raw: &Raw, max_pixels: u64) -> Result<Decoded, String> {
    let factory = factory()?;
    let file;
    let bytes = match raw {
        Raw::Png(png) => png.as_slice(),
        Raw::Dib(dib) => {
            file = bmp_file(dib)?;
            file.as_slice()
        }
    };

    let source = decode(&factory, bytes)?;
    let (width, height) = size(&source)?;
    if width == 0 || height == 0 {
        return Err("пустая картинка".to_owned());
    }
    if u64::from(width) * u64::from(height) > max_pixels {
        return Err(format!("картинка {width}×{height} больше потолка"));
    }

    let png = match raw {
        Raw::Png(png) => png.clone(),
        Raw::Dib(_) => encode(&factory, &source, width, height)?,
    };

    let (thumb_width, thumb_height) = fit(width, height, THUMB);
    let scaler = unsafe { factory.CreateBitmapScaler() }.map_err(|error| format!("масштаб WIC: {error}"))?;
    unsafe { scaler.Initialize(&source, thumb_width, thumb_height, WICBitmapInterpolationModeFant) }
        .map_err(|error| format!("масштаб WIC: {error}"))?;
    let scaled: IWICBitmapSource = scaler.cast().map_err(|error| format!("масштаб WIC: {error}"))?;
    let thumb = encode(&factory, &scaled, thumb_width, thumb_height)?;

    Ok(Decoded {
        hash: Sha256::digest(&png).into(),
        png,
        width,
        height,
        thumb: format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(thumb)),
    })
}

/// PNG в CF_DIB для возврата в буфер: BITMAPINFOHEADER и 32-битные точки
/// снизу вверх. Сверху вниз (отрицательная высота) многие программы
/// вставляют перевёрнутым.
pub fn dib(png: &[u8]) -> Result<Vec<u8>, String> {
    let factory = factory()?;
    let source = decode(&factory, png)?;
    let (width, height) = size(&source)?;
    let stride = width as usize * 4;
    let mut pixels = vec![0u8; stride * height as usize];
    unsafe { source.CopyPixels(std::ptr::null(), stride as u32, &mut pixels) }
        .map_err(|error| format!("точки WIC: {error}"))?;

    let mut dib = Vec::with_capacity(40 + pixels.len());
    dib.extend_from_slice(&40u32.to_le_bytes()); // biSize
    dib.extend_from_slice(&(width as i32).to_le_bytes());
    dib.extend_from_slice(&(height as i32).to_le_bytes());
    dib.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    dib.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    dib.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    dib.extend_from_slice(&(pixels.len() as u32).to_le_bytes());
    dib.extend_from_slice(&[0u8; 16]); // разрешение и палитра — нули
    for row in pixels.chunks_exact(stride).rev() {
        dib.extend_from_slice(row);
    }
    Ok(dib)
}

/// Дописывает к DIB заголовок файла BMP.
///
/// Смещение точек считается по заголовку: после BITMAPINFOHEADER у
/// BI_BITFIELDS идут три маски, у палитровых — таблица цветов. У V4 и V5 маски
/// внутри заголовка.
fn bmp_file(dib: &[u8]) -> Result<Vec<u8>, String> {
    let read_u32 = |at: usize| dib.get(at..at + 4).map(|bytes| u32::from_le_bytes(bytes.try_into().expect("четыре байта")));
    let read_u16 = |at: usize| dib.get(at..at + 2).map(|bytes| u16::from_le_bytes(bytes.try_into().expect("два байта")));
    let header = read_u32(0).ok_or("короткий DIB")? as usize;
    let bits = read_u16(14).ok_or("короткий DIB")?;
    let compression = read_u32(16).ok_or("короткий DIB")?;
    let used = read_u32(32).ok_or("короткий DIB")? as usize;
    if header < 40 || header > dib.len() {
        return Err(format!("странный заголовок DIB: {header}"));
    }

    let masks = if header == 40 && compression == 3 { 12 } else { 0 };
    let colors = if used > 0 { used } else if bits <= 8 { 1 << bits } else { 0 };
    let offset = FILE_HEADER + header + masks + colors * 4;
    let total = FILE_HEADER + dib.len();

    let mut file = Vec::with_capacity(total);
    file.extend_from_slice(b"BM");
    file.extend_from_slice(&(total as u32).to_le_bytes());
    file.extend_from_slice(&[0u8; 4]);
    file.extend_from_slice(&(offset as u32).to_le_bytes());
    file.extend_from_slice(dib);
    Ok(file)
}

/// Первый кадр картинки в 32-битном BGRA.
fn decode(factory: &IWICImagingFactory, bytes: &[u8]) -> Result<IWICBitmapSource, String> {
    let stream = unsafe { factory.CreateStream() }.map_err(|error| format!("поток WIC: {error}"))?;
    unsafe { stream.InitializeFromMemory(bytes) }.map_err(|error| format!("поток WIC: {error}"))?;
    let decoder = unsafe { factory.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand) }
        .map_err(|error| format!("WIC не узнал картинку: {error}"))?;
    let frame = unsafe { decoder.GetFrame(0) }.map_err(|error| format!("кадр WIC: {error}"))?;

    let converter = unsafe { factory.CreateFormatConverter() }.map_err(|error| format!("перевод WIC: {error}"))?;
    unsafe {
        converter.Initialize(
            &frame,
            &GUID_WICPixelFormat32bppBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )
    }
    .map_err(|error| format!("перевод WIC: {error}"))?;
    converter.cast().map_err(|error| format!("перевод WIC: {error}"))
}

fn size(source: &IWICBitmapSource) -> Result<(u32, u32), String> {
    let (mut width, mut height) = (0, 0);
    unsafe { source.GetSize(&mut width, &mut height) }.map_err(|error| format!("размер WIC: {error}"))?;
    Ok((width, height))
}

/// PNG из готового источника точек.
fn encode(factory: &IWICImagingFactory, source: &IWICBitmapSource, width: u32, height: u32) -> Result<Vec<u8>, String> {
    let fail = |what: &'static str| move |error: windows::core::Error| format!("PNG WIC, {what}: {error}");
    unsafe {
        let stream = CreateStreamOnHGlobal(Default::default(), true).map_err(fail("поток"))?;
        let encoder = factory.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null()).map_err(fail("кодер"))?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache).map_err(fail("кодер"))?;

        let mut frame = None;
        let mut options = None;
        encoder.CreateNewFrame(&mut frame, &mut options).map_err(fail("кадр"))?;
        let frame = frame.ok_or("PNG WIC: кадра нет")?;
        frame.Initialize(options.as_ref()).map_err(fail("кадр"))?;
        frame.SetSize(width, height).map_err(fail("размер"))?;
        let mut format: GUID = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut format).map_err(fail("формат"))?;
        frame.WriteSource(source, std::ptr::null()).map_err(fail("точки"))?;
        frame.Commit().map_err(fail("кадр"))?;
        encoder.Commit().map_err(fail("кодер"))?;

        stream.Seek(0, STREAM_SEEK_SET, None).map_err(fail("поток"))?;
        let memory = GetHGlobalFromStream(&stream).map_err(fail("память"))?;
        let length = GlobalSize(memory);
        let pointer = GlobalLock(memory).cast::<u8>();
        if pointer.is_null() {
            return Err("PNG WIC: память не заперлась".to_owned());
        }
        // Блок бывает больше записанного: длина — по позиции конца потока
        let mut end = 0u64;
        stream.Seek(0, windows::Win32::System::Com::STREAM_SEEK_END, Some(&mut end)).map_err(fail("поток"))?;
        let bytes = std::slice::from_raw_parts(pointer, (end as usize).min(length)).to_vec();
        let _ = GlobalUnlock(memory);
        Ok(bytes)
    }
}

/// Вписывает картинку в квадрат, не увеличивая.
fn fit(width: u32, height: u32, side: u32) -> (u32, u32) {
    if width <= side && height <= side {
        return (width, height);
    }
    if width >= height {
        (side, (u64::from(height) * u64::from(side) / u64::from(width)).max(1) as u32)
    } else {
        ((u64::from(width) * u64::from(side) / u64::from(height)).max(1) as u32, side)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thumbnail_fits_without_growing() {
        assert_eq!(fit(1920, 1080, 160), (160, 90));
        assert_eq!(fit(1080, 1920, 160), (90, 160));
        assert_eq!(fit(100, 40, 160), (100, 40));
        assert_eq!(fit(10_000, 1, 160), (160, 1));
    }

    /// Круг DIB → PNG → DIB на живом WIC: точки и ориентация сохраняются.
    #[test]
    fn a_dib_survives_the_round_trip() {
        enter();
        // 2×1, BI_RGB 32 бита, снизу вверх: синяя точка и красная
        let mut dib = Vec::new();
        dib.extend_from_slice(&40u32.to_le_bytes());
        dib.extend_from_slice(&2i32.to_le_bytes());
        dib.extend_from_slice(&1i32.to_le_bytes());
        dib.extend_from_slice(&1u16.to_le_bytes());
        dib.extend_from_slice(&32u16.to_le_bytes());
        dib.extend_from_slice(&0u32.to_le_bytes());
        dib.extend_from_slice(&8u32.to_le_bytes());
        dib.extend_from_slice(&[0u8; 16]);
        dib.extend_from_slice(&[255, 0, 0, 255, 0, 0, 255, 255]);

        let decoded = picture(&Raw::Dib(dib), 1000).expect("WIC разобрал DIB");
        assert_eq!((decoded.width, decoded.height), (2, 1));
        assert!(decoded.png.starts_with(b"\x89PNG"));
        assert!(decoded.thumb.starts_with("data:image/png;base64,"));

        let back = super::dib(&decoded.png).expect("PNG обратно в DIB");
        assert_eq!(&back[40..43], &[255, 0, 0], "синий первым");
        assert_eq!(&back[44..47], &[0, 0, 255], "красный вторым");
    }

    #[test]
    fn an_oversized_picture_is_refused() {
        enter();
        let mut dib = Vec::new();
        dib.extend_from_slice(&40u32.to_le_bytes());
        dib.extend_from_slice(&2i32.to_le_bytes());
        dib.extend_from_slice(&2i32.to_le_bytes());
        dib.extend_from_slice(&1u16.to_le_bytes());
        dib.extend_from_slice(&32u16.to_le_bytes());
        dib.extend_from_slice(&0u32.to_le_bytes());
        dib.extend_from_slice(&16u32.to_le_bytes());
        dib.extend_from_slice(&[0u8; 16]);
        dib.extend_from_slice(&[0u8; 16]);
        assert!(picture(&Raw::Dib(dib), 3).is_err());
    }
}
