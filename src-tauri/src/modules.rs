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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
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

/// Переходы модулей идут строго по одному и в том порядке, в каком их просили.
///
/// ВАЖНО: **без очереди быстрое «выключить — включить» теряет окно.**
/// Разборка уходит главному потоку, а метка окна освобождается не сразу:
/// включение, пришедшее следом, видело ещё живое окно, решало «уже загружен» —
/// и разборка догоняла его, оставляя включённый модуль без окна.
///
/// ВАЖНО: **очередь по номерам, а не голый `Mutex`.** Обычный замок порядка
/// ожидающих не обещает: «Включить все» и следом «выключить конвертер», оба
/// пришедшие во время чужого перехода, могли пройти задом наперёд — и
/// конвертер включился бы вопреки последнему нажатию. Номер берётся в момент
/// просьбы, до ухода в пул ([`ticket`]).
struct Turns {
    issued: u64,
    serving: u64,
}

static TURNS: Mutex<Turns> = Mutex::new(Turns { issued: 0, serving: 0 });
static TURN: Condvar = Condvar::new();

/// Номер в очереди переходов. Брать его сразу, в потоке просьбы.
pub fn ticket() -> u64 {
    let mut turns = TURNS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let number = turns.issued;
    turns.issued += 1;
    number
}

/// Подошедшая очередь; уходя, пропускает следующего — и при панике тоже.
struct Turn;

impl Drop for Turn {
    fn drop(&mut self) {
        TURNS.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).serving += 1;
        TURN.notify_all();
    }
}

fn wait_turn(number: u64) -> Turn {
    let mut turns = TURNS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    while turns.serving != number {
        turns = TURN.wait(turns).unwrap_or_else(|poisoned| poisoned.into_inner());
    }
    Turn
}

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
/// [`set_later`]. `turn` — номер из [`ticket`], взятый в момент просьбы.
pub fn set(app: &AppHandle, turn: u64, widget: &'static Widget, on: bool) -> Result<(), String> {
    let _turn = wait_turn(turn);

    // Галочку видеокарты без дискретной карты гасит окно настроек; эта
    // проверка — на случай, если просьба придёт мимо него. Выключать можно
    // всегда: после обновления драйвера карта бывает не видна до перезапуска
    if on && widget.id == "gpu" && crate::poller::lacks_graphics_card() {
        return Err(tr!("settings.widgets.noGpu"));
    }

    let state = app.state::<SettingsState>();
    settings::update(app, &state, |settings| {
        settings.edit_widget(widget, |own| {
            own.enabled = on;
            true
        });
    });

    // Сперва вид, потом окно: галочка не должна ждать сборки webview
    crate::settings_window::emit(app);

    if on {
        enable(app, widget)
    } else {
        unload(app, widget.id);
        Ok(())
    }
}

/// То же, что [`set`], но из главного потока: работа уходит в пул и
/// возвращается сразу. Зовут меню виджета и крестик окна с рамкой.
pub fn set_later(app: &AppHandle, widget: &'static Widget, on: bool) {
    let app = app.clone();
    let turn = ticket();
    tauri::async_runtime::spawn_blocking(move || {
        let _ = set(&app, turn, widget, on);
    });
}

/// Неудавшееся включение откатывается: `enabled` назад в `false`.
fn enable(app: &AppHandle, widget: &'static Widget) -> Result<(), String> {
    if let Err(error) = load(app, widget) {
        eprintln!("модуль «{}» не включился: {error}", widget.id);
        settings::update_widget(app, &app.state::<SettingsState>(), widget, |_, own| {
            own.enabled = false;
            true
        });
        crate::settings_window::emit(app);
        // После таймаута окно могло уже собраться, но не пройти доводку.
        // Разбираем его до следующего перехода, как при обычном выключении.
        unload(app, widget.id);
        return Err(tr!("settings.widgets.enableFailed", title = widget.title()));
    }
    Ok(())
}

/// Грузит модуль: собирает окно, если его нет, и запускает работы.
fn load(app: &AppHandle, widget: &'static Widget) -> Result<(), String> {
    let existing = app.get_webview_window(widget.id);
    let fresh = existing.is_none();
    let window = match existing {
        Some(window) => window,
        None => windows::create(app, widget)?,
    };
    // После таймаута доводка могла остаться в очереди главного потока.
    // Запоздавшая работа не должна показать выключенный модуль.
    let cancelled = Arc::new(AtomicBool::new(false));
    let pending = cancelled.clone();
    let result = on_main_and_wait(app, move |app| {
        if pending.load(Ordering::SeqCst) || app.get_webview_window(widget.id).is_none() {
            return Err(format!("доводка окна «{}» отменена", widget.id));
        }
        // Рубильник «Спрятать все» — нынешний, а не снятый до сборки: его
        // могли и поднять, и опустить, пока собирался webview, а `show_all`
        // окна, ещё не зарегистрированного, не видит
        let visible = !app.state::<SettingsState>().get().hidden_all;
        if fresh {
            windows::finish(&app, &window, widget, false);
        }
        if pending.load(Ordering::SeqCst) {
            return Err(format!("доводка окна «{}» отменена", widget.id));
        }
        if visible {
            window.show().map_err(|error| format!("окно «{}» не показалось: {error}", widget.id))?;
        }
        if fresh {
            attach(&app, widget.id);
        }
        Ok(())
    });
    if result.is_err() {
        cancelled.store(true, Ordering::SeqCst);
    }
    result?;

    start(app, widget.id);
    Ok(())
}

