//! Процессы: кто именно ест процессор и память.
//!
//! Виджеты железа отвечают на вопрос «сколько занято», но не отвечают на
//! «кем». Диспетчер задач это знает, но его надо открыть, а виджет висит на
//! втором мониторе постоянно.
//!
//! Устройство то же, что у буфера обмена: свой поток ОС, событие в окна и
//! снимок по команде для окна, открытого между тиками. Команд наружу нет
//! вовсе — переключатель ЦПУ/ОЗУ живёт на фронте и выбирает из уже
//! присланного.
//!
//! ВАЖНО: **завершать процессы виджет не умеет, и это решение, а не
//! недоделка.** Окно без рамки, поверх всех, рядом с обоями — не то место, где
//! нажимают «снять задачу»: промах мышью там стоит несохранённого документа.

#[cfg(windows)]
mod memory;

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use serde::Serialize;
use sysinfo::{CpuRefreshKind, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};
use tauri::{AppHandle, Emitter, Manager, State};

/// Метка окна виджета: по ней опрос узнаёт, смотрит ли на него кто-нибудь.
const WIDGET: &str = "processes";

/// Событие, которым списки расходятся по окнам.
const PROCESSES_EVENT: &str = "processes";

/// Шаг опроса.
///
/// Не секунда, как у датчиков, и это выбор: перебор трёх сотен процессов —
/// самый дорогой опрос в приложении, а список, который каждую секунду меняет
/// порядок строк, читать невозможно.
const STEP: Duration = Duration::from_secs(2);

/// Пауза между заходами заводки.
///
/// Ждать ради неё полный шаг незачем: у sysinfo нижний предел двести
/// миллисекунд, берём с запасом.
const PRIME: Duration = Duration::from_millis(300);

/// Сколько заходов уходит на заводку счёта.
///
/// ВАЖНО: **два, а не один.** Проценты считаются от прошлого опроса, и первый
/// заход даёт нули — сравнивать не с чем. Но и второй врёт: система при первой
/// встрече с процессом оставляет его прошлые времена нулями, и разница
/// получается не от прошлого опроса, а от загрузки машины, то есть средним за
/// часы. Осмысленное число даёт только третий заход.
///
/// Поймано живым тестом: на занятом ядре верхняя строка показывала 0,3 %.
const WARMUP: u32 = 2;

/// Сколько строк показываем.
///
/// Хвост из мелочи по 0,1 % бесполезен, а восьми хватает, чтобы виноватый в
/// нагрузке точно попал в список.
const TOP: usize = 8;

/// Одно показание по процессу — то, что отдаёт система, до всякой обработки.
///
/// Отдельным типом ради тестов: вся арифметика виджета работает над ним, и
/// проверить её можно без живых процессов.
#[derive(Debug, Clone, PartialEq)]
struct Sample {
    /// Имя процесса как есть, с расширением.
    name: String,
    /// Проценты от **одного** ядра: занятый целиком процесс на двенадцати
    /// потоках даёт тут 1200.
    cpu: f32,
    /// Частный рабочий набор в байтах — колонка «Память» в диспетчере задач.
    memory: u64,
}

/// Процессы с одним именем, сложенные в одну строку.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    /// Имя без расширения: «msedge», а не «msedge.exe».
    pub name: String,
    /// Проценты от всей машины: сотня — все ядра заняты целиком.
    pub cpu: f32,
    /// Частный рабочий набор в байтах, сумма по группе.
    pub memory: u64,
    /// Сколько процессов с этим именем. Единица в строке не показывается.
    pub count: usize,
}

/// Оба порядка приезжают в одном событии.
///
/// Переключатель ЦПУ/ОЗУ обязан срабатывать мгновенно, а через команду он ждал
/// бы до двух секунд. Шестнадцать строк — меньше килобайта, дешевле любой
/// переписки.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub by_cpu: Vec<Group>,
    pub by_memory: Vec<Group>,
}

/// Имя для показа: без расширения.
///
/// Расширение занимает место в строке шириной в 440 точек и не различает
/// ничего: одноимённых процессов с разными расширениями не бывает.
fn shorten(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((head, tail)) if !head.is_empty() && tail.eq_ignore_ascii_case("exe") => head,
        _ => name,
    }
}

/// Складывает процессы с одним именем.
///
/// Без этого тридцать процессов браузера порознь забивают всю восьмёрку и не
/// отвечают ни на один вопрос: каждый по два процента, а всего — шестьдесят.
///
/// Здесь же проценты приводятся к машине целиком: система считает их от одного
/// ядра, и без деления сумма столбца доходила бы до 1200.
fn group(samples: &[Sample], cores: usize) -> Vec<Group> {
    let cores = cores.max(1) as f32;
    let mut groups: BTreeMap<&str, Group> = BTreeMap::new();

    for sample in samples {
        let name = shorten(&sample.name);
        let group = groups.entry(name).or_insert_with(|| Group {
            name: name.to_owned(),
            cpu: 0.0,
            memory: 0,
            count: 0,
        });

        group.cpu += sample.cpu / cores;
        group.memory = group.memory.saturating_add(sample.memory);
        group.count += 1;
    }

    groups.into_values().collect()
}

