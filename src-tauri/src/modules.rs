//! Модули: включённый виджет грузится, выключенный — нет.
//!
//! Модуль — это виджет из реестра целиком: его окно с webview и его фоновые
//! работы в Rust. Выключенный не прячется, а не существует вовсе: окна нет,
//! опрос не идёт, программ он не просит.
//!
//! Флаг — прежнее поле `enabled` в настройках виджета: имя оставлено ради
//! старого `settings.json`, смысл сменился с «показан» на «загружен».
//!
//! Состояние модулей в Rust (`.manage` в `lib.rs`) заводится всегда: оно
//! дёшево, а страница выключенного модуля в настройках продолжает работать —
//! город погоды, учётные данные почты, программы конвертера.

use std::collections::BTreeSet;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};

use crate::settings::{self, Settings, SettingsState};
use crate::widgets::{self, Widget};
use crate::windows;

/// Модули, которым нужен сайдкар датчиков. Опрос держит его, только пока
/// включён хоть один из них.
///
/// Сеть — ради скоростей приёма и отдачи: отклик у неё свой, от пинга.
/// Процессов тут нет: они читают систему сами, через `sysinfo`.
pub const SENSOR_WIDGETS: &[&str] = &["gpu", "cpu", "disks", "network", "debug", "kitchensink"];

/// Переходы модулей идут строго по одному.
///
/// ВАЖНО: **без очереди быстрое «выключить — включить» теряет окно.**
/// Разборка уходит главному потоку, а метка окна освобождается не сразу:
/// включение, пришедшее следом, видело ещё живое окно, решало «уже загружен» —
/// и разборка догоняла его, оставляя включённый модуль без окна.
static TRANSITION: Mutex<()> = Mutex::new(());

/// Фоновые работы, которые уже запущены. Каждая запускается один раз и живёт
/// до выхода: остановки у них нет, а выключенный модуль они не трогают сами —
/// окно уничтожено, и проверка видимости гасит опрос.
static STARTED: Mutex<BTreeSet<&'static str>> = Mutex::new(BTreeSet::new());

/// Сколько ждать главный поток, пока он доводит или разбирает окно.
const MAIN_THREAD_WAIT: Duration = Duration::from_secs(10);

/// Включён ли модуль.
pub fn on(app: &AppHandle, id: &str) -> bool {
    let Some(state) = app.try_state::<SettingsState>() else {
        return false;
    };
    widgets::find(id).is_some_and(|widget| state.get().widget(widget).enabled)
}

/// Нужен ли сайдкар датчиков: включён ли хоть один модуль железа.
pub fn sensors_needed(app: &AppHandle) -> bool {
    app.try_state::<SettingsState>()
        .is_some_and(|state| needs_sensors(&state.get()))
}

fn needs_sensors(settings: &Settings) -> bool {
    widgets::shown().any(|widget| SENSOR_WIDGETS.contains(&widget.id) && settings.widget(widget).enabled)
}

/// Запускает работы всех включённых модулей. Зовётся из `setup`, главным
/// потоком, после сборки окон: многим работам окно нужно уже готовым.
pub fn boot(app: &AppHandle) {
    let settings = app.state::<SettingsState>().get();

    for widget in widgets::shown().filter(|widget| settings.widget(widget).enabled) {
        start(app, widget.id);
        attach(app, widget.id);
    }
}

/// Работа запускается один раз за жизнь приложения.
fn once(work: &'static str, start: impl FnOnce()) {
    let fresh = STARTED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(work);
    if fresh {
        start();
    }
}

/// Фоновые работы модуля. Повторный вызов ничего не делает.
///
/// У кого работ нет — заметки, дела, пароли, коды 2FA, спидтест, — тем
/// хватает окна: всё остальное они просят командами сами.
fn start(app: &AppHandle, id: &str) {
    if SENSOR_WIDGETS.contains(&id) {
        once("sensors", || crate::poller::spawn(app.clone()));
    }

    match id {
        "network" => once("ping", || crate::ping::spawn(app.clone())),
        "sound" => once("sound", || crate::sound::spawn(app.clone())),
        // Буфер пишет, пока модуль включён, а видно окно или нет — неважно
        // (правило буфера обмена); выключенный он забывает сам
        "clipboard" => once("clipboard", || crate::clipboard::spawn(app.clone())),
        "processes" => once("processes", || crate::processes::spawn(app.clone())),
        "weather" => once("weather", || crate::weather::spawn(app.clone())),
        "mail" => once("mail", || crate::mail::spawn(app.clone())),
        // Медиасессии — это WinRT, и из потока команды к ним ходить нельзя
        "media" => once("media", || crate::media::spawn(app.clone())),
        "spotify" => once("spotify", || crate::spotify::spawn(app.clone())),
        "converter" => once("converter", || crate::converter::spawn(app.clone())),
        "downloader" => once("downloader", || crate::downloader::spawn(app.clone())),
        _ => {}
    }
}

