//! Меню виджета по правой кнопке мыши.
//!
//! Меню нативное, а не карточка в самом окне, и причина простая: окно виджета
//! бывает высотой в сто тридцать точек, а меню из четырёх пунктов внутри такого
//! окна не поместится вовсе. Всплывающее меню рисует система, и границы окна
//! ему не указ.
//!
//! Собирается тем же `tauri::menu`, что и меню трея (`tray.rs`), но живёт
//! отдельно: у трея свой список слушателей, и события отсюда туда не попадают.

use serde::Serialize;
use tauri::menu::{CheckMenuItem, ContextMenu, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::{AppHandle, Emitter, LogicalPosition, Manager, WebviewWindow};

use crate::settings::SettingsState;
use crate::{settings_window, widgets, windows};

/// Приставка у пунктов меню виджета: по ней меню отличается от чужих.
const PREFIX: &str = "widget:";

const TOP: &str = "top";
const LOCK: &str = "lock";
/// За приставкой идёт номер обложки: `widget:spotify:cover:2`.
const COVER: &str = "cover";
/// За приставкой идёт номер размера: `widget:gpu:size:2`.
const SIZE: &str = "size";
/// То же для вида: `widget:speedtest:view:1`.
const VIEW: &str = "view";
const LAYOUT: &str = "layout";
const SETTINGS: &str = "settings";
/// Страница драйвера PawnIO на GitHub — только у процессора.
const PAWNIO: &str = "pawnio";
const CLOSE: &str = "close";

/// Приставка у пунктов контекстного меню отдельного дела.
const TASK_PREFIX: &str = "task:";
const TASK_EDIT: &str = "edit";
const TASK_REMOVE: &str = "remove";

/// Имя события, рассылаемого при выборе пункта в меню дела.
const TASK_MENU_EVENT: &str = "task-menu";

/// Полезная нагрузка события `task-menu`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskMenuPayload {
    pub action: &'static str,
    pub id: u64,
}

/// ВАЖНО: **виджет опознаётся по идентификатору пункта, а не по окну.**
/// Событие меню Tauri раздаёт всем глобальным слушателям и всем оконным разом,
/// не глядя на то, из какого окна меню всплыло (`app.rs`, разбор
/// `EventLoopMessage::MenuEvent`). Оконный слушатель поэтому получал бы и чужие
/// нажатия — и «закрыть виджет» закрывало бы все окна сразу.
fn item_id(widget: &str, action: &str) -> String {
    format!("{PREFIX}{widget}:{action}")
}

fn task_item_id(action: &str, id: u64) -> String {
    format!("{TASK_PREFIX}{action}:{id}")
}