/// Восьмёрка по загрузке процессора.
///
/// Равные проценты разбираются памятью, а совсем равные — именем: без этого
/// строки с одинаковыми нулями переставлялись бы местами каждый тик.
fn top_by_cpu(groups: &[Group]) -> Vec<Group> {
    let mut sorted = groups.to_vec();
    sorted.sort_by(|left, right| {
        right
            .cpu
            .total_cmp(&left.cpu)
            .then_with(|| right.memory.cmp(&left.memory))
            .then_with(|| left.name.cmp(&right.name))
    });
    sorted.truncate(TOP);
    sorted
}

/// Восьмёрка по памяти. Порядок разбора тот же, только метрики поменялись
/// местами.
fn top_by_memory(groups: &[Group]) -> Vec<Group> {
    let mut sorted = groups.to_vec();
    sorted.sort_by(|left, right| {
        right
            .memory
            .cmp(&left.memory)
            .then_with(|| right.cpu.total_cmp(&left.cpu))
            .then_with(|| left.name.cmp(&right.name))
    });
    sorted.truncate(TOP);
    sorted
}

/// Снимает процессы: имя, проценты и память.
///
/// Память спрашиваем сами, а не берём у sysinfo: у него это рабочий набор
/// целиком, вместе с общими страницами, и группа из тридцати вкладок считает
/// одни и те же библиотеки тридцать раз. Подробности — в `memory.rs`.
fn collect(system: &System) -> Vec<Sample> {
    system
        .processes()
        .iter()
        // Процесс простоя иначе стоит первой строкой с почти сотней процентов,
        // и виджет не показывает ровно ничего
        .filter(|(pid, _)| pid.as_u32() != 0)
        .map(|(pid, process)| Sample {
            name: process.name().to_string_lossy().into_owned(),
            cpu: process.cpu_usage(),
            memory: private_working_set(pid.as_u32()).unwrap_or_else(|| process.memory()),
        })
        .collect()
}

/// Частный рабочий набор процесса.
///
/// Не дали — берём рабочий набор целиком: у защищённых процессов вроде
/// «Системы безопасности» счётчиков не спросить никак, а строка с нулём
/// означала бы «памяти не ест», что неправда.
fn private_working_set(pid: u32) -> Option<u64> {
    #[cfg(windows)]
    {
        memory::private_working_set(pid)
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        None
    }
}

fn view(samples: &[Sample], cores: usize) -> View {
    let groups = group(samples, cores);

    View {
        by_cpu: top_by_cpu(&groups),
        by_memory: top_by_memory(&groups),
    }
}

/// Последние списки. Нужны окну, открытому между тиками: иначе оно ждало бы
/// две секунды с пустой карточкой.
#[derive(Default)]
pub struct Processes(Mutex<View>);

impl Processes {
    fn locked(&self) -> MutexGuard<'_, View> {
        // Отравленный мьютекс не повод падать: данные внутри осмысленные
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[tauri::command]
pub fn processes_state(state: State<'_, Processes>) -> View {
    state.locked().clone()
}

/// Поднимает поток опроса. Живёт до конца работы приложения.
pub fn spawn(app: AppHandle) {
    if let Err(error) = std::thread::Builder::new()
        .name("processes".to_owned())
        .spawn(move || work(app))
    {
        eprintln!("[процессы] поток не поднялся: {error}");
    }
}

/// Смотрит ли кто-нибудь на виджет.
///
/// Правило звука, а не буфера обмена: истории виджет не копит, и опрашивать
/// систему ради спрятанного окна незачем. Выключенный в настройках виджет сюда
/// попадает тем же путём — его окно спрятано.
fn watched(app: &AppHandle) -> bool {
    app.get_webview_window(WIDGET)
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false)
}

fn publish(app: &AppHandle, next: View) {
    let state = app.state::<Processes>();
    {
        let mut stored = state.locked();
        // Ничего не поменялось — молчим. На живой машине так не бывает почти
        // никогда, но пустой список у спрятанного окна рассылался бы вечно.
        if *stored == next {
            return;
        }
        *stored = next.clone();
    }

    // Неудачу рассылки глотаем: окно могли спрятать прямо в этот момент
    let _ = app.emit(PROCESSES_EVENT, next);
}

