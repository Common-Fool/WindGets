//! Файлы из Проводника на окно конвертера — невидимой подложкой и старым
//! `WM_DROPFILES`, а не OLE.
//!
//! ВАЖНО: **приложение повышено, а Проводник нет, и OLE-перетаскивание между
//! ними Windows не пропускает** (UIPI): приёмник в повышенном процессе — это
//! COM, и Проводник до него не достучится. Курсор над карточкой просто
//! становится запрещающим — так и было на живой проверке.
//!
//! Путь в обход — старый, ещё из Windows 3.1. Окну с `WS_EX_ACCEPTFILES` без
//! OLE-приёмника OLE сам достаёт из перетаскиваемого список файлов и шлёт
//! `WM_DROPFILES`. Сообщение из процесса ниже уровнем фильтр UIPI тоже режет,
//! но его можно открыть поштучно (`ChangeWindowMessageFilterEx`) — вместе с
//! `WM_COPYDATA` и `WM_COPYGLOBALDATA`, которыми Windows переносит сам список
//! в чужой процесс.
//!
//! ВАЖНО: **на само окно конвертера это не ставится — проверено живьём.** Под
//! курсором там лежат окна WebView2 из его собственных процессов
//! (`Chrome_RenderWidgetHostHWND`, `Intermediate D3D Window`), у каждого свой
//! OLE-приёмник, и OLE находит их раньше, чем доходит до нашего окна. Снять
//! чужие приёмники нашим процессом нельзя.
//!
//! Поэтому приёмник — отдельное окно поверх конвертера: без OLE, с
//! `WS_EX_ACCEPTFILES` и почти прозрачное. Альфа 1 из 255 глазу не видна, а
//! мышь в такое окно попадает; при нуле оно пропускало бы мышь насквозь, и
//! перетаскивание вместе с ней. Стоять всегда оно не может — отняло бы у
//! карточки все нажатия, — поэтому показывается только на время чужого
//! перетаскивания над конвертером.
//!
//! Узнать о перетаскивании без OLE нечем, кроме опроса (`tick`), — таймером
//! самого окна, в главном потоке, раз в 60 мс. Пока кнопка мыши отпущена, это
//! два чтения её состояния, и только.

use std::cell::{Cell, RefCell};
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::rc::Rc;