fn build(app: &AppHandle, widget: &widgets::Widget) -> tauri::Result<Menu<tauri::Wry>> {
    let settings = app.state::<SettingsState>().get();
    let own = settings.widget(widget);
    let menu = Menu::new(app)?;

    // Виды идут первыми: они меняют, ЧТО показано, а размеры — сколько от
    // этого влезло. Есть они у немногих, и лишней строки у остальных не будет
    if widget.switches_views() && !(widget.has_reference_views() && settings.design == crate::settings::Design::App) {
        for (index, name) in widget.views.iter().enumerate() {
            menu.append(&CheckMenuItem::with_id(
                app,
                item_id(widget.id, &format!("{VIEW}:{index}")),
                crate::i18n::text(name),
                true,
                index == own.view,
                None::<&str>,
            )?)?;
        }

        menu.append(&PredefinedMenuItem::separator(app)?)?;
    }

    let layouts = widget.layout_choices(settings.design, own.view);
    if layouts.len() > 1 {
        for (index, name) in layouts.iter().enumerate() {
            menu.append(&CheckMenuItem::with_id(app, item_id(widget.id, &format!("{LAYOUT}:{index}")), crate::i18n::text(name), true, index == own.layout, None::<&str>)?)?;
        }
        menu.append(&PredefinedMenuItem::separator(app)?)?;
    }

    // Обложка идёт второй осью: она отвечает на «чем тут служит картинка», а
    // не «что показано» и не «сколько влезло». Есть она у одного пульта
    if widget.switches_covers(own.view) {
        for (index, name) in widget.covers.iter().enumerate() {
            menu.append(&CheckMenuItem::with_id(
                app,
                item_id(widget.id, &format!("{COVER}:{index}")),
                crate::i18n::text(name),
                true,
                index == own.cover,
                None::<&str>,
            )?)?;
        }

        menu.append(&PredefinedMenuItem::separator(app)?)?;
    }

    // Размеры стоят прямо в меню, а не в подменю: их два-четыре, а подменю
    // прячет за лишним движением мыши ровно то, ради чего меню и открывают.
    // Пунктов нет вовсе у тех, у кого размер один, — у погоды и у текстовых
    // виджетов, которые тянут мышью
    let sizes = widget.size_choices(settings.design, own.view, own.layout);
    if sizes.len() > 1 {
        for (index, preset) in &sizes {
            menu.append(&CheckMenuItem::with_id(
                app,
                item_id(widget.id, &format!("{SIZE}:{index}")),
                crate::i18n::text(widget.size_name(settings.design, preset)),
                true,
                *index == own.size,
                None::<&str>,
            )?)?;
        }

        menu.append(&PredefinedMenuItem::separator(app)?)?;
    }

    menu.append(&CheckMenuItem::with_id(
        app,
        item_id(widget.id, TOP),
        tr!("menu.onTop"),
        true,
        own.always_on_top,
        None::<&str>,
    )?)?;

    // Пункт погашен, пока поднят общий рубильник: свой флаг он не отменяет, но
    // и отпереть им запертое нельзя — нажатие тут не сделало бы ничего, а
    // выглядело бы как непрошедшее. Галочка при этом показывает свой флаг, а
    // не общий: ровно так же ведёт себя «Показ» под рубильником «спрятать все»
    menu.append(&CheckMenuItem::with_id(
        app,
        item_id(widget.id, LOCK),
        tr!("menu.lock"),
        !settings.locked,
        own.locked,
        None::<&str>,
    )?)?;

    menu.append(&PredefinedMenuItem::separator(app)?)?;

    // Процессору нужен драйвер PawnIO, и откуда он — видно в один щелчок: там
    // же его можно поставить и самому, мимо кнопки в карточке
    if widget.id == "cpu" {
        menu.append(&MenuItem::with_id(app, item_id(widget.id, PAWNIO), tr!("menu.pawnio"), true, None::<&str>)?)?;
    }

    menu.append(&MenuItem::with_id(
        app,
        item_id(widget.id, SETTINGS),
        tr!("menu.settings"),
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        item_id(widget.id, CLOSE),
        tr!("menu.close"),
        true,
        None::<&str>,
    )?)?;

    Ok(menu)
}

fn build_task_menu(app: &AppHandle, id: u64) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;

    menu.append(&MenuItem::with_id(
        app,
        task_item_id(TASK_EDIT, id),
        tr!("menu.task.edit"),
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        task_item_id(TASK_REMOVE, id),
        tr!("menu.task.remove"),
        true,
        None::<&str>,
    )?)?;

    Ok(menu)
}