/// Что модулю нужно от главного потока сразу после сборки окна.
fn attach(app: &AppHandle, id: &str) {
    if id == "converter" {
        crate::converter::accept_drops(app);
    }
}

/// И перед разборкой окна — тоже главным потоком.
fn detach(id: &str) {
    if id == "converter" {
        crate::converter::refuse_drops();
    }
}

/// Включает или выключает модуль: пишет настройку, собирает или разбирает окно.
///
/// ВАЖНО: **не из главного потока.** Сборка окна ждёт главный поток, и
/// вызванная из него она ждала бы сама себя — ровно так зависала синхронная
/// команда (см. [`windows::create`]). Из обработчиков меню и событий окна зовут
/// [`set_later`].
pub fn set(app: &AppHandle, widget: &'static Widget, on: bool) {
    let _turn = TRANSITION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let state = app.state::<SettingsState>();
    let mut own = state.get().widget(widget);
    own.enabled = on;
    let settings = settings::update(app, &state, |settings| settings.set_widget(widget.id, own));

    // Сперва вид, потом окно: галочка не должна ждать сборки webview
    crate::settings_window::emit(app);

    if on {
        load(app, widget, !settings.hidden_all);
    } else {
        unload(app, widget.id);
    }
}

/// Все модули разом — строка «Все» в списке виджетов. Одна запись настроек на
/// нажатие, а не по одной на каждый модуль.
pub fn set_all(app: &AppHandle, on: bool) {
    let _turn = TRANSITION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let state = app.state::<SettingsState>();
    // Дискретной видеокарты нет — её виджет «Включить все» пропускает:
    // показывать ему нечего, и в списке его галочка погашена (этап 57).
    // Признак помнится и тогда, когда опрос уснул вместе с модулями железа
    let no_gpu = on && crate::poller::lacks_graphics_card();
    let mut touched: Vec<&'static Widget> = Vec::new();
    let settings = settings::update(app, &state, |settings| {
        for widget in widgets::shown() {
            let mut own = settings.widget(widget);
            if own.enabled == on || (no_gpu && widget.id == "gpu") {
                continue;
            }
            own.enabled = on;
            settings.set_widget(widget.id, own);
            touched.push(widget);
        }
    });

    if touched.is_empty() {
        return;
    }
    crate::settings_window::emit(app);

    for widget in touched {
        if on {
            load(app, widget, !settings.hidden_all);
        } else {
            unload(app, widget.id);
        }
    }
}

/// То же, что [`set`], но из главного потока: работа уходит в пул и
/// возвращается сразу. Зовут меню виджета и крестик окна с рамкой.
pub fn set_later(app: &AppHandle, widget: &'static Widget, on: bool) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || set(&app, widget, on));
}

/// Грузит модуль: собирает окно, если его нет, и запускает работы.
fn load(app: &AppHandle, widget: &'static Widget, visible: bool) {
    if let Some(window) = app.get_webview_window(widget.id) {
        // Уже загружен — например, дважды нажатая галочка
        if visible {
            on_main_and_wait(app, move |_| {
                let _ = window.show();
            });
        }
        return;
    }

    match windows::create(app, widget) {
        Ok(window) => {
            let id = widget.id;
            on_main_and_wait(app, move |app| {
                windows::finish(&app, &window, widget, visible);
                attach(&app, id);
            });
        }
        Err(error) => {
            eprintln!("{error}");
            return;
        }
    }

    start(app, widget.id);
}

