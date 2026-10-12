//! Окно без системной рамки — на всю его жизнь, а не до первого обращения.
//!
//! ВАЖНО: **снять `WS_CAPTION` один раз мало.** tao собирает стиль окна заново
//! при каждой смене его флагов — показе, «поверх всех», прозрачности для мыши,
//! растягиваемости (`WindowFlags::to_window_styles`), и заголовок в этой сборке
//! стоит всегда, даже при `decorations(false)`. Прежде стиль снимали вслед за
//! каждым известным обращением к окну, но `show()` мимо этого списка проходил,
//! и заголовок возвращался.
//!
//! Виден он был из-за нашего же `DWMNCRP_DISABLED`: запрет DWM рисовать
//! неклиентскую часть отдаёт её `DefWindowProc`, а тот рисует заголовок
//! по-старому — светлой полосой с названием окна и кнопками — при каждой
//! активации и прямо поверх клиентской части, под webview. Сквозь
//! полупрозрачную карточку он и проступал; плотная карточка его лишь прятала.
//!
//! Поэтому здесь подкласс на всё время жизни окна. Предложенный стиль он правит
//! в `WM_STYLECHANGING`, то есть до записи, — заголовок не появляется даже на
//! мгновение, — а неклиентскую часть не даёт рисовать ни при активации, ни при
//! захвате окна. DWM при этом включён.
//!
//! Такая же полоса с «Windgets» или «about:blank» — не отсюда: это
//! WebView2 Runtime 144.0.3719.82 рисовал заголовок в своих окнах у прозрачной
//! страницы (WebView2Feedback #5492). Лечится обновлением Runtime, не кодом.

use tauri::WebviewWindow;
use windows::Win32::Foundation::{
    GetLastError, SetLastError, HWND, LPARAM, LRESULT, WIN32_ERROR, WPARAM,
};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMNCRP_ENABLED, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
    DWMWA_NCRENDERING_POLICY, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DWMWINDOWATTRIBUTE,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, GWL_STYLE, PRF_NONCLIENT,
    STYLESTRUCT, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    WINDOW_LONG_PTR_INDEX, WM_NCACTIVATE, WM_NCCALCSIZE, WM_NCDESTROY, WM_NCPAINT, WM_PRINT,
    WM_STYLECHANGING, WS_CAPTION, WS_EX_CLIENTEDGE, WS_EX_DLGMODALFRAME, WS_EX_STATICEDGE,
    WS_EX_WINDOWEDGE,
};

/// Своё значение подкласса, не то, что у прилипания: снимаются они порознь.
const SUBCLASS: usize = 0x6672_616d;

/// Стиль без заголовка.
///
/// ВАЖНО: **снимаем ровно `WS_CAPTION`, а `WS_THICKFRAME` не трогаем** — ни
/// снимая, ни добавляя. Им окно тянется за края, и без него `DefWindowProc` не
/// пускает `SC_SIZE`: снятый заодно с заголовком, он однажды уже лишил виджеты
/// изменения размера целиком. На нём же держится замок размера
/// (`set_resizable`), так что и вернуть его запертому окну было бы поломкой.
fn without_caption(style: u32) -> u32 {
    style & !WS_CAPTION.0
}

/// Расширенный стиль без объёмных краёв: `WS_EX_WINDOWEDGE` tao ставит в
/// каждую сборку стиля. «Поверх всех», прозрачность для мыши и кнопка в
/// панели задач остаются как были.
fn without_edges(style: u32) -> u32 {
    style & !(WS_EX_WINDOWEDGE | WS_EX_CLIENTEDGE | WS_EX_DLGMODALFRAME | WS_EX_STATICEDGE).0
}

/// Ставит окну подкласс, снимает уже стоящий заголовок и настраивает DWM.
///
/// Зовётся один раз, сразу после сборки, пока окно спрятано: до
/// восстановления места, до флагов из настроек и до первого показа. Дальше
/// каждую запись стиля правит подкласс, и снимать рамку заново незачем.
///
/// Только из главного потока: `SetWindowSubclass` не вешается на окно чужого
/// потока.
pub fn install(window: &WebviewWindow) -> Result<(), String> {
    let handle = window.hwnd().map_err(|error| error.to_string())?;
    // Окно только что собрано этим же потоком и живо
    unsafe { install_on(handle) }
}