/// Просьба фронта показать меню. Координаты — от левого верхнего угла окна.
///
/// ВАЖНО: **всплываем из главного потока, а не из потока команды.** Внутри
/// `popup_at` стоит `run_item_main_thread!`: он ставит работу главному потоку и
/// **ждёт ответа**. Из главного потока `send_user_message` выполняет её на
/// месте, и ожидание кончается сразу; из потока команды тот же вызов держал бы
/// поток команды всё время, пока меню открыто, — то есть пока человек его
/// читает.
///
/// Пока меню на экране, главный поток стоит в модальном цикле `TrackPopupMenu`,
/// а вместе с ним стоит и приём IPC у WebView2 — ровно как при перетаскивании
/// окна. Это нормально и кончается вместе с меню.
///
/// ВАЖНО: **перед меню окно обязано забрать и мышь, и передний план.**
///
/// Симптом один и тот же: меню всплывает, но не подсвечивает пункт под
/// курсором, и нажатие уходит мимо. Причин у него две, и лечить надо обе.
///
/// Первая — захват мыши. Правую кнопку принял webview, и WebView2 к этому
/// моменту держит `SetCapture`: все сообщения мыши идут ему, а не меню.
/// Модальный цикл `TrackPopupMenu` при этом крутится исправно и просто не
/// видит курсора. Снимается `ReleaseCapture`.
///
/// Вторая — передний план. `TrackPopupMenu` следит за мышью только у меню
/// активного окна, а виджет активным почти никогда не бывает: окна у него без
/// рамки и мимо панели задач. Просьба стать активным при чужом активном окне
/// отказывает молча — как её добиться, сказано у `take_the_mouse`.
///
/// Первая попытка чинила одну вторую причину — этого не хватило.
///
/// ВАЖНО: **команда асинхронная, и это главное лекарство** (11 октября, после
/// третьего возврата бага). Синхронная команда выполняется в главном потоке
/// прямо в обработчике запроса WebView2, `on_main` оттуда работает на месте,
/// и модальный цикл меню крутился внутри этого обработчика — пока WebView2
/// ждал ответа на тот самый вызов. Окно WebView2 лежит в нашем ребёнком из
/// другого процесса, и очередь ввода у них общая: занятый ожиданием поток
/// WebView2 задерживал движения мыши, и меню их не видело — «через раз»,
/// смотря чем WebView2 был занят в эту долю секунды. Из async-команды вызов
/// получает ответ сразу, а меню всплывает отдельным шагом главного потока,
/// уже вне обработчика.
#[tauri::command]
pub async fn open_widget_menu(window: WebviewWindow, x: f64, y: f64) {
    let app = window.app_handle().clone();
    let label = window.label().to_owned();

    windows::on_main(&app, move |app| {
        // Окно настроек виджетом не притворяется: меню ему не полагается
        let Some(widget) = widgets::find(&label) else {
            return;
        };
        let Some(window) = app.get_webview_window(&label) else {
            return;
        };

        take_the_mouse(&window);

        match build(&app, widget) {
            Ok(menu) => {
                let place = LogicalPosition::new(x, y);
                if let Err(error) = menu.popup_at(window.as_ref().window(), place) {
                    eprintln!("меню «{}» не открылось: {error}", widget.id);
                }
                wake_the_queue(&window);
            }
            Err(error) => eprintln!("меню «{}» не собралось: {error}", widget.id),
        }
    });
}

/// Показывает контекстное меню отдельного дела в списке задач.
///
/// Всплывает строго из главного потока по тем же правилам, что и меню виджета,
/// и окно так же делается активным: без этого `TrackPopupMenu` теряет мышь.
/// Асинхронная по той же причине, что `open_widget_menu`.
#[tauri::command]
pub async fn open_task_menu(app: AppHandle, window: WebviewWindow, x: f64, y: f64, id: u64) {
    let label = window.label().to_owned();

    windows::on_main(&app, move |app| {
        let Some(window) = app.get_webview_window(&label) else {
            return;
        };

        take_the_mouse(&window);

        match build_task_menu(&app, id) {
            Ok(menu) => {
                let place = LogicalPosition::new(x, y);
                if let Err(error) = menu.popup_at(window.as_ref().window(), place) {
                    eprintln!("меню дела {id} не открылось: {error}");
                }
                wake_the_queue(&window);
            }
            Err(error) => eprintln!("меню дела {id} не собралось: {error}"),
        }
    });
}

