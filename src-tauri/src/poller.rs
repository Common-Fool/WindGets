use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::lhm::roles::{self, Choices, Roles, Seen};
use crate::lhm::Reading;
use crate::sensors::{self, Sensors};

/// Сколько осечек подряд терпим, прежде чем объявить источник недоступным.
/// Одиночная осечка — обычное дело, пугать ею пользователя незачем.
const FAILURES_BEFORE_ALARM: u32 = 3;

/// Как часто спящий опрос смотрит, не включили ли модуль железа. Короче шага
/// опроса: включённая карточка не должна ждать первых чисел пять секунд.
const IDLE_CHECK: Duration = Duration::from_millis(500);

/// Пауза перед новой попыткой поднять сайдкар. Держать её короче шага опроса
/// незачем: если он не запускается, чаще пробовать смысла нет.
const RESTART_AFTER: Duration = Duration::from_secs(3);

/// Событие, которым снимок расходится по окнам виджетов.
const TELEMETRY_EVENT: &str = "telemetry";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// Момент снятия, unix-время в миллисекундах.
    pub at: u64,
    pub readings: Vec<Reading>,
    /// Какой датчик что значит на этом железе (`lhm/roles.rs`). Виджеты
    /// процессора и видеокарты ищут показания по этим путям, а не хвостами.
    pub roles: Roles,
    /// Датчики платы для ручного выбора в настройках процессора (этап 58).
    pub choices: Choices,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum SourceState {
    /// Ещё ни одного удачного ответа, но и тревожить рано.
    Connecting,
    /// Датчики никому не нужны: все модули железа выключены, и сайдкар не
    /// запущен вовсе.
    Idle,
    /// Снимок в куче: с ролями он в разы крупнее остальных вариантов, а
    /// состояние копируется на каждой рассылке. На проводе `Box` не виден.
    Ok { snapshot: Box<Snapshot> },
    Unavailable { reason: String },
}

/// Последнее состояние источника. Нужно, чтобы окно, открытое между тиками,
/// получило данные сразу, а не ждало следующей секунды.
pub struct Telemetry(Mutex<SourceState>);

impl Default for Telemetry {
    fn default() -> Self {
        Telemetry(Mutex::new(SourceState::Connecting))
    }
}

#[tauri::command]
pub fn telemetry_state(app: AppHandle, state: State<'_, Telemetry>) -> SourceState {
    // Без модулей железа опрос не запускался вовсе, и в состоянии так и стоит
    // «подключаемся» — раздел «Данные» ждал бы вечно
    if !crate::modules::sensors_needed(&app) {
        return SourceState::Idle;
    }

    // Отравленный мьютекс не повод падать: данные внутри остаются осмысленными
    state
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Что последний снимок сказал о видеокарте: 0 — снимков не было, 1 — она
/// есть, 2 — дискретной нет.
///
/// Отдельно от состояния источника, а не из него: выключи человек все модули
/// железа, опрос засыпает и публикует `Idle`, а с ним пропал бы и снимок. Тогда
/// «Включить все» на машине без видеокарты включило бы и её (нашло ревью
/// второй модели). Роли за запуск не теряются (`Seen`), так что и признак не
/// перескочит обратно.
static GRAPHICS: AtomicU8 = AtomicU8::new(0);

/// Снимки говорят, что дискретной видеокарты нет. Снимков ещё не было —
/// `false`: судить не по чему, и пропускать видеокарту не с чего.
pub fn lacks_graphics_card() -> bool {
    GRAPHICS.load(Ordering::Relaxed) == 2
}

/// Запрос удержания и владение процессом меняются одним атомарным словом:
/// между подтверждением «процесса нет» и запуском не пролезет новый сайдкар.
struct SidecarControl(AtomicU8);

impl SidecarControl {
    const HOLD: u8 = 1;
    const LIVE: u8 = 2;

    const fn new() -> Self {
        Self(AtomicU8::new(0))
    }

    fn hold(&self, on: bool) {
        if on {
            self.0.fetch_or(Self::HOLD, Ordering::SeqCst);
        } else {
            self.0.fetch_and(!Self::HOLD, Ordering::SeqCst);
        }
    }

    fn holding(&self) -> bool {
        self.0.load(Ordering::SeqCst) & Self::HOLD != 0
    }

    fn held(&self) -> bool {
        self.0.load(Ordering::SeqCst) == Self::HOLD
    }

    fn starting(&self) -> bool {
        self.0.compare_exchange(0, Self::LIVE, Ordering::SeqCst, Ordering::SeqCst).is_ok()
    }

    fn stopped(&self) {
        self.0.fetch_and(!Self::LIVE, Ordering::SeqCst);
    }
}

static CONTROL: SidecarControl = SidecarControl::new();

/// Сайдкар перезапустить на следующем такте: LHM открывает драйвер один раз,
/// при старте, и поставленный на ходу PawnIO сам не подхватит.
static RESTART: AtomicBool = AtomicBool::new(false);

pub fn hold(on: bool) {
    CONTROL.hold(on);
}

/// Запрошена пауза: её слышит и ожидание первого ответа сайдкара.
pub fn holding() -> bool {
    CONTROL.holding()
}

/// Процесса нет либо его выход уже подтверждён. Отсутствие самого потока
/// опроса тоже безопасно: новый процесс не займёт место, пока поднят HOLD.
pub fn held() -> bool {
    CONTROL.held()
}

/// Неподтверждённый выход не теряет Child: следующий заход повторит остановку.
fn stop_sensors(sensors: &mut Option<Sensors>) -> bool {
    if sensors.as_mut().is_some_and(|sensors| !sensors.stop()) {
        return false;
    }
    *sensors = None;
    CONTROL.stopped();
    true
}

/// Сон между тактами — кусками, чтобы `hold` и перезапуск не ждали шага
/// целиком.
fn nap(pause: Duration) {
    let until = std::time::Instant::now() + pause;
    while std::time::Instant::now() < until {
        if holding() || RESTART.load(Ordering::SeqCst) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100).min(until.saturating_duration_since(std::time::Instant::now())));
    }
}