/// Выгружает модуль: окно запоминает место и разбирается вместе с webview.
///
/// Возвращается, когда метка окна освободилась: следующий переход застанет
/// честную картину.
fn unload(app: &AppHandle, id: &'static str) {
    on_main_and_wait(app, move |app| {
        let Some(window) = app.get_webview_window(id) else {
            return;
        };

        detach(id);
        // До разборки: разобранное окно координат уже не отдаёт
        windows::save_places(&app);
        let _ = window.destroy();
    });

    let started = Instant::now();
    while app.get_webview_window(id).is_some() {
        if started.elapsed() > MAIN_THREAD_WAIT {
            eprintln!("окно «{id}» не разобралось за {} с", MAIN_THREAD_WAIT.as_secs());
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Ставит работу главному потоку и ждёт, пока он её сделает.
///
/// Зовётся только из пула: из главного потока `run_on_main_thread` выполнил бы
/// работу на месте, а сборку окна в главном потоке сюда не носят вовсе.
fn on_main_and_wait<F>(app: &AppHandle, job: F)
where
    F: FnOnce(AppHandle) + Send + 'static,
{
    let (done, finished) = std::sync::mpsc::channel();
    windows::on_main(app, move |app| {
        job(app);
        let _ = done.send(());
    });

    if finished.recv_timeout(MAIN_THREAD_WAIT).is_err() {
        eprintln!("главный поток не ответил за {} с", MAIN_THREAD_WAIT.as_secs());
    }
}

/// Включение и выключение модуля — галочка «Вкл.» в окне настроек.
///
/// ВАЖНО: **async, а не `fn`.** Синхронная команда выполняется в главном
/// потоке, прямо в обработчике IPC, и собранное оттуда окно подвешивало приём
/// IPC во всём приложении. Работа уходит в пул блокирующих потоков; проба
/// 28 сентября — десять кругов «выгрузить — собрать» и дребезг без пауз —
/// прошла без единого зависания, новое окно отвечало за 150 мс.
#[tauri::command]
pub async fn set_module(app: AppHandle, id: String, on: bool) {
    let Some(widget) = widgets::find(&id) else {
        return;
    };

    let _ = tauri::async_runtime::spawn_blocking(move || set(&app, widget, on)).await;
}

/// Все модули разом — строка «Все». Тоже async, по той же причине.
#[tauri::command]
pub async fn set_all_modules(app: AppHandle, on: bool) {
    let _ = tauri::async_runtime::spawn_blocking(move || set_all(&app, on)).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::WidgetSettings;

    fn with_only(on: &[&str]) -> Settings {
        let mut settings = Settings::default();
        for widget in widgets::shown() {
            let own = WidgetSettings {
                enabled: on.contains(&widget.id),
                ..settings.widget(widget)
            };
            settings.set_widget(widget.id, own);
        }
        settings
    }

    /// Сайдкар нужен, пока включён хоть один модуль железа, и только тогда.
    #[test]
    fn sensors_follow_the_hardware_modules() {
        assert!(!needs_sensors(&with_only(&[])), "всё выключено — сайдкар не нужен");
        assert!(!needs_sensors(&with_only(&["notes", "processes", "weather"])), "процессы читают систему сами");
        for id in SENSOR_WIDGETS {
            assert!(needs_sensors(&with_only(&[id])), "{id} читает датчики");
        }
    }

    /// Каждый модуль из списка датчиков есть в реестре.
    #[test]
    fn sensor_modules_exist() {
        for id in SENSOR_WIDGETS {
            assert!(widgets::find(id).is_some(), "{id} нет в реестре");
        }
    }

    /// Виджет, чья карточка читает телеметрию, обязан стоять в списке датчиков:
    /// иначе с ним одним включённым опрос спал бы, а карточка вечно ждала бы
    /// чисел. Смотрим в исходники фронта, как `rules_paths_resolve` смотрит в
    /// правила.
    #[test]
    fn every_telemetry_reader_needs_the_sidecar() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/widgets");
        let registry = std::fs::read_to_string(root.join("registry.ts")).expect("registry.ts читается");

        for widget in widgets::shown() {
            // Папку виджета называет сам реестр фронта: «./<папка>/…Widget»
            let Some(line) = registry
                .lines()
                .skip_while(|line| !line.contains(&format!("id: \"{}\"", widget.id)))
                .find(|line| line.contains("import(\"./"))
            else {
                continue;
            };
            let folder = line.split("import(\"./").nth(1).and_then(|rest| rest.split('/').next());
            let Some(folder) = folder else {
                continue;
            };

            let reads = std::fs::read_dir(root.join(folder))
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
                .any(|text| text.contains("useTelemetry") || text.contains("lib/history"));

            if reads {
                assert!(
                    SENSOR_WIDGETS.contains(&widget.id),
                    "«{}» читает телеметрию, но в SENSOR_WIDGETS его нет",
                    widget.id
                );
            }
        }
    }
}