/// Нажали пункт меню. Ставится глобальным слушателем при сборке приложения.
///
/// Работу делает не этот модуль, а `settings_window::set_widget_option` — та же
/// функция, что и галочка в окне настроек. Своей ветки тут быть не должно:
/// настройка одна, и разойтись двум путям её правки — вопрос времени.
pub fn handle(app: &AppHandle, event: MenuEvent) {
    let event_id = event.id().as_ref();

    // Сперва проверяем пункты меню дела: у них своя приставка
    if let Some(rest) = event_id.strip_prefix(TASK_PREFIX) {
        let Some((action, id_str)) = rest.split_once(':') else {
            return;
        };
        let Ok(id) = id_str.parse::<u64>() else {
            return;
        };

        let action_str = match action {
            TASK_EDIT => TASK_EDIT,
            TASK_REMOVE => TASK_REMOVE,
            _ => return,
        };

        let _ = app.emit(
            TASK_MENU_EVENT,
            TaskMenuPayload {
                action: action_str,
                id,
            },
        );
        return;
    }

    let Some(rest) = event_id.strip_prefix(PREFIX) else {
        return;
    };
    // Именно с начала: имя виджета двоеточий не содержит, а действие содержит
    // — у размера за ним идёт ещё и номер
    let Some((id, action)) = rest.split_once(':') else {
        return;
    };
    let Some(widget) = widgets::find(id) else {
        return;
    };

    match action {
        TOP => {
            let now = app.state::<SettingsState>().get().widget(widget).always_on_top;
            settings_window::set_widget_option(
                app.clone(),
                widget.id.to_owned(),
                "top".to_owned(),
                !now,
            );
        }
        LOCK => {
            let now = app.state::<SettingsState>().get().widget(widget).locked;
            settings_window::set_widget_option(
                app.clone(),
                widget.id.to_owned(),
                "lock".to_owned(),
                !now,
            );
        }
        // Окно настроек одно, а этот пункт открывает его сразу на странице
        // своего виджета: общие разделы стоят там же, в колонке слева
        SETTINGS => settings_window::show_for(app, widget.id),
        PAWNIO => crate::links::open_link(crate::pawnio::PAGE.to_owned()),

        _ if action.starts_with(VIEW) => {
            if let Some(index) = number_in(action, VIEW) {
                settings_window::set_widget_view(app.clone(), widget.id.to_owned(), index);
            }
        }

        _ if action.starts_with(LAYOUT) => {
            if let Some(index) = number_in(action, LAYOUT) {
                settings_window::set_widget_layout(app.clone(), widget.id.to_owned(), index);
            }
        }

        _ if action.starts_with(COVER) => {
            if let Some(index) = number_in(action, COVER) {
                settings_window::set_widget_cover(app.clone(), widget.id.to_owned(), index);
            }
        }

        _ if action.starts_with(SIZE) => {
            // Номер приходит от нашего же меню, но разбор всё равно проверяем:
            // испорченный идентификатор не должен ставить размер номер ноль
            if let Some(index) = number_in(action, SIZE) {
                settings_window::set_widget_size(app.clone(), widget.id.to_owned(), index);
            }
        }

        // Выключает модуль: окно разбирается целиком. Обработчик меню живёт в
        // главном потоке, а разборка ждёт его же — поэтому через пул
        CLOSE => crate::modules::set_later(app, widget, false),
        _ => {}
    }
}

/// Отбирает мышь и передний план перед всплытием меню.
///
/// Не вышло — меню всё равно надо показать, поэтому отказ только пишется в
/// лог: по нему видно, что баг вернулся и почему.
///
/// ВАЖНО: **одного `SetForegroundWindow` мало, и отказывает он молча.**
/// Windows отдаёт передний план только процессу, получившему последний ввод,
/// а правый щелчок принимает не наш процесс, а процесс WebView2 — его окно
/// лежит в нашем ребёнком. Пока активна чужая программа (а виджет почти
/// всегда стоит неактивным рядом с ней), просьба отклоняется, и меню всплывает
/// у неактивного окна — то есть не подсвечивает пункт под курсором, и нажатие
/// уходит мимо. Отсюда «то есть, то нет»: баг зависел от того, кто был
/// активен до щелчка. Лечится известным приёмом: на мгновение подключиться к
/// вводу активного окна (`AttachThreadInput`), тогда его права переднего плана
/// становятся нашими.
#[cfg(windows)]
fn take_the_mouse(window: &WebviewWindow) {
    // Ведущее `::` обязательно: в этом модуле `windows` — это `crate::windows`,
    // наш собственный модуль окон, а не крейт
    use ::windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use ::windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
    use ::windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
    };

    // Захват держит webview, принявший правую кнопку: пока он не отпущен,
    // сообщения мыши до меню не доходят вовсе
    unsafe {
        let _ = ReleaseCapture();
    }

    if let Ok(handle) = window.hwnd() {
        // SAFETY: только вызовы user32 и kernel32 над живым окном этого потока;
        // подключение к чужому вводу снимается сразу же
        unsafe {
            let _ = SetForegroundWindow(handle);
            let foreground = GetForegroundWindow();
            if foreground != handle {
                let theirs = GetWindowThreadProcessId(foreground, None);
                let ours = GetCurrentThreadId();
                if theirs != 0 && theirs != ours && AttachThreadInput(ours, theirs, true).as_bool() {
                    let _ = SetForegroundWindow(handle);
                    let _ = AttachThreadInput(ours, theirs, false);
                }
                if GetForegroundWindow() != handle {
                    eprintln!("меню: окно «{}» не стало активным, пункты могут не подсвечиваться", window.label());
                }
            }
        }
    }

    // Через tao — там ещё и SetFocus вдобавок к переднему плану
    let _ = window.set_focus();
}

