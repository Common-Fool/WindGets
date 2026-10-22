//! Значок и меню в трее.
//!
//! Настройки живут в своём окне: виджетов десять, а половина настроек —
//! ползунки и поля, которых нативный `HMENU` не держит вовсе. Здесь остаётся
//! то, что дёргают не глядя: открыть настройки, убрать виджеты с экрана,
//! запереть их положение и выйти.
//!
//! Виджеты без рамки закрыть иначе нельзя, а при включённой прозрачности
//! для мыши до них вообще не дотянуться — поэтому выход отсюда никуда не
//! денется.

use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::settings::{self, SettingsState};
use crate::{settings_window, widgets, windows};

pub const TRAY_ID: &str = "main";

const OPEN: &str = "app:settings";
const HIDE_ALL: &str = "app:hide-all";
const LOCK: &str = "app:lock";
const QUIT: &str = "app:quit";

pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Windgets")
        .menu(&build(app)?)
        // Меню — только правой кнопкой. Левая поднимает виджеты над окнами,
        // двойная открывает настройки. Двойному нажатию предшествует
        // одиночное, и виджеты успевают подняться — это не мешает
        .show_menu_on_left_click(false)
        .on_menu_event(handle)
        .on_tray_icon_event(|tray, event| match event {
            // Сперва проверка потерянных окон: восстановление перезапускает
            // приложение, и тогда само нажатие уже не нужно
            TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
                if !windows::recover_unavailable(tray.app_handle()) =>
            {
                raise_all(tray.app_handle());
            }
            TrayIconEvent::DoubleClick { button: MouseButton::Left, .. }
                if !windows::recover_unavailable(tray.app_handle()) =>
            {
                settings_window::show(tray.app_handle());
            }
            _ => {}
        });

    // Значок один — белые плитки, тот же, что у окон в панели задач
    builder = builder.icon(tauri::image::Image::new(
        include_bytes!("../../src/assets/app-icons/mono-mosaic.rgba"), 32, 32,
    ));

    builder.build(app)?;
    Ok(())
}

/// Пересобирает меню целиком.
///
/// Дешевле и надёжнее, чем держать ссылки на галочки и обновлять их поштучно:
/// рассинхронизироваться тут просто нечему. Зовётся, когда рубильники дёрнули
/// не отсюда, а из окна настроек, и когда сменили язык.
pub fn refresh(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };

    match build(app) {
        Ok(menu) => {
            let _ = tray.set_menu(Some(menu));
        }
        Err(error) => eprintln!("меню трея не пересобралось: {error}"),
    }
}

fn build(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let settings = app.state::<SettingsState>().get();
    let menu = Menu::new(app)?;

    menu.append(&MenuItem::with_id(
        app,
        OPEN,
        tr!("tray.settings"),
        true,
        None::<&str>,
    )?)?;

    // Два рубильника: их дёргают чаще всего и не глядя, ради них лезть в окно
    // настроек было бы издевательством
    menu.append(&CheckMenuItem::with_id(
        app,
        HIDE_ALL,
        tr!("tray.hideAll"),
        true,
        settings.hidden_all,
        None::<&str>,
    )?)?;
    menu.append(&CheckMenuItem::with_id(
        app,
        LOCK,
        tr!("tray.lock"),
        true,
        settings.locked,
        None::<&str>,
    )?)?;

    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(app, QUIT, tr!("tray.quit"), true, None::<&str>)?)?;

    Ok(menu)
}

fn handle(app: &AppHandle, event: MenuEvent) {
    if matches!(event.id().as_ref(), OPEN | HIDE_ALL | LOCK) && windows::recover_unavailable(app) {
        return;
    }
    let state = app.state::<SettingsState>();

    match event.id().as_ref() {
        OPEN => settings_window::show(app),

        HIDE_ALL => {
            let next = !state.get().hidden_all;
            settings::update(app, &state, |settings| settings.hidden_all = next);

            if next {
                windows::hide_all(app);
            } else {
                windows::show_all(app);
            }

            refresh(app);
            settings_window::emit(app);
        }

        LOCK => {
            let next = !state.get().locked;
            settings::update(app, &state, |settings| settings.locked = next);

            settings_window::apply_flags(app);
            refresh(app);
            settings_window::emit(app);
        }

        QUIT => windows::quit(app),

        _ => {}
    }
}

/// Поднимает показанные виджеты над всеми окнами, не забирая фокус.
///
/// Через «поверх всех» туда и обратно: `HWND_TOP` без активации не
/// поднимает окно над чужим активным приложением. У кого «поверх всех»
/// включено своим флагом, тому его и оставляем.
fn raise_all(app: &AppHandle) {
    #[cfg(windows)]
    {
        use ::windows::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        };

        let settings = app.state::<SettingsState>().get();
        let flags = SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE;
        for widget in widgets::shown() {
            let Some(window) = app.get_webview_window(widget.id) else { continue };
            if !window.is_visible().unwrap_or(false) {
                continue;
            }
            let Ok(hwnd) = window.hwnd() else { continue };
            unsafe {
                let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, flags);
                if !settings.widget(widget).always_on_top {
                    let _ = SetWindowPos(hwnd, Some(HWND_NOTOPMOST), 0, 0, 0, 0, flags);
                }
            }
        }
    }
}
