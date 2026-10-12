//! Напоминание о донатах: изредка всплывающая карточка «Нравится HW
//! Dashboard?» с тремя кнопками — «Поддержать», «Позже», «Напоминать реже».
//!
//! Решения пользователя (7 октября 2026): маленькая карточка, а не окно
//! настроек; первый раз — через две недели после первого запуска, дальше не
//! чаще раза в месяц; «Напоминать реже» — раз в полгода. Полного выключения
//! нет — это тоже его слово.
//!
//! Карточка обязана не мешать: виджеты живут рядом с играми и работой.
//! Поэтому она не забирает фокус (`focused(false)` — tao показывает такое окно
//! через `SW_SHOWNOACTIVATE`), не всплывает, пока система говорит, что человек
//! занят (полноэкранная игра, презентация, «Не беспокоить»), и молчит, пока
//! поднят рубильник «Спрятать все виджеты».
//!
//! И главное условие — **есть куда звать.** Пока ни у одного способа в
//! `src/lib/support.json` нет ссылки, напоминание спит: карточка с погашенными
//! кнопками только раздражала бы.
//!
//! Окна у карточки нет до показа и не остаётся после ответа: собирается она
//! из своего потока — не из главного и не из синхронной команды, это
//! безопасно (`modules.md`), — доводится главным потоком и разбирается.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use tauri::{AppHandle, LogicalPosition, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::settings::{self, SettingsState, SupportReminder};
use crate::windows;

/// Метка окна карточки. Виджетом она не притворяется: в реестре её нет.
pub const LABEL: &str = "support-reminder";

const DAY: u64 = 24 * 60 * 60;

/// Первый показ — не раньше двух недель от первого запуска.
const FIRST: u64 = 14 * DAY;

/// Дальше — не чаще раза в месяц.
const EVERY: u64 = 30 * DAY;

/// «Напоминать реже» — не чаще раза в полгода.
const RARE: u64 = 180 * DAY;

/// Первая проверка — не при старте: при входе в систему человеку не до неё.
const SETTLE: Duration = Duration::from_secs(10 * 60);

/// Дальше — раз в час: сроки считаются днями, чаще смотреть незачем.
const TICK: Duration = Duration::from_secs(60 * 60);

/// Размер карточки при масштабе 100 %.
const WIDTH: f64 = 340.0;
const HEIGHT: f64 = 132.0;

/// Отступ от угла рабочей области.
const MARGIN: f64 = 16.0;

/// Пора ли показывать: чистое правило, без часов и окон.
///
/// Нулевой `first_run` — первый запуск ещё не отмечен; его отметит поток при
/// первой проверке, и отсчёт двух недель пойдёт оттуда.
pub fn due(now: u64, state: SupportReminder) -> bool {
    if state.first_run == 0 || now < state.first_run.saturating_add(FIRST) {
        return false;
    }
    match state.last_shown {
        None => true,
        Some(shown) => now >= shown.saturating_add(if state.rare { RARE } else { EVERY }),
    }
}

/// Способ поддержать — ровно то, что нужно отсюда из `support.json`.
#[derive(Deserialize)]
struct Way {
    value: String,
}

/// Список способов — тот же файл, что показывает раздел «Поддержать».
const WAYS: &str = include_str!("../../src/lib/support.json");

/// Есть ли куда звать: хоть у одного способа заполнена ссылка или адрес.
fn has_a_way() -> bool {
    serde_json::from_str::<Vec<Way>>(WAYS)
        .map(|ways| ways.iter().any(|way| !way.value.trim().is_empty()))
        .unwrap_or(false)
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|since| since.as_secs()).unwrap_or(0)
}

/// Свободен ли человек для уведомления — так, как это понимает сама Windows.
///
/// `SHQueryUserNotificationState`: в полноэкранной игре, в презентации и в
/// режиме «Не беспокоить» ответ не `QUNS_ACCEPTS_NOTIFICATIONS`. Не ответила
/// система — не мешаем: лучше пропустить показ, чем всплыть посреди игры.
#[cfg(windows)]
fn free() -> bool {
    // Крейт, а не наш модуль `crate::windows`, импортированный выше
    use ::windows::Win32::UI::Shell::{SHQueryUserNotificationState, QUNS_ACCEPTS_NOTIFICATIONS};
    // SAFETY: вызов только читает состояние оболочки
    unsafe { SHQueryUserNotificationState() }.is_ok_and(|state| state == QUNS_ACCEPTS_NOTIFICATIONS)
}

