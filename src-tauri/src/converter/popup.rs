//! Кадр над ручкой обрезки — своим окном поверх всех, а не блоком в карточке.
//!
//! Просьба пользователя: кадр втрое крупнее, и пусть вылезает за границу
//! виджета. Разметка webview за своё окно выйти не может, поэтому кадр
//! рисует отдельное окно Windows: попап без рамки, поверх всех, прозрачный
//! для мыши и без фокуса.
//!
//! ВАЖНО: **это не второй webview, а голое окно с GDI.** Webview, собранный
//! в рантайме, вешает приём IPC во всём приложении, а собранный
//! при старте — ещё один процесс рендера ради одной картинки. ffmpeg отдаёт
//! кадр в BMP, и `StretchDIBits` рисует его как есть — ни WIC, ни декодера
//! JPEG не нужно.
//!
//! Всё здесь — только главный поток: окно живёт в его очереди сообщений.

use std::cell::RefCell;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, COLORREF, ERROR_CLASS_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, GetMonitorInfoW, InvalidateRect, MonitorFromPoint, StretchDIBits,
    BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, MONITORINFO, MONITOR_DEFAULTTONEAREST, PAINTSTRUCT, SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, RegisterClassW, SetLayeredWindowAttributes, SetWindowPos,
    ShowWindow, HWND_TOPMOST, LWA_ALPHA, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WM_ERASEBKGND,
    WM_PAINT, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};

const CLASS: PCWSTR = w!("WindgetsFramePopup");

/// Зазор между кадром и ручкой, в физических точках при масштабе 100 %.
const GAP: i32 = 8;

/// Цвет тонкой рамки DWM вокруг кадра: `0x00BBGGRR`, светлый на тёмном.
const BORDER: u32 = 0x0050_5050;

/// Где ручка на экране, в физических точках: середина по горизонтали, верх и
/// низ. Кадр встаёт над верхом, а не влезет — под низом.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Anchor {
    pub x: i32,
    pub top: i32,
    pub bottom: i32,
}

/// Размеры картинки и где в файле начинаются её точки. Заголовки BMP —
/// `BITMAPFILEHEADER` в 14 байт и за ним `BITMAPINFOHEADER`.
pub fn bmp_parts(bytes: &[u8]) -> Option<(i32, i32, usize)> {
    if bytes.len() < 54 || &bytes[..2] != b"BM" {
        return None;
    }
    let read = |at: usize| i32::from_le_bytes(bytes[at..at + 4].try_into().expect("четыре байта"));
    let offset = usize::try_from(read(10)).ok()?;
    let (width, height) = (read(18), read(22).checked_abs()?);
    let planes = u16::from_le_bytes(bytes[26..28].try_into().ok()?);
    let bits = u16::from_le_bytes(bytes[28..30].try_into().ok()?);
    // ffmpeg отдаёт несжатый BMP без палитры. Другой заголовок или сжатие
    // нельзя передавать GDI как BITMAPINFOHEADER с сырыми строками.
    if read(14) != 40 || planes != 1 || !matches!(bits, 24 | 32) || read(30) != 0
        || width <= 0 || height == 0 || offset < 54
    {
        return None;
    }
    let row = usize::try_from(width).ok()?.checked_mul(usize::from(bits))?
        .checked_add(31)?.checked_div(32)?.checked_mul(4)?;
    let size = row.checked_mul(usize::try_from(height).ok()?)?;
    (offset.checked_add(size)? <= bytes.len()).then_some((width, height, offset))
}

/// Копия заголовка в выровненную структуру: в BMP он начинается с байта 14.
/// Вызывается только после проверки bmp_parts.
fn bitmap_info(bytes: &[u8]) -> BITMAPINFO {
    let mut info = BITMAPINFO::default();
    // Заголовок уже проверен: ровно 40 байт без палитры и масок цвета.
    unsafe {
        std::ptr::copy_nonoverlapping(
            bytes[14..54].as_ptr(),
            std::ptr::addr_of_mut!(info.bmiHeader).cast::<u8>(),
            std::mem::size_of::<BITMAPINFOHEADER>(),
        );
    }
    info
}

/// Где встать кадру размером `width`×`height`: над ручкой по её середине,
/// не вылезая за рабочую область монитора. Сверху места нет — под ручкой.
pub fn place(anchor: Anchor, width: i32, height: i32, area: RECT, gap: i32) -> (i32, i32) {
    let x = (anchor.x - width / 2).clamp(area.left, (area.right - width).max(area.left));
    let above = anchor.top - gap - height;
    let y = if above >= area.top { above } else { (anchor.bottom + gap).min(area.bottom - height).max(area.top) };
    (x, y)
}