/// # Safety
///
/// `handle` — живое окно, созданное вызывающим потоком.
unsafe fn install_on(handle: HWND) -> Result<(), String> {
    if !SetWindowSubclass(handle, Some(procedure), SUBCLASS, 0).as_bool() {
        return Err("подкласс не поставился".into());
    }

    // Стиль, записанный при сборке, подкласс ещё не видел. Запись идёт уже
    // через него, так что поправлено будет дважды — вреда в этом нет
    let style = GetWindowLongPtrW(handle, GWL_STYLE) as u32;
    set_style(handle, GWL_STYLE, without_caption(style))?;
    let extended = GetWindowLongPtrW(handle, GWL_EXSTYLE) as u32;
    set_style(handle, GWL_EXSTYLE, without_edges(extended))?;

    // ВАЖНО: **DWM не выключаем.** Выключенный отдаёт неклиентскую часть
    // `DefWindowProc` — отсюда и был классический заголовок. Включённому
    // рисовать нечего: клиентская часть занимает окно целиком
    set_attribute(handle, DWMWA_NCRENDERING_POLICY, DWMNCRP_ENABLED);
    // Включённый DWM обводит окно Windows 11 линией в точку по краю
    set_attribute(handle, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE);
    // У окна без рамки углы прямые, и они торчали бы из-под скруглённой
    // карточки
    set_attribute(handle, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND);

    // Иначе Windows пересчитает рамку только к следующему изменению окна
    SetWindowPos(
        handle,
        None,
        0,
        0,
        0,
        0,
        SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
    )
    .map_err(|error| format!("рамка не пересчиталась: {error}"))
}

/// Пишет стиль и не глотает отказ: прежнее снятие рамки молчало, и окно с
/// заголовком выглядело бы исправным.
///
/// # Safety
///
/// `handle` — живое окно вызывающего потока, `index` — `GWL_STYLE` или
/// `GWL_EXSTYLE`.
unsafe fn set_style(handle: HWND, index: WINDOW_LONG_PTR_INDEX, style: u32) -> Result<(), String> {
    // Ноль в ответ значит и отказ, и прежний стиль «пусто». Различает их
    // только код ошибки, поэтому его сперва обнуляют
    SetLastError(WIN32_ERROR(0));
    let previous = SetWindowLongPtrW(handle, index, style as isize);
    let error = GetLastError();

    if previous == 0 && error.0 != 0 {
        return Err(format!("стиль окна не записался, ошибка {}", error.0));
    }
    Ok(())
}

/// Пишет атрибут DWM. Отказ не страшен: на Windows 10 части атрибутов просто
/// нет, как нет там и того, что они выключают.
///
/// # Safety
///
/// `handle` — живое окно, `T` — тип, который DWM ждёт у этого атрибута.
unsafe fn set_attribute<T>(handle: HWND, attribute: DWMWINDOWATTRIBUTE, value: T) {
    let _ = DwmSetWindowAttribute(
        handle,
        attribute,
        std::ptr::addr_of!(value).cast(),
        std::mem::size_of::<T>() as u32,
    );
}