#[cfg(not(windows))]
fn free() -> bool {
    false
}

/// Отладочная проверка карточки вживую: `HW_SUPPORT_REMINDER_NOW=1` в
/// разработке показывает её через десять секунд, в обход сроков и пустых
/// ссылок. В собранном приложении переменная не читается вовсе.
fn forced() -> bool {
    cfg!(debug_assertions) && std::env::var_os("HW_SUPPORT_REMINDER_NOW").is_some_and(|value| value == "1")
}

/// Запускает поток напоминания. Живёт до выхода и почти всё время спит.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    let spawned = std::thread::Builder::new().name("support-reminder".into()).spawn(move || {
        if forced() {
            std::thread::sleep(Duration::from_secs(10));
            show(&app);
            return;
        }

        std::thread::sleep(SETTLE);
        loop {
            check(&app);
            std::thread::sleep(TICK);
        }
    });
    if let Err(error) = spawned {
        eprintln!("поток напоминания не запустился: {error}");
    }
}

/// Одна проверка: отметить первый запуск, сверить срок и условия, показать.
fn check(app: &AppHandle) {
    let state = app.state::<SettingsState>();
    let now = now();

    if state.get().support_reminder.first_run == 0 {
        settings::update(app, &state, |settings| settings.support_reminder.first_run = now);
        return;
    }

    let current = state.get();
    if !due(now, current.support_reminder) || current.hidden_all || !has_a_way() || !free() {
        return;
    }
    if app.get_webview_window(LABEL).is_some() {
        return;
    }

    // Срок пишем до показа: упади приложение на карточке — она не станет
    // всплывать на каждом запуске
    settings::update(app, &state, |settings| settings.support_reminder.last_shown = Some(now));
    show(app);
}

/// Собирает карточку спрятанной в правом нижнем углу основного монитора и
/// показывает её главным потоком, без фокуса.
fn show(app: &AppHandle) {
    let scale = app.state::<SettingsState>().get().scale;
    let (width, height) = (WIDTH * scale, HEIGHT * scale);

    // Угол рабочей области — в логических точках основного монитора
    let Ok(Some(monitor)) = app.primary_monitor() else {
        return;
    };
    let factor = monitor.scale_factor();
    let area = monitor.work_area();
    let right = f64::from(area.position.x) / factor + f64::from(area.size.width) / factor;
    let bottom = f64::from(area.position.y) / factor + f64::from(area.size.height) / factor;
    let place = LogicalPosition::new(right - width - MARGIN, bottom - height - MARGIN);

    let url = format!("index.html?widget={LABEL}");
    let built = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App(url.into()))
        .title(tr!("window.supportReminder"))
        .additional_browser_args(crate::windows::BROWSER_ARGS)
        .inner_size(width, height)
        .position(place.x, place.y)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .skip_taskbar(true)
        .resizable(false)
        .always_on_top(true)
        .focused(false)
        // Спрятанной — до защиты рамки: иначе tao успел бы показать заголовок
        .visible(false)
        .build();

    if let Err(error) = built {
        eprintln!("карточка напоминания не собралась: {error}");
        return;
    }
    windows::on_main(app, move |app| {
        let Some(window) = app.get_webview_window(LABEL) else {
            return;
        };
        windows::keep_frameless(&window);
        if let Err(error) = crate::window_effects::apply_to(&window) {
            eprintln!("подложка у карточки напоминания не снялась: {error}");
        }
        // Первый показ окна, собранного с `focused(false)`, — без активации
        let _ = window.show();
    });
}

/// Ответ на карточку.
#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Answer {
    /// Открыть «Поддержать» в окне настроек.
    Support,
    /// Просто закрыть: следующий раз — через обычный срок.
    Later,
    /// Дальше — не чаще раза в полгода.
    Rare,
}