struct Popup {
    window: HWND,
    image: Vec<u8>,
}

thread_local! {
    static POPUP: RefCell<Option<Popup>> = const { RefCell::new(None) };
}

/// Показывает кадр над ручкой. Окно собирается при первом показе — голое
/// окно Windows, в отличие от webview, собирать в рантайме безопасно.
pub fn show(image: Vec<u8>, anchor: Anchor, scale: f64) {
    let Some((width, height, _)) = bmp_parts(&image) else {
        return;
    };
    let Some(window) = window() else {
        return;
    };

    POPUP.with(|slot| {
        if let Some(popup) = slot.borrow_mut().as_mut() {
            popup.image = image;
        }
    });

    unsafe {
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let monitor = MonitorFromPoint(POINT { x: anchor.x, y: anchor.top }, MONITOR_DEFAULTTONEAREST);
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return;
        }
        let gap = (f64::from(GAP) * scale).round() as i32;
        let (x, y) = place(anchor, width, height, info.rcWork, gap);
        let _ = InvalidateRect(Some(window), None, false);
        let _ = SetWindowPos(window, Some(HWND_TOPMOST), x, y, width, height, SWP_NOACTIVATE | SWP_SHOWWINDOW);
    }
}

pub fn hide() {
    POPUP.with(|slot| {
        if let Some(popup) = slot.borrow_mut().as_mut() {
            popup.image = Vec::new();
            unsafe {
                let _ = ShowWindow(popup.window, SW_HIDE);
            }
        }
    });
}

/// Окно попапа — собранное однажды.
fn window() -> Option<HWND> {
    if let Some(window) = POPUP.with(|slot| slot.borrow().as_ref().map(|popup| popup.window)) {
        return Some(window);
    }

    let window = unsafe { create() }.map_err(|error| eprintln!("[конвертер] окно кадра не собралось: {error}")).ok()?;
    POPUP.with(|slot| *slot.borrow_mut() = Some(Popup { window, image: Vec::new() }));
    Some(window)
}

unsafe fn create() -> Result<HWND, String> {
    let class = WNDCLASSW { lpfnWndProc: Some(procedure), lpszClassName: CLASS, ..Default::default() };
    if RegisterClassW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
        return Err("класс не зарегистрировался".into());
    }

    // Прозрачное для мыши (`WS_EX_TRANSPARENT` со слоем): кадр встаёт над
    // соседними окнами и не должен отнимать у них ни нажатия, ни фокус
    let window = CreateWindowExW(
        WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
        CLASS,
        w!(""),
        WS_POPUP,
        0,
        0,
        0,
        0,
        None,
        None,
        Some(HINSTANCE::default()),
        None,
    )
    .map_err(|error| error.to_string())?;

    SetLayeredWindowAttributes(window, COLORREF(0), 255, LWA_ALPHA).map_err(|error| error.to_string())?;
    // Углы и рамка — как у виджетов: скругление и тонкая светлая линия DWM
    let corners = DWMWCP_ROUND;
    let _ = DwmSetWindowAttribute(
        window,
        DWMWA_WINDOW_CORNER_PREFERENCE,
        std::ptr::addr_of!(corners).cast(),
        std::mem::size_of_val(&corners) as u32,
    );
    let border = BORDER;
    let _ = DwmSetWindowAttribute(
        window,
        DWMWA_BORDER_COLOR,
        std::ptr::addr_of!(border).cast(),
        std::mem::size_of_val(&border) as u32,
    );
    Ok(window)
}