/// Выгружает модуль: окно запоминает место и разбирается вместе с webview.
///
/// Возвращается, когда метка окна освободилась: следующий переход застанет
/// честную картину.
fn unload(app: &AppHandle, id: &'static str) {
    // Тот же путь, что у кнопки «Отмена»: рабочий поток гасит Job Object
    // и убирает недописанное; загрузчик также прерывает обновление yt-dlp.
    match id {
        // Кадр над ручкой — своё окно GDI поверх всех, а прослушивание —
        // PlaySound: гасит их уборка карточки, а у разобранного окна её нет
        "converter" => {
            crate::converter::converter_cancel(app.clone());
            crate::converter::converter_hush(app.clone());
            crate::converter::converter_frame_hide(app.clone());
        }
        "downloader" => crate::downloader::downloader_cancel(app.clone()),
        // Замер идёт сам по себе, восемью соединениями: окно ушло, а он бы
        // лил и отдавал дальше и дописал бы историю выключенному модулю
        "speedtest" => crate::speedtest::cancel(app),
        "clipboard" => crate::clipboard::forget(),
        _ => {}
    }

    if let Err(error) = on_main_and_wait(app, move |app| {
        let Some(window) = app.get_webview_window(id) else {
            return Ok(());
        };

        detach(id);
        // До разборки: разобранное окно координат уже не отдаёт. Поднятое под
        // раскрытую панель записалось бы поднятым
        windows::return_lift(&window);
        windows::save_places(&app);
        window.destroy().map_err(|error| format!("окно «{id}» не разобралось: {error}"))
    }) {
        eprintln!("выгрузка модуля «{id}»: {error}");
    }

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
fn on_main_and_wait<F>(app: &AppHandle, job: F) -> Result<(), String>
where
    F: FnOnce(AppHandle) -> Result<(), String> + Send + 'static,
{
    let (done, finished) = std::sync::mpsc::channel();
    windows::on_main(app, move |app| {
        let _ = done.send(job(app));
    });

    wait_for_main(finished, MAIN_THREAD_WAIT)
}

fn wait_for_main(finished: std::sync::mpsc::Receiver<Result<(), String>>, limit: Duration) -> Result<(), String> {
    match finished.recv_timeout(limit) {
        Ok(result) => result,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            Err(format!("главный поток не ответил за {} с", limit.as_secs()))
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err("главный поток не принял работу".to_owned()),
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
pub async fn set_module(app: AppHandle, id: String, on: bool) -> Result<(), String> {
    let widget = widgets::find(&id).ok_or_else(|| tr!("common.failed"))?;
    let turn = ticket();
    tauri::async_runtime::spawn_blocking(move || set(&app, turn, widget, on)).await.map_err(|error| {
        eprintln!("переключение модуля не завершилось: {error}");
        tr!("common.failed")
    })?
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

    /// Успех подтверждается ответом главного потока, отказ операции не теряется.
    #[test]
    fn main_thread_result_reaches_the_caller() {
        for outcome in [Ok(()), Err("окно не показалось".to_owned())] {
            let (done, finished) = std::sync::mpsc::channel();
            done.send(outcome.clone()).unwrap();
            assert_eq!(wait_for_main(finished, Duration::ZERO), outcome);
        }
    }

    /// Отложенная доводка не считается удачным включением по истечении срока.
    #[test]
    fn main_thread_timeout_is_a_failure() {
        let (done, finished) = std::sync::mpsc::channel();
        let error = wait_for_main(finished, Duration::ZERO).unwrap_err();
        assert!(error.contains("не ответил"));
        assert!(done.send(Ok(())).is_err(), "запоздавший ответ уже не подтверждает включение");
    }

    /// Отказ постановки работы в очередь не превращается в успешное включение.
    #[test]
    fn a_dropped_main_thread_job_is_a_failure() {
        let (done, finished) = std::sync::mpsc::channel();
        drop(done);
        assert!(wait_for_main(finished, MAIN_THREAD_WAIT).unwrap_err().contains("не принял"));
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