/// Кнопка карточки. Карточка после ответа разбирается.
#[tauri::command]
pub fn support_reminder_answer(app: AppHandle, answer: Answer) {
    if answer == Answer::Rare {
        let state = app.state::<SettingsState>();
        settings::update(&app, &state, |settings| settings.support_reminder.rare = true);
        crate::settings_window::emit(&app);
    }

    windows::on_main(&app, move |app| {
        if answer == Answer::Support {
            crate::settings_window::show_support(&app);
        }
        if let Some(window) = app.get_webview_window(LABEL) {
            let _ = window.destroy();
        }
    });
}

/// Галочка «Напоминать реже» в разделе «Поддержать» — то же поле, что у
/// кнопки карточки: решение с карточки так можно и вернуть.
#[tauri::command]
pub fn set_support_reminder_rare(app: AppHandle, rare: bool) {
    let state = app.state::<SettingsState>();
    if state.get().support_reminder.rare == rare {
        return;
    }
    settings::update(&app, &state, |settings| settings.support_reminder.rare = rare);
    crate::settings_window::emit(&app);
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: u64 = 1_700_000_000;

    fn state(last_shown: Option<u64>, rare: bool) -> SupportReminder {
        SupportReminder { first_run: START, last_shown, rare }
    }

    #[test]
    fn the_first_reminder_waits_two_weeks() {
        assert!(!due(START + FIRST - 1, state(None, false)));
        assert!(due(START + FIRST, state(None, false)));
        // Не отмеченный первый запуск не показывает ничего
        assert!(!due(START + 1000 * DAY, SupportReminder::default()));
    }

    #[test]
    fn then_once_a_month() {
        let shown = START + FIRST;
        assert!(!due(shown + EVERY - 1, state(Some(shown), false)));
        assert!(due(shown + EVERY, state(Some(shown), false)));
    }

    #[test]
    fn rarer_means_twice_a_year_and_never_never() {
        let shown = START + FIRST;
        assert!(!due(shown + EVERY, state(Some(shown), true)));
        assert!(!due(shown + RARE - 1, state(Some(shown), true)));
        assert!(due(shown + RARE, state(Some(shown), true)), "реже — не значит никогда");
    }

    /// Ссылки есть (12 октября: Monobank, Ko-fi, криптовалюта) — напоминание
    /// живое. Опустеет список — тест упадёт, и это повод проверить, что
    /// карточка снова уснула, а не зовёт в никуда.
    #[test]
    fn ways_are_read_from_the_shared_list() {
        let ways: Vec<serde_json::Value> = serde_json::from_str(WAYS).expect("support.json читается");
        assert!(!ways.is_empty());
        let mut ids: Vec<&str> = ways.iter().map(|way| way["id"].as_str().expect("id есть")).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), ways.len(), "id способов не повторяются");
        assert!(has_a_way(), "ссылок не осталось — напоминание уснёт, проверить и обновить тест");
    }

    /// Ключи каталога в `support.json` типом TypeScript больше не проверить —
    /// проверяем здесь, по русскому каталогу.
    #[test]
    fn support_ways_name_real_catalog_keys() {
        let catalog: serde_json::Value =
            serde_json::from_str(include_str!("../../src/i18n/ru.json")).expect("каталог читается");
        let ways: Vec<serde_json::Value> = serde_json::from_str(WAYS).expect("support.json читается");
        for way in &ways {
            for field in ["about", "nameKey"] {
                if let Some(key) = way[field].as_str() {
                    assert!(catalog.get(key).is_some(), "ключа «{key}» нет в каталоге");
                }
            }
        }
    }

    const FRONTEND_REGISTRY: &str = include_str!("../../src/widgets/registry.ts");

    /// Окна нет в реестре виджетов, но компонент ему нужен.
    #[test]
    fn the_card_has_a_component_on_the_front() {
        assert!(FRONTEND_REGISTRY.contains(&format!("id: \"{LABEL}\"")));
    }

    /// И виджетом оно не притворяется: иначе попало бы в список и открывалось
    /// бы при старте.
    #[test]
    fn the_card_is_not_a_widget() {
        assert!(crate::widgets::find(LABEL).is_none());
    }
}