pub fn restart() {
    RESTART.store(true, Ordering::SeqCst);
}

/// Ручной выбор датчиков платы из настроек (этап 58). Читается на каждом
/// такте, как и шаг: выбранное в окне настроек доходит до карточки за секунду.
fn picks(app: &AppHandle) -> std::collections::BTreeMap<String, String> {
    app.try_state::<crate::settings::SettingsState>()
        .map(|state| state.get().sensor_picks)
        .unwrap_or_default()
}

/// Шаг опроса из настроек.
///
/// Спрашиваем на каждом заходе, а не один раз при старте: он правится в окне
/// настроек, а перезапускать опрос ради этого незачем.
fn step(app: &AppHandle) -> Duration {
    let Some(state) = app.try_state::<crate::settings::SettingsState>() else {
        return Duration::from_secs(crate::settings::DEFAULT_POLL_SECONDS);
    };

    Duration::from_secs(state.get().poll_seconds)
}

/// Запускает опрос железа. Живёт до конца работы приложения, но сайдкар
/// держит, только пока включён хоть один модуль железа (`modules.rs`).
///
/// Выключили последний — процесс гасится, а поток спит и раз в полсекунды
/// смотрит, не включили ли снова. LHM с 233 датчиками раз в секунду — не та
/// работа, которую стоит делать в пустоту.
///
/// Свой поток ОС, а не задача рантайма: разговор с сайдкаром блокирующий, и
/// держать на нём поток пула незачем. Заодно ушла и вся асинхронность —
/// HTTP-клиента тут больше нет.
pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let exe = match sensors::locate(app.path().resource_dir().ok()) {
            Some(path) => path,
            None => {
                publish(
                    &app,
                    SourceState::Unavailable {
                        reason: tr!("source.sidecarMissing"),
                    },
                );
                return;
            }
        };

        let mut sensors: Option<Sensors> = None;
        let mut failures: u32 = 0;
        let mut stopping = false;
        let mut driver_update = false;
        let mut idle = false;
        // Роли, увиденные за запуск: живут дольше сайдкара, чтобы его
        // перезапуск после сбоя не убрал из карточек ни одного ряда
        let mut seen = Seen::default();

        loop {
            if stopping {
                if !stop_sensors(&mut sensors) {
                    std::thread::sleep(IDLE_CHECK);
                    continue;
                }
                stopping = false;
            }
            if !crate::modules::sensors_needed(&app) {
                stopping = !stop_sensors(&mut sensors);
                failures = 0;
                if !idle {
                    idle = true;
                    publish(&app, SourceState::Idle);
                }
                std::thread::sleep(IDLE_CHECK);
                continue;
            }
            if idle {
                idle = false;
                publish(&app, SourceState::Connecting);
            }

            // Драйвер ставят или удаляют: сайдкар гасится и ждёт. Карточки
            // честно говорят «подключаемся», а не держат последний снимок
            if holding() {
                let had = sensors.is_some();
                stopping = !stop_sensors(&mut sensors);
                if had {
                    publish(&app, SourceState::Connecting);
                }
                failures = 0;
                std::thread::sleep(IDLE_CHECK);
                continue;
            }
            // Поставили драйвер мимо нас (сам LHM, FanControl) — заметить и
            // перезапустить сайдкар. Сам себе ставит срок, тут дёшево
            driver_update |= crate::pawnio::watch(&app);
            // ВАЖНО: **сперва «подключаемся», потом «драйвер готов».** Готовый
            // драйвер снимает замок с карточки процессора, а прежний сайдкар
            // читал без него — нулями. Скажи окнам о драйвере раньше, они бы
            // показали эти нули исправными показаниями до нового снимка
            if RESTART.swap(false, Ordering::SeqCst) && sensors.is_some() {
                stopping = !stop_sensors(&mut sensors);
                publish(&app, SourceState::Connecting);
                if stopping {
                    continue;
                }
            }
            if driver_update {
                crate::pawnio::publish(&app);
                driver_update = false;
            }

            // Шаг отсчитываем от конца опроса, а не по расписанию: пропущенные
            // тики догонять пачкой всё равно было нечем
            let pause = step(&app);

            if sensors.is_none() {
                if !CONTROL.starting() {
                    continue;
                }
                match Sensors::start(&exe) {
                    Ok(started) => sensors = Some(started),
                    Err(reason) => {
                        CONTROL.stopped();
                        failures = failures.saturating_add(1);
                        if failures >= FAILURES_BEFORE_ALARM {
                            publish(&app, SourceState::Unavailable { reason });
                        }

                        nap(RESTART_AFTER);
                        continue;
                    }
                }
            }

            let outcome = sensors
                .as_mut()
                .map_or_else(|| Err(tr!("source.sidecarDown")), Sensors::poll);

            // Ответ, прерванный удержанием, не осечка источника. Сам процесс
            // гасим до подтверждения в начале следующего захода.
            if holding() {
                continue;
            }

            match outcome {
                Ok(flat) => {
                    failures = 0;
                    // Выбор кладётся поверх памяти за запуск, а не в неё:
                    // снятый выбор возвращает найденное автоматикой
                    let roles = roles::apply_picks(seen.remember(roles::resolve(&flat)), &flat, &picks(&app));
                    let choices = roles::choices(&flat);
                    publish(
                        &app,
                        SourceState::Ok {
                            snapshot: Box::new(Snapshot {
                                at: now_ms(),
                                readings: flat.readings,
                                roles,
                                choices,
                            }),
                        },
                    );
                }
                Err(reason) => {
                    // Сорванный разговор лечится только новым процессом:
                    // на середине ответа поток уже не синхронизировать
                    stopping = !stop_sensors(&mut sensors);

                    failures = failures.saturating_add(1);
                    if failures >= FAILURES_BEFORE_ALARM {
                        publish(&app, SourceState::Unavailable { reason });
                    }
                }
            }

            nap(pause);
        }
    });
}