/// Будит очередь сообщений окна после закрытия меню.
///
/// Спутник `SetForegroundWindow` из документации `TrackPopupMenu`: без пустого
/// сообщения вдогонку окно остаётся с недоставленной порцией ввода, и
/// следующее меню всплывает уже не следя за мышью. Стоит **после** всплытия —
/// `popup_at` возвращается, когда меню уже закрыли.
#[cfg(windows)]
fn wake_the_queue(window: &WebviewWindow) {
    use ::windows::Win32::Foundation::{LPARAM, WPARAM};
    use ::windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_NULL};

    if let Ok(handle) = window.hwnd() {
        unsafe {
            let _ = PostMessageW(Some(handle), WM_NULL, WPARAM(0), LPARAM(0));
        }
    }
}

#[cfg(not(windows))]
fn wake_the_queue(_window: &WebviewWindow) {}

#[cfg(not(windows))]
fn take_the_mouse(window: &WebviewWindow) {
    let _ = window.set_focus();
}

/// Номер за приставкой действия. Испорченный идентификатор не должен
/// молча превращаться в нулевой номер.
fn number_in(action: &str, kind: &str) -> Option<usize> {
    action
        .strip_prefix(kind)
        .and_then(|tail| tail.strip_prefix(':'))
        .and_then(|number| number.parse().ok())
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Разбор идентификатора — единственное место, где меню может промахнуться
    /// мимо виджета. Живьём такое ловится только глазами и по одному пункту.
    #[test]
    fn an_item_id_names_its_widget_and_action() {
        let id = item_id("gpu", TOP);
        assert_eq!(id, "widget:gpu:top");

        let rest = id.strip_prefix(PREFIX).expect("приставка на месте");
        assert_eq!(rest.split_once(':'), Some(("gpu", "top")));
    }

    /// У размера за действием идёт ещё и номер, поэтому разбор идёт с начала:
    /// с конца «widget:gpu:size:2» дало бы виджет «gpu:size».
    #[test]
    fn a_size_item_keeps_its_number() {
        let id = item_id("gpu", &format!("{SIZE}:2"));
        assert_eq!(id, "widget:gpu:size:2");

        let rest = id.strip_prefix(PREFIX).expect("приставка на месте");
        let (widget, action) = rest.split_once(':').expect("имя и действие");
        assert_eq!(widget, "gpu");
        assert_eq!(action, "size:2");

        let number = action
            .strip_prefix(SIZE)
            .and_then(|tail| tail.strip_prefix(':'))
            .and_then(|number| number.parse::<usize>().ok());
        assert_eq!(number, Some(2));
    }

    /// Идентификатор пункта меню дела содержит действие и номер задачи.
    #[test]
    fn a_task_item_id_names_its_action_and_task_id() {
        assert_eq!(task_item_id(TASK_EDIT, 42), "task:edit:42");
        assert_eq!(task_item_id(TASK_REMOVE, 100), "task:remove:100");

        let rest = "task:edit:42".strip_prefix(TASK_PREFIX).expect("приставка дела");
        let (action, id_str) = rest.split_once(':').expect("действие и номер");
        assert_eq!(action, "edit");
        assert_eq!(id_str.parse::<u64>().ok(), Some(42));
    }

    /// Чужой пункт (например, из меню трея) обязан пройти мимо: приставки у
    /// него нет, и разбор до поиска виджета или дела не доходит.
    #[test]
    fn a_stranger_item_is_left_alone() {
        assert!("app:quit".strip_prefix(PREFIX).is_none());
        assert!("app:quit".strip_prefix(TASK_PREFIX).is_none());
    }
}