fn work(app: AppHandle) {
    let mut system = System::new_with_specifics(RefreshKind::nothing());

    // ВАЖНО: список процессоров нужен не ради числа ядер, а ради самих
    // процентов: sysinfo умножает свою долю на длину этого списка, и с пустым
    // списком все проценты выходят нулями.
    system.refresh_cpu_list(CpuRefreshKind::nothing());
    let cores = system.cpus().len().max(1);

    let wanted = ProcessRefreshKind::nothing().with_cpu().with_memory();
    // Сколько заходов заводки уже сделано: проценты считаются от прошлого
    // опроса, и первые числа выходят враньём
    let mut warmed = 0;

    loop {
        if !watched(&app) {
            // Окно спрятали: счёт придётся заводить заново. Иначе первое же
            // измерение после показа размажет нагрузку по всему времени, что
            // окно было закрыто, и покажет ровную скуку вместо всплеска.
            warmed = 0;
            std::thread::sleep(STEP);
            continue;
        }

        system.refresh_processes_specifics(ProcessesToUpdate::All, true, wanted);

        if warmed < WARMUP {
            warmed += 1;
            std::thread::sleep(PRIME);
            continue;
        }

        publish(&app, view(&collect(&system), cores));
        std::thread::sleep(STEP);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str, cpu: f32, memory: u64) -> Sample {
        Sample {
            name: name.to_owned(),
            cpu,
            memory,
        }
    }

    fn named<'a>(groups: &'a [Group], name: &str) -> &'a Group {
        groups
            .iter()
            .find(|group| group.name == name)
            .unwrap_or_else(|| panic!("группы «{name}» нет"))
    }

    /// Расширение уходит: место в строке дороже, а различает оно ничего.
    #[test]
    fn the_extension_is_dropped() {
        assert_eq!(shorten("msedge.exe"), "msedge");
        assert_eq!(shorten("MSEDGE.EXE"), "MSEDGE");
        assert_eq!(shorten("Registry"), "Registry");
        // Точка в имени — не расширение
        assert_eq!(shorten("node.js.helper"), "node.js.helper");
        // Имя из одного расширения не должно исчезнуть целиком
        assert_eq!(shorten(".exe"), ".exe");
    }

    /// Тридцать процессов браузера — одна строка: каждый по два процента, а
    /// всего шестьдесят, и вопрос «кто ест» отвечается только суммой.
    #[test]
    fn processes_with_one_name_add_up() {
        let samples: Vec<Sample> = (0..30).map(|_| sample("msedge.exe", 24.0, 100)).collect();

        let groups = group(&samples, 12);
        assert_eq!(groups.len(), 1);

        let browser = &groups[0];
        assert_eq!(browser.name, "msedge");
        assert_eq!(browser.count, 30);
        assert_eq!(browser.memory, 3000);
        // 30 × 24 % от одного ядра — это 60 % от машины на двенадцати потоках
        assert!((browser.cpu - 60.0).abs() < 0.01, "вышло {}", browser.cpu);
    }

    /// Проценты приводятся к машине целиком: система считает их от ядра.
    #[test]
    fn percents_are_divided_by_cores() {
        let groups = group(&[sample("stress.exe", 1200.0, 0)], 12);
        assert!((named(&groups, "stress").cpu - 100.0).abs() < 0.01);
    }

    /// Ноль ядер из системы означал бы деление на ноль, то есть бесконечность
    /// в каждой строке.
    #[test]
    fn zero_cores_do_not_blow_up_the_numbers() {
        let groups = group(&[sample("stress.exe", 50.0, 0)], 0);
        assert_eq!(named(&groups, "stress").cpu, 50.0);
    }

    #[test]
    fn the_top_is_eight_rows_long() {
        let samples: Vec<Sample> = (0..40)
            .map(|number| sample(&format!("proc{number:02}.exe"), number as f32, number))
            .collect();

        let view = view(&samples, 1);
        assert_eq!(view.by_cpu.len(), TOP);
        assert_eq!(view.by_memory.len(), TOP);
        assert_eq!(view.by_cpu[0].name, "proc39");
        assert_eq!(view.by_cpu[TOP - 1].name, "proc32");
    }

    /// Два порядка независимы: тяжёлый по памяти и тяжёлый по процессору —
    /// обычно разные процессы, ради этого переключатель и сделан.
    #[test]
    fn the_two_orders_are_sorted_by_their_own_metric() {
        let samples = [
            sample("obs.exe", 40.0, 1),
            sample("idea.exe", 1.0, 8_000_000_000),
            sample("explorer.exe", 5.0, 100_000_000),
        ];

        let view = view(&samples, 1);
        assert_eq!(view.by_cpu[0].name, "obs");
        assert_eq!(view.by_memory[0].name, "idea");
    }

    /// Равные значения разбираются второй метрикой, а совсем равные — именем:
    /// иначе строки переставлялись бы местами каждый тик на ровном месте.
    #[test]
    fn ties_are_broken_the_same_way_every_tick() {
        let samples = [
            sample("вторая.exe", 0.0, 0),
            sample("первая.exe", 0.0, 0),
            sample("третья.exe", 0.0, 5),
        ];

        let view = view(&samples, 1);
        let names: Vec<&str> = view.by_cpu.iter().map(|group| group.name.as_str()).collect();
        assert_eq!(names, ["третья", "вторая", "первая"]);
    }

    /// Пустой снимок — это пустые списки, а не отсутствие полей: карточка
    /// рисуется до первого измерения.
    #[test]
    fn an_empty_view_serializes_to_empty_lists() {
        let json = serde_json::to_string(&View::default()).expect("сериализуется");
        assert_eq!(json, r#"{"byCpu":[],"byMemory":[]}"#);
    }

    /// Разметка уходит на фронт как есть: snake_case там уже однажды всплывал.
    #[test]
    fn view_arrives_in_camel_case() {
        let json = serde_json::to_string(&view(&[sample("msedge.exe", 12.0, 4096)], 1))
            .expect("сериализуется");

        assert!(json.contains(r#""name":"msedge""#), "{json}");
        assert!(json.contains(r#""cpu":12.0"#), "{json}");
        assert!(json.contains(r#""memory":4096"#), "{json}");
        assert!(json.contains(r#""count":1"#), "{json}");
    }

    /// Живой круг: заводим счёт, занимаем ядро и меряем.
    ///
    /// Проверяет ровно то, чего не проверить на выдуманных числах: что система
    /// вообще отдаёт проценты и что заводки хватает. Занимаем ядро своим же
    /// процессом и ищем в списке себя — на тихой машине проверка «верхняя
    /// строка больше нуля» проходила бы и с полностью сломанным счётом.
    ///
    /// Запуск: `cargo test --manifest-path src-tauri/Cargo.toml -- --ignored`
    ///
    /// ВАЖНО: **машина при этом должна быть незанята.** Доля считается от
    /// целого ядра, и параллельная сборка отбирает его у заводки: под `cargo
    /// build` в соседнем окне тест давал 2,5 % вместо восьми и падал так, будто
    /// сломан счёт процентов. Ослаблять границы нельзя — они и проверяют
    /// деление на число ядер, ради которого всё написано.
    #[test]
    #[ignore = "ходит в живую систему и занимает ядро на секунду"]
    fn reads_real_processes() {
        /// Занимает одно ядро на время заводки.
        fn burn(how_long: Duration) {
            let until = std::time::Instant::now() + how_long;
            let mut spin: u64 = 0;
            while std::time::Instant::now() < until {
                spin = spin.wrapping_add(1);
            }
            assert!(spin > 0, "цикл должен был крутиться");
        }

        let mut system = System::new_with_specifics(RefreshKind::nothing());
        system.refresh_cpu_list(CpuRefreshKind::nothing());
        let cores = system.cpus().len().max(1);
        assert!(cores > 1, "процессоров нашлось {cores}");

        let own = std::env::current_exe()
            .ok()
            .and_then(|path| path.file_name().map(|name| name.to_string_lossy().into_owned()))
            .map(|name| shorten(&name).to_owned())
            .expect("своё имя");

        let wanted = ProcessRefreshKind::nothing().with_cpu().with_memory();
        for _ in 0..=WARMUP {
            system.refresh_processes_specifics(ProcessesToUpdate::All, true, wanted);
            burn(PRIME);
        }

        let samples = collect(&system);
        assert!(samples.len() > 50, "процессов нашлось {}", samples.len());
        // Частный рабочий набор всегда меньше рабочего набора целиком, но не
        // нулевой: ноль означал бы, что вызов не прошёл ни разу
        assert!(
            samples.iter().filter(|sample| sample.memory > 0).count() > 40,
            "память нашлась не у всех процессов"
        );

        let view = view(&samples, cores);
        assert_eq!(view.by_cpu.len(), TOP);
        assert!(view.by_memory[0].memory > 10_000_000);

        // Себя мы держали занятым целое ядро: на двенадцати потоках это восемь
        // процентов машины. Ноль тут означает, что заводки не хватило.
        let mine = view
            .by_cpu
            .iter()
            .find(|group| group.name == own)
            .unwrap_or_else(|| panic!("сам тест не попал в восьмёрку: {:?}", view.by_cpu));
        let expected = 100.0 / cores as f32;
        assert!(
            mine.cpu > expected / 2.0 && mine.cpu < expected * 2.0,
            "занятое ядро дало {} %, а ожидалось около {expected}",
            mine.cpu
        );
    }
}