fn publish(app: &AppHandle, state: SourceState) {
    if let SourceState::Ok { snapshot } = &state {
        GRAPHICS.store(if snapshot.roles.gpu.is_some() { 1 } else { 2 }, Ordering::Relaxed);
    }

    if let Some(telemetry) = app.try_state::<Telemetry>() {
        let mut current = telemetry
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *current = state.clone();
    }

    // Неудачу рассылки глотаем: окно могли закрыть прямо в этот момент
    let _ = app.emit(TELEMETRY_EVENT, state);
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lhm::Unit;

    /// Снимок живого железа через сайдкар.
    ///
    /// Права администратора обязательны: без них накопители и плата не читаются
    /// вовсе, а Tctl/Tdie приходит нулём. Это замерено,
    /// поэтому тест на пустом месте не «падает по-непонятному» — он падает ровно
    /// там, где кончаются права.
    /// Снимок с живого железа — и сразу отказ, если прав не хватило.
    ///
    /// ВАЖНО: **без прав администратора сайдкар отвечает, но урезанно.** Из 233
    /// датчиков доходит 115: накопителей нет вовсе, у платы ноль датчиков, а
    /// Tctl/Tdie приходит нулём. Тесты при этом падают на температуре в ноль
    /// и на ненайденном «/Fans/CPU Fan» — то есть выглядят как сломанный
    /// разбор выхлопа, хотя разбор исправен. Ловим это здесь, одним местом на
    /// оба теста, и называем причину вслух.
    fn snapshot() -> crate::lhm::Flat {
        let exe = sensors::locate(None).expect("сайдкар собран: npm run sidecar");
        let mut sensors = Sensors::start(&exe).expect("сайдкар запускается");
        let flat = sensors.poll().expect("снимок приходит");
        let readings = &flat.readings;
        assert!(
            readings.len() > 200,
            "датчиков пришло {}, а с правами приходит за две сотни. \
             Похоже, терминал запущен без прав администратора: плата и \
             накопители тогда не отдают ничего, а Tctl/Tdie приходит нулём. \
             Код тут ни при чём — перезапусти из терминала администратора",
            readings.len()
        );
        flat
    }

    /// Сквозная проверка на живом железе: разговор, разбор, единицы.
    /// Запуск: `cargo test --manifest-path src-tauri/Cargo.toml -- --ignored`
    #[test]
    #[ignore = "нужен собранный сайдкар и права администратора"]
    fn reads_real_hardware() {
        let readings = snapshot().readings;

        let cpu = readings
            .iter()
            .find(|reading| reading.path.ends_with("/Temperatures/Core (Tctl/Tdie)"))
            .expect("датчик Tctl/Tdie на месте");
        let degrees = cpu
            .value
            .expect("Tctl/Tdie пуст — это вопрос PawnIO и прав LHM, а не кода");
        assert!(
            (20.0..120.0).contains(&degrees),
            "неправдоподобная температура процессора: {degrees}"
        );

        // Видеопамять — ключевой показатель, единица обязана быть байтами
        let vram = readings
            .iter()
            .find(|reading| reading.path.ends_with("/Data/GPU Memory Total"))
            .expect("объём видеопамяти на месте");
        assert_eq!(vram.unit, Unit::Byte);
        assert!(
            vram.value.unwrap_or_default() > 8e9,
            "16 ГБ видеопамяти ожидались в байтах, а пришло {:?}",
            vram.value
        );
    }

    /// Роли датчиков на этой машине — ровно те пути, что давали хвосты до
    /// этапа 57, и сверх них напряжение ядра с платы. Так живое железо держит
    /// `lhm/roles.rs`: образцы в его тестах собраны по коду библиотеки, а
    /// этот — по настоящему выхлопу.
    ///
    /// Ловит и прежние беды — переименованный датчик после обновления LHM, — и
    /// новую: роль, ушедшую не туда.
    #[test]
    #[ignore = "нужен собранный сайдкар и права администратора"]
    fn widget_selectors_are_unambiguous() {
        let flat = snapshot();
        let readings = &flat.readings;
        let found = roles::resolve(&flat);

        let expect = |role: &Option<String>, suffix: &str| {
            let path = role
                .as_deref()
                .unwrap_or_else(|| panic!("роль для «{suffix}» не нашлась"));
            assert!(path.ends_with(suffix), "«{suffix}» ушла на «{path}»");
        };

        expect(&found.cpu.load, "/Load/CPU Total");
        assert_eq!(found.cpu.clock.len(), 1, "частота: {:?}", found.cpu.clock);
        assert!(found.cpu.clock[0].ends_with("/Clocks/Cores (Average)"));
        expect(&found.cpu.temperature, "/Temperatures/Core (Tctl/Tdie)");
        expect(&found.cpu.power, "/Powers/Package");
        expect(&found.cpu.voltage, "/Voltages/Vcore");
        expect(&found.board.cpu_fan, "/Fans/CPU Fan");
        expect(&found.board.chassis_fan, "/Fans/Chassis Fan");
        expect(&found.board.temperature, "/Temperatures/Motherboard");

        let gpu = found.gpu.expect("видеокарта найдена");
        expect(&gpu.load, "/Load/GPU Core");
        expect(&gpu.temperature, "/Temperatures/GPU Core");
        expect(&gpu.second_temperature.map(|heat| heat.path), "/Temperatures/GPU Memory Junction");
        expect(&gpu.memory_used, "/Data/GPU Memory Used");
        expect(&gpu.memory_total, "/Data/GPU Memory Total");
        expect(&gpu.memory_load, "/Load/GPU Memory");
        expect(&gpu.clock, "/Clocks/GPU Core");
        expect(&gpu.memory_clock, "/Clocks/GPU Memory");
        expect(&gpu.power, "/Powers/GPU Package");
        expect(&gpu.voltage, "/Voltages/GPU Core Voltage");
        assert_eq!(gpu.fans.len(), 2, "вентиляторы: {:?}", gpu.fans);
        assert!(gpu.fans[0].ends_with("/Fans/GPU Fan 1") && gpu.fans[1].ends_with("/Fans/GPU Fan 2"));

        // Память ищется хвостами по-прежнему: у неё всё общее на любом железе.
        // Продублировано из src/lib/sensors.ts намеренно
        const SINGLE: &[&str] = &[
            "/Total Memory/Load/Memory",
            "/Total Memory/Data/Memory Used",
            "/Total Memory/Data/Memory Available",
        ];

        for selector in SINGLE {
            let hits = readings
                .iter()
                .filter(|reading| reading.path.ends_with(selector))
                .count();
            assert_eq!(
                hits, 1,
                "селектор «{selector}» нашёл показаний: {hits}, а должен ровно одно"
            );
        }

        // Накопители и адаптеры находятся списком, тут проверяем сам факт
        let disks = readings
            .iter()
            .filter(|reading| reading.path.ends_with("/Load/Used Space"))
            .count();
        assert!(disks >= 2, "накопителей найдено {disks}, ожидалось хотя бы два");

        let life = readings
            .iter()
            .filter(|reading| reading.path.ends_with("/Levels/Life"))
            .count();
        assert_eq!(life, disks, "ресурс по SMART нашёлся не у всех накопителей");

        // Пороги Warning и Critical в показания идти не должны
        let measured = readings
            .iter()
            .filter(|reading| {
                reading.path.contains("/Temperatures/")
                    && (reading.name == "Composite Temperature"
                        || reading.name.starts_with("Temperature #"))
            })
            .count();
        assert!(
            measured >= disks * 2,
            "измеряемых температур накопителей всего {measured}, ожидалось больше"
        );

        let adapters = readings
            .iter()
            .filter(|reading| reading.path.ends_with("/Data/Data Downloaded"))
            .count();
        assert!(adapters >= 1, "сетевых адаптеров не найдено вовсе");
    }

    #[test]
    fn no_poller_is_safe_but_an_opening_sidecar_is_not_stopped() {
        let control = SidecarControl::new();
        control.hold(true);
        assert!(control.held());
        assert!(!control.starting(), "удержание запрещает первый запуск");
        control.hold(false);
        assert!(control.starting());
        control.hold(true);
        assert!(control.holding());
        assert!(!control.held(), "первый опрос ещё держит процесс");
        control.stopped();
        assert!(control.held());
        assert!(!control.starting());
        control.hold(false);
        assert!(control.starting(), "после установки запускается новый процесс");
    }

    #[test]
    fn releasing_a_failed_hold_does_not_forget_the_live_process() {
        let control = SidecarControl::new();
        assert!(control.starting());
        control.hold(true);
        assert!(!control.held());
        control.hold(false);
        assert!(!control.starting(), "старый процесс ещё не ушёл");
        control.hold(true);
        assert!(!control.held(), "следующее удержание тоже требует выхода");
        control.stopped();
        assert!(control.held());
    }
}