/// # Safety
///
/// Зовёт только Windows, отдавая свои же сообщения. У `WM_STYLECHANGING` в
/// `lparam` лежит `STYLESTRUCT`, и правка `styleNew` на месте —
/// документированный ответ на это сообщение.
unsafe extern "system" fn procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass: usize,
    _data: usize,
) -> LRESULT {
    match message {
        // Правим до записи и отдаём дальше по цепочке уже поправленным
        WM_STYLECHANGING if lparam.0 != 0 => {
            let change = &mut *(lparam.0 as *mut STYLESTRUCT);
            // Индексы стиля отрицательные и приходят в беззнаковом `wparam`
            match wparam.0 as i32 {
                index if index == GWL_STYLE.0 => change.styleNew = without_caption(change.styleNew),
                index if index == GWL_EXSTYLE.0 => change.styleNew = without_edges(change.styleNew),
                _ => {}
            }
        }

        // С `TRUE` рамку считает tao: окну без рамки он отдаёт клиенту всё окно
        // и помнит о рабочей области у развёрнутого. С `FALSE` он зовёт
        // `DefWindowProc`, а тот отнял бы у клиента поля классической рамки.
        // Ноль без правки прямоугольника значит «клиент равен окну»
        WM_NCCALCSIZE if wparam.0 == 0 => return LRESULT(0),

        // Рисовать неклиентскую часть некому и незачем
        WM_NCPAINT => return LRESULT(0),

        // Сама активация обязана дойти до tao — на ней держится фокус. `-1` в
        // `lparam` запрещает `DefWindowProc` перерисовать при этом рамку, и
        // только
        WM_NCACTIVATE => return DefSubclassProc(window, message, wparam, LPARAM(-1)),

        // Захват окна — `PrintWindow`, программы снимков — просит нарисовать
        // и неклиентскую часть и получил бы тот самый классический заголовок.
        // Клиента с webview рисуем, рамку нет
        WM_PRINT => {
            let flags = LPARAM(lparam.0 & !(PRF_NONCLIENT as isize));
            return DefSubclassProc(window, message, wparam, flags);
        }

        WM_NCDESTROY => {
            let _ = RemoveWindowSubclass(window, Some(procedure), SUBCLASS);
        }

        _ => {}
    }

    DefSubclassProc(window, message, wparam, lparam)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_MAXIMIZEBOX,
        WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME, WS_VISIBLE,
    };

    #[test]
    fn the_caption_goes_but_resizing_and_the_window_menu_stay() {
        let resizable = (WS_VISIBLE | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX).0;
        assert_eq!(without_caption(resizable | WS_CAPTION.0), resizable);

        // Запертое окно растягиваемость обратно не получает
        let locked = (WS_VISIBLE | WS_SYSMENU | WS_MAXIMIZEBOX).0;
        assert_eq!(without_caption(locked | WS_CAPTION.0), locked);
    }

    #[test]
    fn edges_go_but_topmost_click_through_and_taskbar_stay() {
        let kept = (WS_EX_APPWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT).0;
        let edges =
            (WS_EX_WINDOWEDGE | WS_EX_CLIENTEDGE | WS_EX_DLGMODALFRAME | WS_EX_STATICEDGE).0;
        assert_eq!(without_edges(kept | edges), kept);
    }

    /// Настоящее окно, а не арифметика масок: подкласс обязан править записи
    /// стиля сам — те, что делает tao на замке, показе и «поверх всех».
    #[test]
    fn a_real_window_keeps_no_caption_through_later_style_writes() {
        use windows::core::w;
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, SendMessageW, WS_OVERLAPPEDWINDOW,
        };

        /// Своё спрятанное окно: ни виджетов, ни настроек тест не касается.
        struct Fixture(HWND);

        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = unsafe { DestroyWindow(self.0) };
            }
        }

        unsafe {
            let fixture = Fixture(
                CreateWindowExW(
                    WS_EX_WINDOWEDGE,
                    w!("STATIC"),
                    w!("frameless"),
                    WS_OVERLAPPEDWINDOW,
                    0,
                    0,
                    320,
                    240,
                    None,
                    None,
                    None,
                    None,
                )
                .expect("тестовое окно не создалось"),
            );
            let handle = fixture.0;
            install_on(handle).unwrap();

            // Так пишет стиль tao: заголовок в каждой сборке, растягиваемость
            // по замку
            for resizable in [true, false, true] {
                let mut proposed = WS_OVERLAPPEDWINDOW.0;
                if !resizable {
                    proposed &= !WS_THICKFRAME.0;
                }
                set_style(handle, GWL_STYLE, proposed).unwrap();

                let actual = GetWindowLongPtrW(handle, GWL_STYLE) as u32;
                assert_eq!(actual & WS_CAPTION.0, 0);
                assert_eq!(actual & WS_THICKFRAME.0 != 0, resizable);
                assert_ne!(actual & WS_SYSMENU.0, 0);
            }

            set_style(
                handle,
                GWL_EXSTYLE,
                (WS_EX_WINDOWEDGE | WS_EX_CLIENTEDGE | WS_EX_APPWINDOW).0,
            )
            .unwrap();
            let extended = GetWindowLongPtrW(handle, GWL_EXSTYLE) as u32;
            assert_ne!(extended & WS_EX_APPWINDOW.0, 0);
            assert_eq!(extended & WS_EX_CLIENTEDGE.0, 0);
            // `WS_EX_WINDOWEDGE` при `WS_THICKFRAME` Windows возвращает сама;
            // рисовать его не дают обработчики неклиентской части

            // Без подкласса `DefWindowProc` окна с `WS_THICKFRAME` отнял бы у
            // клиента поля рамки
            let mut client = RECT {
                left: 10,
                top: 20,
                right: 330,
                bottom: 260,
            };
            SendMessageW(
                handle,
                WM_NCCALCSIZE,
                Some(WPARAM(0)),
                Some(LPARAM(std::ptr::addr_of_mut!(client) as isize)),
            );
            assert_eq!(
                (client.left, client.top, client.right, client.bottom),
                (10, 20, 330, 260)
            );
        }
    }
}
