//! Значок программы для строки микшера.
//!
//! Берём его оттуда же, откуда берут панель задач, диспетчер задач и системный
//! микшер, — из самого exe. У сессии есть своё поле `GetIconPath`, но у обычных
//! программ оно пустое: его заполняют единицы, и полагаться на него нельзя.
//!
//! ВАЖНО: **наружу уходят сами точки, а не файл картинки.** Кодировать PNG
//! пришлось бы своим сжатием или новой зависимостью, а BMP с прозрачностью
//! webview рисует не везде одинаково. Точки же рисуются `putImageData` в
//! `<canvas>` — ни формата, ни библиотеки.
//!
//! Дорого тут одно — сам поход в оболочку за значком, поэтому зовётся это
//! только для новых имён, а найденное держится в карте у потока звука.

#[cfg(windows)]
use std::ffi::c_void;

use serde::Serialize;
#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::HWND;
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
};
#[cfg(windows)]
use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};

/// Значок в виде точек RGBA, готовых к `putImageData`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Icon {
    pub width: u32,
    pub height: u32,
    /// Точки RGBA подряд, в base64.
    pub pixels: String,
}

/// Значок программы по пути к её exe.
///
/// `None` — значка нет или оболочка его не отдала. Строка микшера от этого не
/// пропадает: у неё остаётся имя.
#[cfg(windows)]
pub fn of(path: &str) -> Option<Icon> {
    let icon = load(path)?;
    let picture = pixels(icon);

    // Значок наш, и вернуть его системе обязаны мы
    unsafe {
        let _ = DestroyIcon(icon);
    }

    picture
}

/// Просит у оболочки крупный значок файла.
#[cfg(windows)]
fn load(path: &str) -> Option<HICON> {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut info = SHFILEINFOW::default();

    let outcome = unsafe {
        SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            Default::default(),
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        )
    };

    (outcome != 0 && !info.hIcon.is_invalid()).then_some(info.hIcon)
}

/// Точки значка: BGRA от Windows, RGBA наружу.
#[cfg(windows)]
fn pixels(icon: HICON) -> Option<Icon> {
    unsafe {
        let mut info = ICONINFO::default();
        GetIconInfo(icon, &mut info).ok()?;

        let colour = info.hbmColor;
        let mask = info.hbmMask;

        let picture = from_bitmap(colour);

        // Оба битмапа отдаёт `GetIconInfo`, и убирать их за собой нам
        let _ = DeleteObject(HGDIOBJ(colour.0));
        let _ = DeleteObject(HGDIOBJ(mask.0));

        picture
    }
}

/// Разбирает цветной битмап значка в точки.
#[cfg(windows)]
unsafe fn from_bitmap(bitmap: windows::Win32::Graphics::Gdi::HBITMAP) -> Option<Icon> {
    unsafe {
        let mut shape = BITMAP::default();
        let read = GetObjectW(
            HGDIOBJ(bitmap.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(std::ptr::addr_of_mut!(shape).cast::<c_void>()),
        );
        if read == 0 || shape.bmWidth <= 0 || shape.bmHeight <= 0 {
            return None;
        }

        let width = shape.bmWidth as u32;
        let height = shape.bmHeight as u32;

        let mut header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: shape.bmWidth,
                // Со знаком минус строки идут сверху вниз — так же, как их
                // ждёт `putImageData`
                biHeight: -shape.bmHeight,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bytes = vec![0u8; (width * height * 4) as usize];

        let screen = GetDC(None);
        let lines = GetDIBits(
            screen,
            bitmap,
            0,
            height,
            Some(bytes.as_mut_ptr().cast::<c_void>()),
            &mut header,
            DIB_RGB_COLORS,
        );
        ReleaseDC(Some(HWND::default()), screen);

        if lines == 0 {
            return None;
        }

        recolour(&mut bytes);

        Some(Icon {
            width,
            height,
            pixels: base64(&bytes),
        })
    }
}

/// BGRA от Windows в RGBA для холста.
///
/// Заодно спасаем значки без альфы: у старых она сплошь нулевая, и такой
/// значок нарисовался бы пустым местом.
fn recolour(bytes: &mut [u8]) {
    let opaque = bytes.chunks_exact(4).any(|pixel| pixel[3] != 0);

    for pixel in bytes.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        if !opaque {
            pixel[3] = 255;
        }
    }
}

#[cfg(not(windows))]
pub fn of(_path: &str) -> Option<Icon> {
    None
}

/// Точки едут в окно строкой, а её надо чем-то закодировать.
///
/// Свои двадцать строк вместо зависимости: алфавит тут постоянный, а
/// единственный пользователь — этот файл.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);

    for chunk in bytes.chunks(3) {
        let triple = u32::from(chunk[0]) << 16
            | u32::from(chunk.get(1).copied().unwrap_or(0)) << 8
            | u32::from(chunk.get(2).copied().unwrap_or(0));

        text.push(ALPHABET[(triple >> 18 & 63) as usize] as char);
        text.push(ALPHABET[(triple >> 12 & 63) as usize] as char);
        text.push(if chunk.len() > 1 {
            ALPHABET[(triple >> 6 & 63) as usize] as char
        } else {
            '='
        });
        text.push(if chunk.len() > 2 {
            ALPHABET[(triple & 63) as usize] as char
        } else {
            '='
        });
    }

    text
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::of;
    use super::{base64, recolour};

    /// Кодировщик свой, поэтому проверяется на известных строках — включая обе
    /// длины с хвостом, где и живут все ошибки такого кода.
    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0, 255, 128]), "AP+A");
    }

    /// Windows отдаёт точки в BGRA, холст ждёт RGBA.
    #[test]
    fn blue_and_red_swap_places() {
        let mut bytes = vec![10, 20, 30, 255];
        recolour(&mut bytes);
        assert_eq!(bytes, vec![30, 20, 10, 255]);
    }

    /// Значок без альфы у старых программ пришёл бы пустым местом.
    #[test]
    fn an_icon_without_alpha_stays_visible() {
        let mut bytes = vec![10, 20, 30, 0, 40, 50, 60, 0];
        recolour(&mut bytes);

        assert_eq!(bytes[3], 255);
        assert_eq!(bytes[7], 255);
    }

    /// А вот у значка с альфой её трогать нельзя: прозрачные углы обязаны
    /// остаться прозрачными.
    #[test]
    fn a_real_alpha_channel_is_left_alone() {
        let mut bytes = vec![10, 20, 30, 0, 40, 50, 60, 255];
        recolour(&mut bytes);

        assert_eq!(bytes[3], 0);
        assert_eq!(bytes[7], 255);
    }

    /// Живая проверка: значок собственного exe читается и он не пустой.
    ///
    /// Запуск: `cargo test --manifest-path src-tauri/Cargo.toml -- --ignored`
    #[test]
    #[cfg(windows)]
    #[ignore = "ходит в оболочку Windows за значком"]
    fn reads_a_real_icon() {
        let path = std::env::current_exe().expect("свой путь");
        let icon = of(&path.to_string_lossy()).expect("значок нашёлся");

        assert!(icon.width >= 16 && icon.height >= 16, "{icon:?}");
        assert_eq!(
            icon.pixels.len(),
            (icon.width * icon.height * 4).div_ceil(3) as usize * 4
        );
    }
}