use tauri::WebviewWindow;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, COLORREF, ERROR_CLASS_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{GetStockObject, BLACK_BRUSH, HBRUSH};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VIRTUAL_KEY, VK_LBUTTON, VK_RBUTTON};
use windows::Win32::UI::Shell::{DragFinish, DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{
    ChangeWindowMessageFilterEx, CreateWindowExW, DefWindowProcW, DestroyWindow, GetAncestor, GetCursorPos,
    GetForegroundWindow, GetGUIThreadInfo, GetWindowLongPtrW, GetWindowRect,
    GetWindowThreadProcessId, IsWindowVisible, KillTimer, RegisterClassW,
    SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow, WindowFromPoint, GA_ROOT,
    GUITHREADINFO, GUI_INMENUMODE, GUI_INMOVESIZE, GWLP_HINSTANCE, GWL_EXSTYLE, HWND_TOP, HWND_TOPMOST,
    LWA_ALPHA, MSGFLT_ALLOW, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WM_COPYDATA, WM_DESTROY,
    WM_DROPFILES, WM_TIMER, WNDCLASSW, WS_EX_ACCEPTFILES, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

/// Недокументированное, но известное с Vista: им Windows переносит блок
/// `HDROP` в чужой процесс, и без него `WM_DROPFILES` приходит пустым.
const WM_COPYGLOBALDATA: u32 = 0x0049;

const CLASS: PCWSTR = w!("WindgetsDropCatcher");

const TIMER: usize = 1;

/// Шаг опроса. Подложка встаёт не позже чем через столько после того, как
/// файл внесли над карточку, — а бросают его, задержавшись над ней.
const TICK_MS: u32 = 60;

/// Непрозрачность подложки: 1 из 255 не видна, а мышь ловит.
const ALPHA: u8 = 1;

struct Catcher {
    converter: HWND,
    overlay: HWND,
    /// Берёт ли конвертер файлы прямо сейчас: пока идёт пачка — нет, и
    /// курсор «копировать» над ним соврал бы.
    accepts: Box<dyn Fn() -> bool>,
    on_hover: Box<dyn Fn(bool)>,
    on_drop: Box<dyn Fn(Vec<PathBuf>)>,
    shown: Cell<bool>,
    was_down: Cell<bool>,
    /// Кнопку зажали над самим конвертером: это нажатие по нему или перенос
    /// его окна, а не бросок.
    pressed_here: Cell<bool>,
}

thread_local! {
    /// Всё живёт в главном потоке: и окно, и его таймер, и оконная процедура.
    static CATCHER: RefCell<Option<Rc<Catcher>>> = const { RefCell::new(None) };
}

fn catcher() -> Option<Rc<Catcher>> {
    CATCHER.with(|slot| slot.borrow().clone())
}

/// Ставит подложку над окном конвертера.
///
/// Только из главного потока: окно подложки и его таймер принадлежат ему, а
/// сообщения им разбирает цикл tao.
pub fn attach(
    window: &WebviewWindow,
    accepts: impl Fn() -> bool + 'static,
    on_hover: impl Fn(bool) + 'static,
    on_drop: impl Fn(Vec<PathBuf>) + 'static,
) -> Result<(), String> {
    if catcher().is_some() {
        return Err("подложка уже стоит".into());
    }
    let converter = window.hwnd().map_err(|error| error.to_string())?;
    // Окно конвертера собрано этим же потоком и живо
    let overlay = unsafe { create(converter)? };

    CATCHER.with(|slot| {
        *slot.borrow_mut() = Some(Rc::new(Catcher {
            converter,
            overlay,
            accepts: Box::new(accepts),
            on_hover: Box::new(on_hover),
            on_drop: Box::new(on_drop),
            shown: Cell::new(false),
            was_down: Cell::new(false),
            pressed_here: Cell::new(false),
        }));
    });

    if unsafe { SetTimer(Some(overlay), TIMER, TICK_MS, None) } == 0 {
        return Err("таймер подложки не завёлся".into());
    }
    Ok(())
}

/// Снимает подложку — перед разборкой окна конвертера, когда модуль
/// выключают. Главным потоком, как и [`attach`]: окно подложки принадлежит ему.
///
/// Без этого включённый снова конвертер остался бы без перетаскивания:
/// подложка одна на поток, и вторую [`attach`] не ставит.
pub fn detach() {
    let Some(catcher) = CATCHER.with(|slot| slot.borrow_mut().take()) else {
        return;
    };
    // Таймер гасит сама оконная процедура на `WM_DESTROY`
    unsafe {
        let _ = DestroyWindow(catcher.overlay);
    }
}

/// # Safety
///
/// `converter` — живое окно вызывающего потока.
unsafe fn create(converter: HWND) -> Result<HWND, String> {
    // Модуль — тот же, что у окна tao: отдельная возможность крейта ради
    // `GetModuleHandleW` не нужна
    let instance = HINSTANCE(GetWindowLongPtrW(converter, GWLP_HINSTANCE) as *mut std::ffi::c_void);

    let class = WNDCLASSW {
        lpfnWndProc: Some(procedure),
        hInstance: instance,
        // Слой с альфой 1 закрашивается фоном класса: чёрное в 1/255 глазу
        // не видно, а незакрашенное окно показало бы мусор
        hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
        lpszClassName: CLASS,
        ..Default::default()
    };
    // Класс остаётся зарегистрированным и после снятия подложки: конвертер,
    // выключенный и включённый снова, находит его уже на месте
    if RegisterClassW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
        return Err("класс подложки не зарегистрировался".into());
    }

    // ВАЖНО: **без владельца.** Для попапа `GetParent` отдаёт владельца, и
    // OLE, поднимаясь от подложки в поисках приёмника, дошёл бы до окна
    // конвертера — а на нём свои OLE-приёмники от tao, с которыми упёрся бы в
    // тот же запрет. Над конвертером подложку ставит `show`, местом в очереди
    // окон. Без кнопки в панели задач, фокуса не берёт
    let overlay = CreateWindowExW(
        WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_ACCEPTFILES,
        CLASS,
        w!(""),
        WS_POPUP,
        0,
        0,
        0,
        0,
        None,
        None,
        Some(instance),
        None,
    )
    .map_err(|error| format!("подложка не создалась: {error}"))?;

    SetLayeredWindowAttributes(overlay, COLORREF(0), ALPHA, LWA_ALPHA)
        .map_err(|error| format!("прозрачность подложки не встала: {error}"))?;
    for message in [WM_DROPFILES, WM_COPYDATA, WM_COPYGLOBALDATA] {
        ChangeWindowMessageFilterEx(overlay, message, MSGFLT_ALLOW, None)
            .map_err(|error| format!("сообщение {message:#x} не открылось: {error}"))?;
    }
    Ok(overlay)
}

/// Зажата ли кнопка физически. Обе кнопки: у левши основная — правая, а
/// перетаскивание правой в Проводнике тоже бывает.
unsafe fn held(key: VIRTUAL_KEY) -> bool {
    GetAsyncKeyState(i32::from(key.0)) as u16 & 0x8000 != 0
}

/// Курсор над видимой частью конвертера — или над самой подложкой. Спрашиваем
/// `WindowFromPoint`, а не прямоугольник: окно, перекрывшее карточку, своё
/// перетаскивание получит само.
unsafe fn over_converter(catcher: &Catcher, cursor: POINT) -> bool {
    if !IsWindowVisible(catcher.converter).as_bool() {
        return false;
    }
    let root = GetAncestor(WindowFromPoint(cursor), GA_ROOT);
    root == catcher.converter || root == catcher.overlay
}

/// Кнопку держит чужое приложение, и держит не ради переноса окна или меню.
///
/// Перенос чужого окна над конвертером — тоже зажатая кнопка над ним, но не
/// бросок: такой поток стоит в `GUI_INMOVESIZE`. Своё приложение — это
/// перенос соседнего виджета.
unsafe fn foreign_drag() -> bool {
    let mut process = 0u32;
    let thread = GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut process as *mut u32));
    if process == GetCurrentProcessId() {
        return false;
    }

    let mut info = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
    if GetGUIThreadInfo(thread, &mut info).is_err() {
        // Не узнали — скорее перетаскивание, чем нет: лишняя подложка на миг
        // безвредна, пропущенный бросок — нет
        return true;
    }
    !info.flags.contains(GUI_INMOVESIZE) && !info.flags.contains(GUI_INMENUMODE)
}