/// # Safety
///
/// Зовёт только Windows со своими же сообщениями.
unsafe extern "system" fn procedure(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        // Фон не стираем: картинка закрывает окно целиком, а стирание
        // мигнуло бы чёрным между кадрами
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let context = BeginPaint(window, &mut paint);
            POPUP.with(|slot| {
                if let Some(popup) = slot.borrow().as_ref() {
                    if let Some((width, height, offset)) = bmp_parts(&popup.image) {
                        // Заголовок точек лежит сразу за заголовком файла, а
                        // высота в нём со знаком — порядок строк BMP
                        // `StretchDIBits` понимает сам
                        let info = bitmap_info(&popup.image);
                        let bits = popup.image[offset..].as_ptr().cast();
                        StretchDIBits(
                            context, 0, 0, width, height, 0, 0, width, height,
                            Some(bits), &info, DIB_RGB_COLORS, SRCCOPY,
                        );
                    }
                }
            });
            let _ = EndPaint(window, &paint);
            LRESULT(0)
        }
        _ => DefWindowProcW(window, message, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bmp(width: i32, height: i32) -> Vec<u8> {
        let row = (width as usize * 3).next_multiple_of(4);
        let mut bytes = vec![0u8; 54 + row * height.unsigned_abs() as usize];
        bytes[..2].copy_from_slice(b"BM");
        bytes[10..14].copy_from_slice(&54i32.to_le_bytes());
        bytes[14..18].copy_from_slice(&40i32.to_le_bytes());
        bytes[18..22].copy_from_slice(&width.to_le_bytes());
        bytes[22..26].copy_from_slice(&height.to_le_bytes());
        bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
        bytes[28..30].copy_from_slice(&24u16.to_le_bytes());
        bytes
    }

    #[test]
    fn a_bmp_needs_all_padded_rows_and_a_representable_height() {
        let full = bmp(2, 2);
        assert_eq!(bmp_parts(&full), Some((2, 2, 54)));
        assert_eq!(bmp_parts(&full[..full.len() - 1]), None);
        // Шесть байт точек в строке занимают восемь байт вместе с выравниванием.
        assert_eq!(bmp_parts(&full[..54 + 12]), None);
        for (at, value) in [(22, i32::MIN), (22, 0), (18, 0), (18, i32::MAX), (10, 53), (10, i32::MAX)] {
            let mut broken = full.clone();
            broken[at..at + 4].copy_from_slice(&value.to_le_bytes());
            assert_eq!(bmp_parts(&broken), None);
        }
        for (at, value) in [(14, 124i32), (26, 0), (28, 8), (30, 1)] {
            let mut broken = full.clone();
            broken[at..at + 4].copy_from_slice(&value.to_le_bytes());
            assert_eq!(bmp_parts(&broken), None);
        }
    }

    #[test]
    fn the_gdi_header_is_aligned_and_preserves_the_row_order() {
        let bytes = bmp(2, -2);
        let info = bitmap_info(&bytes);
        assert_eq!(info.bmiHeader.biWidth, 2);
        assert_eq!(info.bmiHeader.biHeight, -2);
        assert_eq!(info.bmiHeader.biBitCount, 24);
        assert_eq!(std::ptr::addr_of!(info) as usize % std::mem::align_of::<BITMAPINFO>(), 0);
    }

    #[test]
    fn hiding_releases_the_frame_and_the_class_can_be_reused() {
        let _turn = crate::file_drop::WINDOW_TEST_TURN.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        unsafe {
            let first = create().expect("первое окно кадра");
            let second = create().expect("класс кадра уже зарегистрирован");
            POPUP.with(|slot| *slot.borrow_mut() = Some(Popup { window: first, image: bmp(2, 2) }));
            hide();
            POPUP.with(|slot| {
                let popup = slot.borrow_mut().take().unwrap();
                assert_eq!(popup.image.capacity(), 0);
            });
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(first);
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(second);
        }
    }


    fn area() -> RECT {
        RECT { left: -1080, top: 0, right: 0, bottom: 1880 }
    }

    /// Второй монитор слева, в отрицательных координатах.
    #[test]
    fn the_frame_stands_above_the_handle_inside_the_monitor() {
        let anchor = Anchor { x: -600, top: 900, bottom: 914 };
        assert_eq!(place(anchor, 384, 216, area(), 8), (-792, 676));

        // У левого края монитора прижимается к нему, а не уходит за край
        let edge = Anchor { x: -1070, ..anchor };
        assert_eq!(place(edge, 384, 216, area(), 8).0, -1080);
        let right = Anchor { x: -5, ..anchor };
        assert_eq!(place(right, 384, 216, area(), 8).0, -384);

        // Сверху места нет — встаёт под ручку
        let high = Anchor { x: -600, top: 100, bottom: 114 };
        assert_eq!(place(high, 384, 216, area(), 8).1, 122);
    }

    #[test]
    fn a_bmp_header_is_read() {
        let mut bytes = bmp(2, -2);

        assert_eq!(bmp_parts(&bytes), Some((2, 2, 54)));

        assert_eq!(bmp_parts(b"not a bitmap"), None);
        bytes[0] = b'X';
        assert_eq!(bmp_parts(&bytes), None);
    }
}