/// Один шаг опроса.
unsafe fn tick(catcher: &Catcher) {
    let down = held(VK_LBUTTON) || held(VK_RBUTTON);
    if !down {
        catcher.was_down.set(false);
        show(catcher, false);
        return;
    }

    let mut cursor = POINT::default();
    if GetCursorPos(&mut cursor).is_err() {
        return;
    }
    let over = over_converter(catcher, cursor);
    if !catcher.was_down.replace(true) {
        catcher.pressed_here.set(over);
    }

    let wanted = over && !catcher.pressed_here.get() && foreign_drag() && (catcher.accepts)();
    show(catcher, wanted);
}

/// Показывает подложку ровно поверх конвертера или прячет её.
unsafe fn show(catcher: &Catcher, visible: bool) {
    if catcher.shown.get() == visible {
        return;
    }

    if visible {
        let mut rect = RECT::default();
        if GetWindowRect(catcher.converter, &mut rect).is_err() {
            return;
        }
        // Владельца нет, и над конвертером подложку держит место в очереди:
        // над окном «поверх всех» — тоже поверх всех, иначе встала бы под ним
        let topmost = GetWindowLongPtrW(catcher.converter, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0;
        let placed = SetWindowPos(
            catcher.overlay,
            Some(if topmost { HWND_TOPMOST } else { HWND_TOP }),
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        if placed.is_err() {
            return;
        }
    } else {
        let _ = ShowWindow(catcher.overlay, SW_HIDE);
    }

    catcher.shown.set(visible);
    (catcher.on_hover)(visible);
}

/// Пути из брошенного блока.
unsafe fn paths(drop: HDROP) -> Vec<PathBuf> {
    let count = DragQueryFileW(drop, u32::MAX, None);
    let mut paths = Vec::new();
    for index in 0..count {
        let length = DragQueryFileW(drop, index, None) as usize;
        let mut buffer = vec![0u16; length + 1];
        let written = DragQueryFileW(drop, index, Some(&mut buffer)) as usize;
        if written > 0 {
            paths.push(PathBuf::from(std::ffi::OsString::from_wide(&buffer[..written])));
        }
    }
    paths
}

/// # Safety
///
/// Зовёт только Windows со своими же сообщениями: у `WM_DROPFILES` в `wparam`
/// лежит `HDROP`.
unsafe extern "system" fn procedure(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_TIMER if wparam.0 == TIMER => {
            if let Some(catcher) = catcher() {
                tick(&catcher);
            }
            LRESULT(0)
        }

        WM_DROPFILES => {
            let drop = HDROP(wparam.0 as *mut std::ffi::c_void);
            let dropped = paths(drop);
            DragFinish(drop);
            if let Some(catcher) = catcher() {
                show(&catcher, false);
                if !dropped.is_empty() {
                    (catcher.on_drop)(dropped);
                }
            }
            LRESULT(0)
        }

        WM_DESTROY => {
            let _ = KillTimer(Some(window), TIMER);
            LRESULT(0)
        }

        _ => DefWindowProcW(window, message, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        DestroyWindow, GetWindow, GW_OWNER, WS_EX_TRANSPARENT, WS_OVERLAPPEDWINDOW,
    };

    /// Настоящее окно: подложка ловит мышь и файлы, фокуса не берёт и до
    /// перетаскивания не видна.
    #[test]
    fn the_overlay_catches_files_but_stays_hidden_and_inert() {
        unsafe {
            let owner = CreateWindowExW(
                Default::default(),
                w!("STATIC"),
                w!("owner"),
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
            .expect("тестовое окно не создалось");
            let overlay = create(owner).expect("подложка не создалась");

            let style = GetWindowLongPtrW(overlay, GWL_EXSTYLE) as u32;
            for (flag, name) in [
                (WS_EX_ACCEPTFILES, "приём файлов"),
                (WS_EX_LAYERED, "слой"),
                (WS_EX_NOACTIVATE, "без фокуса"),
                (WS_EX_TOOLWINDOW, "без панели задач"),
            ] {
                assert_ne!(style & flag.0, 0, "нет: {name}");
            }
            // Прозрачное для мыши окно пропустило бы и перетаскивание
            assert_eq!(style & WS_EX_TRANSPARENT.0, 0);
            assert!(!IsWindowVisible(overlay).as_bool());
            // Владелец привёл бы OLE к приёмникам tao на окне конвертера
            assert!(GetWindow(overlay, GW_OWNER).ok().is_none_or(|owner| owner.is_invalid()));

            let _ = DestroyWindow(overlay);
            let _ = DestroyWindow(owner);
        }
    }
}
