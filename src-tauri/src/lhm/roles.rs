//! Роли датчиков: какой путь у температуры процессора, какие вентиляторы у
//! видеокарты, откуда брать напряжение ядра (этап 57).
//!
//! До этого этапа виджеты искали датчик хвостом пути по всему дереву и брали
//! первое попадание. На этой машине это работало, а на чужой ломалось двояко:
//! имена у производителей разные (у Intel температура и потребление зовутся
//! `CPU Package`, у видеокарты с одним кулером вентилятор — `GPU Fan` без
//! номера), а на машине со встройкой и дискретной видеокартой обе отдают
//! `Load/GPU Core`, и первой по порядку могла оказаться встройка.
//!
//! Поэтому здесь сперва находится железо — по виду, который шлёт сайдкар, —
//! а потом датчик внутри него по списку имён с запасными. Имена сверены
//! дословно со строками `LibreHardwareMonitorLib` 0.9.6: на другой версии их
//! надо сверить заново.
//!
//! Роли, которой нет, нет и в карточке: ячейка или весь ряд уходят. Роль
//! есть, а значения нет — прочерк. Отличать одно от другого — ровно смысл
//! этого модуля.

use std::collections::BTreeMap;

use serde::Serialize;

use super::{Flat, HardwareKind, Reading};
use crate::settings::PICK_OFF;

/// Встройка AMD с видеопамятью меньше этого — не видеокарта, а кусок
/// оперативной памяти, отданный под экран.
const INTEGRATED_VRAM: f64 = 2.0 * 1024.0 * 1024.0 * 1024.0;

/// Все роли снимка. Путь — полный, как у показания: фронт находит по нему
/// само показание (`findReading`).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Roles {
    pub cpu: CpuRoles,
    pub board: BoardRoles,
    /// `None` — дискретной видеокарты нет: только встройка или ничего.
    pub gpu: Option<GpuRoles>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuRoles {
    pub load: Option<String>,
    /// Один путь (`Cores (Average)` у AMD) или частоты всех ядер: у Intel
    /// средней в выхлопе нет, и её считает фронт.
    pub clock: Vec<String>,
    pub temperature: Option<String>,
    pub power: Option<String>,
    /// Напряжение ядра: у самого процессора, а если там нет — у чипа платы.
    pub voltage: Option<String>,
}

/// Датчики платы, которые стоят в карточке процессора.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardRoles {
    pub cpu_fan: Option<String>,
    pub chassis_fan: Option<String>,
    pub temperature: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Vendor {
    Nvidia,
    Amd,
    Intel,
}

/// Что за вторая температура видеокарты — от этого её подпись.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HeatKind {
    Memory,
    Hotspot,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Heat {
    pub path: String,
    pub kind: HeatKind,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuRoles {
    pub name: String,
    /// Частоту памяти удваиваем только у NVIDIA: там LHM отдаёт физическую, а
    /// как отдаёт AMD — не проверено.
    pub vendor: Vendor,
    pub load: Option<String>,
    pub memory_load: Option<String>,
    pub memory_used: Option<String>,
    pub memory_total: Option<String>,
    pub temperature: Option<String>,
    /// Память, а без неё hot spot: у RTX 50 hot spot закрыт, у RTX 20–40
    /// без GDDR6X нет памяти, у AMD обычно есть обе.
    pub second_temperature: Option<Heat>,
    /// Не больше двух — столько столбцов у строки.
    pub fans: Vec<String>,
    pub clock: Option<String>,
    pub memory_clock: Option<String>,
    pub power: Option<String>,
    pub voltage: Option<String>,
}

/// Датчики, из которых выбирают руками в настройках процессора (этап 58):
/// полные пути, по виду. Живые значения окно настроек берёт из того же
/// снимка.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Choices {
    pub fans: Vec<String>,
    pub temperatures: Vec<String>,
    /// Напряжения и самого процессора, и чипа платы: у Zen 4 ядро видно
    /// только у платы, у Intel — только у процессора.
    pub voltages: Vec<String>,
}

/// Чипы платы, у которых ищутся вентиляторы, температура и напряжение.
///
/// Сама плата датчиков обычно не несёт — они у её чипа, — но искать у неё
/// ничего не стоит, а плата, отдающая их напрямую, иначе осталась бы без них.
fn boards_of(flat: &Flat) -> Vec<usize> {
    hardware_of(flat, |kind| {
        matches!(
            kind,
            HardwareKind::Motherboard | HardwareKind::SuperIo | HardwareKind::EmbeddedController | HardwareKind::Cooler
        )
    })
}

/// Что предложить в ручном выборе: всё подходящего вида у платы, а
/// напряжения — ещё и у процессора.
pub fn choices(flat: &Flat) -> Choices {
    let boards = boards_of(flat);
    let cpus = hardware_of(flat, |kind| kind == HardwareKind::Cpu);
    let of = |hardware: &[usize], kind: &str| -> Vec<String> {
        flat.readings
            .iter()
            .filter(|reading| hardware.contains(&reading.hardware) && reading.kind == kind)
            .map(|reading| reading.path.clone())
            .collect()
    };

    let mut voltages = of(&cpus, "Voltage");
    voltages.extend(of(&boards, "Voltage"));
    Choices {
        fans: of(&boards, "Fan"),
        temperatures: of(&boards, "Temperature"),
        voltages,
    }
}

/// Путь без имени машины — так выбор хранится в настройках, чтобы
/// переименование компьютера его не сбросило.
pub fn without_machine(path: &str) -> &str {
    path.split_once('/').map_or(path, |(_, rest)| rest)
}

/// Ручной выбор поверх найденного (этап 58): он главнее автоматики.
///
/// «Не показывать» — роли нет, и ячейка уходит. Выбранный датчик есть в
/// снимке — роль его. Нет — остаётся найденное автоматикой: сменили плату, и
/// карточка не должна опустеть из-за старой записи.
///
/// Кладётся поверх `Seen`, а не внутрь: память за запуск хранит найденное
/// автоматикой, и снятый выбор возвращает именно его.
pub fn apply_picks(mut roles: Roles, flat: &Flat, picks: &BTreeMap<String, String>) -> Roles {
    for (slot, pick) in picks {
        let (target, kind) = match slot.as_str() {
            "cpuFan" => (&mut roles.board.cpu_fan, "Fan"),
            "chassisFan" => (&mut roles.board.chassis_fan, "Fan"),
            "boardTemp" => (&mut roles.board.temperature, "Temperature"),
            "cpuVoltage" => (&mut roles.cpu.voltage, "Voltage"),
            _ => continue,
        };
        if pick == PICK_OFF {
            *target = None;
            continue;
        }
        // Вид сверяется: правленный руками файл не подсунет градусы в строку
        // вентиляторов
        if let Some(found) = flat
            .readings
            .iter()
            .find(|reading| reading.kind == kind && without_machine(&reading.path) == pick)
        {
            *target = Some(found.path.clone());
        }
    }
    roles
}

/// Находит роли в снимке.
pub fn resolve(flat: &Flat) -> Roles {
    let boards = boards_of(flat);

    Roles {
        cpu: cpu(flat, &boards),
        board: board(flat, &boards),
        gpu: gpu(flat),
    }
}

fn cpu(flat: &Flat, boards: &[usize]) -> CpuRoles {
    let Some(&cpu) = hardware_of(flat, |kind| kind == HardwareKind::Cpu).first() else {
        return CpuRoles::default();
    };
    let at = [cpu];

    CpuRoles {
        load: named(flat, &at, "Load", &["CPU Total"]),
        clock: named(flat, &at, "Clock", &["Cores (Average)"]).map_or_else(
            || {
                flat.readings
                    .iter()
                    .filter(|reading| reading.hardware == cpu && reading.kind == "Clock" && core_clock(&reading.name))
                    .map(|reading| reading.path.clone())
                    .collect()
            },
            |average| vec![average],
        ),
        // Zen 2+ отдаёт одну Tctl/Tdie, Zen 1 — две порознь, и Tdie из них
        // настоящая: у Tctl бывает смещение. У Intel — пакет
        temperature: named(
            flat,
            &at,
            "Temperature",
            &["Core (Tctl/Tdie)", "Core (Tdie)", "CPU Package", "Core (Tctl)", "Core Max", "Core Average"],
        ),
        power: named(flat, &at, "Power", &["Package", "CPU Package"]),
        // У Zen 4 и новее своего напряжения в выхлопе нет вовсе — только у
        // чипа платы, «Vcore» (так у чипов Nuvoton)
        voltage: named(flat, &at, "Voltage", &["Core (SVI2 TFN)", "CPU Core"])
            .or_else(|| named(flat, boards, "Voltage", &["Vcore", "CPU Vcore", "VCore", "CPU Core"])),
    }
}

/// Частота одного ядра. У Intel без гибридных ядер это `CPU Core #3` (а у
/// одноядерного — просто `CPU Core`: `CoreString` в `GenericCpu`), у гибридных
/// `P-Core #1` и `E-Core #8`, у AMD `Core #3`. Без `(Effective)` — та средняя
/// с простоем и признана ненужной.
///
/// `CPU Core #n` нашло ревью второй модели: первая редакция его не пускала, и
/// у Intel до 12-го поколения частота пропала бы из карточки целиком.
fn core_clock(name: &str) -> bool {
    if name == "CPU Core" {
        return true;
    }
    let rest = name
        .strip_prefix("CPU ")
        .or_else(|| name.strip_prefix("P-"))
        .or_else(|| name.strip_prefix("E-"))
        .unwrap_or(name);
    rest.strip_prefix("Core #")
        .is_some_and(|number| !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()))
}

fn board(flat: &Flat, boards: &[usize]) -> BoardRoles {
    BoardRoles {
        // Имена вентиляторов даёт таблица LHM под известную плату; у
        // незнакомой они «Fan #1…», и который из них процессорный — не
        // угадать. Тогда роли нет, а выбрать можно будет в настройках
        cpu_fan: named(flat, boards, "Fan", &["CPU Fan"])
            .or_else(|| spinning_fan(flat, boards, |name| name.starts_with("CPU"))),
        chassis_fan: spinning_fan(flat, boards, |name| {
            ["CHASSIS", "SYSTEM", "CASE", "SYS_", "CHA_"]
                .iter()
                .any(|start| name.starts_with(start))
        }),
        temperature: named(flat, boards, "Temperature", &["Motherboard", "System", "Mainboard"]),
    }
}

/// Вентилятор платы по началу имени (в верхнем регистре), кроме помп.
///
/// Подходящий один — он и есть, вращается он или стоит. Из нескольких —
/// первый вращающийся: пустой разъём отдаёт ноль. Не вращается ни один —
/// роли пока нет. Взять первый наугад нельзя: роль за запуск не меняется
/// (`Seen`), и при старте с остановленными кулерами карточка до выхода
/// показывала бы пустой разъём, а раскрутившийся соседний не заметила бы.
/// Нашло ревью второй модели.
fn spinning_fan(flat: &Flat, boards: &[usize], wanted: impl Fn(&str) -> bool) -> Option<String> {
    let fans: Vec<&Reading> = flat
        .readings
        .iter()
        .filter(|reading| boards.contains(&reading.hardware) && reading.kind == "Fan")
        .filter(|reading| {
            let name = reading.name.to_uppercase();
            wanted(&name) && !name.contains("PUMP")
        })
        .collect();

    let chosen = match fans[..] {
        [only] => Some(only),
        _ => fans.iter().copied().find(|reading| reading.value.is_some_and(|value| value > 0.0)),
    };
    chosen.map(|reading| reading.path.clone())
}

fn gpu(flat: &Flat) -> Option<GpuRoles> {
    let candidates = hardware_of(flat, |kind| {
        matches!(kind, HardwareKind::GpuNvidia | HardwareKind::GpuAmd | HardwareKind::GpuIntel)
    });

    // Из нескольких дискретных — та, у которой больше видеопамяти; при
    // равенстве первая по порядку
    let mut best: Option<(usize, f64)> = None;
    for index in candidates.into_iter().filter(|&index| !integrated(flat, index)) {
        let memory = vram(flat, index).unwrap_or(0.0);
        if best.is_none_or(|(_, most)| memory > most) {
            best = Some((index, memory));
        }
    }
    let (index, _) = best?;
    let hardware = &flat.hardware[index];
    let at = [index];

    let memory_heat = named(flat, &at, "Temperature", &["GPU Memory Junction", "GPU Memory"])
        .map(|path| Heat { path, kind: HeatKind::Memory });
    let hotspot = || {
        named(flat, &at, "Temperature", &["GPU Hot Spot"]).map(|path| Heat { path, kind: HeatKind::Hotspot })
    };

    Some(GpuRoles {
        name: hardware.name.clone(),
        vendor: match hardware.kind {
            HardwareKind::GpuNvidia => Vendor::Nvidia,
            HardwareKind::GpuAmd => Vendor::Amd,
            _ => Vendor::Intel,
        },
        load: named(flat, &at, "Load", &["GPU Core"]),
        memory_load: named(flat, &at, "Load", &["GPU Memory"]),
        memory_used: data(flat, index, &["GPU Memory Used", "D3D Dedicated Memory Used"]),
        memory_total: data(flat, index, &["GPU Memory Total", "D3D Dedicated Memory Total"]),
        temperature: named(flat, &at, "Temperature", &["GPU Core"]),
        second_temperature: memory_heat.or_else(hotspot),
        // Кулер один — `GPU Fan` без номера, у NVIDIA без данных о кулерах —
        // просто `GPU`. Поэтому все вентиляторы этого железа, а не по именам
        fans: flat
            .readings
            .iter()
            .filter(|reading| reading.hardware == index && reading.kind == "Fan")
            .take(2)
            .map(|reading| reading.path.clone())
            .collect(),
        clock: named(flat, &at, "Clock", &["GPU Core"]),
        memory_clock: named(flat, &at, "Clock", &["GPU Memory"]),
        power: named(flat, &at, "Power", &["GPU Package", "GPU PPT", "GPU Power", "GPU Board Power"]),
        // У NVIDIA напряжение зовётся с «Voltage» в конце, у AMD и Intel — нет
        voltage: named(flat, &at, "Voltage", &["GPU Core Voltage", "GPU Core"]),
    })
}

/// Встройка, а не видеокарта.
///
/// У Intel встройка — свой класс LHM, и это видно по идентификатору. AMD APU
/// приходит тем же `gpu-amd`, что и дискретная, а признак встройки у LHM
/// внутренний и ставится только гибридным ноутбукам. Поэтому эвристика: у
/// встроек AMD имя кончается на «Graphics» («AMD Radeon(TM) Graphics»,
/// «Radeon 780M Graphics»), а у дискретных это «Radeon RX …»; и видеопамяти у
/// встройки — выделенный кусок оперативной, обычно полгигабайта.
fn integrated(flat: &Flat, index: usize) -> bool {
    let hardware = &flat.hardware[index];
    match hardware.kind {
        HardwareKind::GpuIntel => hardware.id.starts_with("/gpu-intel-integrated"),
        HardwareKind::GpuAmd => {
            hardware.name.trim().to_lowercase().ends_with("graphics")
                || vram(flat, index).is_some_and(|bytes| bytes < INTEGRATED_VRAM)
        }
        _ => false,
    }
}

/// Видеопамять: у AMD через ADL, а без него — через D3D, как её ищет и
/// карточка. Искать одно имя было мало: APU с одной D3D-памятью проходил бы
/// за дискретную (нашло ревью второй модели).
fn vram(flat: &Flat, index: usize) -> Option<f64> {
    let path = data(flat, index, &["GPU Memory Total", "D3D Dedicated Memory Total"])?;
    flat.readings.iter().find(|reading| reading.path == path)?.value
}

/// Объём: у LHM это Data или SmallData, в выхлопе обе в группе «Data».
fn data(flat: &Flat, index: usize, names: &[&str]) -> Option<String> {
    named(flat, &[index], "SmallData", names).or_else(|| named(flat, &[index], "Data", names))
}

/// Железо нужного вида в порядке снимка.
fn hardware_of(flat: &Flat, wanted: impl Fn(HardwareKind) -> bool) -> Vec<usize> {
    flat.hardware
        .iter()
        .enumerate()
        .filter(|(_, hardware)| wanted(hardware.kind))
        .map(|(index, _)| index)
        .collect()
}

/// Путь первого датчика по списку имён: порядок имён главнее порядка в
/// снимке — «Core (Tdie)» берётся раньше «CPU Package», где бы он ни стоял.
fn named(flat: &Flat, hardware: &[usize], kind: &str, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        flat.readings
            .iter()
            .find(|reading| hardware.contains(&reading.hardware) && reading.kind == kind && reading.name == *name)
            .map(|reading| reading.path.clone())
    })
}

/// Роли, увиденные за этот запуск (защита этапа 57).
///
/// ВАЖНО: **найденная роль не пропадает до выхода.** Датчики LHM бывают
/// пустыми на такт — при сбое драйвера, после обновления драйвера NVIDIA,
/// пока видеокарта не вернулась. Решай карточка по одному снимку, ряды уходили
/// бы и возвращались, а окно дёргалось бы вслед. Поэтому роль, найденная хоть
/// раз, держит своё место, и пропавший датчик показывается прочерком. Исчезает
/// только то, чего не было с самого начала.
///
/// Заодно роль и не меняется: вентилятор платы, выбранный по вращению, не
/// перескочит на соседний, когда первый встанет.
#[derive(Debug, Default)]
pub struct Seen(Roles);

impl Seen {
    /// Дополняет увиденное новым снимком и отдаёт итог.
    pub fn remember(&mut self, now: Roles) -> Roles {
        let seen = &mut self.0;

        keep(&mut seen.cpu.load, now.cpu.load);
        keep_list(&mut seen.cpu.clock, now.cpu.clock);
        keep(&mut seen.cpu.temperature, now.cpu.temperature);
        keep(&mut seen.cpu.power, now.cpu.power);
        keep(&mut seen.cpu.voltage, now.cpu.voltage);

        keep(&mut seen.board.cpu_fan, now.board.cpu_fan);
        keep(&mut seen.board.chassis_fan, now.board.chassis_fan);
        keep(&mut seen.board.temperature, now.board.temperature);

        if let Some(now) = now.gpu {
            if let Some(was) = &mut seen.gpu {
                // Та же видеокарта — дополняем; другая при уже найденной не
                // подменяет её: роли за запуск не меняются
                if was.name == now.name {
                    keep(&mut was.load, now.load);
                    keep(&mut was.memory_load, now.memory_load);
                    keep(&mut was.memory_used, now.memory_used);
                    keep(&mut was.memory_total, now.memory_total);
                    keep(&mut was.temperature, now.temperature);
                    keep(&mut was.second_temperature, now.second_temperature);
                    keep_list(&mut was.fans, now.fans);
                    keep(&mut was.clock, now.clock);
                    keep(&mut was.memory_clock, now.memory_clock);
                    keep(&mut was.power, now.power);
                    keep(&mut was.voltage, now.voltage);
                }
            } else {
                seen.gpu = Some(now);
            }
        }

        seen.clone()
    }
}

fn keep<T>(seen: &mut Option<T>, now: Option<T>) {
    if seen.is_none() {
        *seen = now;
    }
}

/// Список дополняется, если нашлось больше: второй вентилятор, включившийся
/// позже первого, свою ячейку получит.
fn keep_list(seen: &mut Vec<String>, now: Vec<String>) {
    if now.len() > seen.len() {
        *seen = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lhm::{flatten, Node};

    /// Железо образца: имя, вид (как шлёт сайдкар; `None` — старый сайдкар без
    /// вида), идентификатор и датчики «тип LHM, имя, значение».
    struct Part<'a> {
        name: &'a str,
        kind: Option<&'a str>,
        id: &'a str,
        sensors: &'a [(&'a str, &'a str, f64)],
    }

    fn node(text: &str) -> Node {
        Node {
            text: text.to_owned(),
            value: None,
            raw_value: None,
            sensor_id: None,
            kind: None,
            hardware_type: None,
            hardware_id: None,
            children: Vec::new(),
        }
    }

    fn unit(kind: &str) -> &'static str {
        match kind {
            "Temperature" => "°C",
            "Fan" => "RPM",
            "Voltage" => "V",
            "Power" => "W",
            "Clock" => "MHz",
            "Load" | "Control" => "%",
            "SmallData" => "MB",
            _ => "",
        }
    }

    /// Снимок из образцов — через тот же `flatten`, что и живой выхлоп.
    ///
    /// Имена датчиков тут не выдуманы: каждое сверено со строками
    /// `LibreHardwareMonitorLib` 0.9.6. Оговорка честная — это код
    /// библиотеки, а не выхлоп живых машин с таким железом.
    fn machine(parts: &[Part]) -> Flat {
        let mut pc = node("PC");
        for part in parts {
            let mut hardware = node(part.name);
            hardware.hardware_type = part.kind.map(str::to_owned);
            hardware.hardware_id = part.kind.map(|_| part.id.to_owned());

            for (index, (kind, name, value)) in part.sensors.iter().enumerate() {
                let group_name = format!("{kind}s");
                let at = match hardware.children.iter().position(|group| group.text == group_name) {
                    Some(at) => at,
                    None => {
                        hardware.children.push(node(&group_name));
                        hardware.children.len() - 1
                    }
                };

                let mut sensor = node(name);
                sensor.value = Some(format!("{value} {}", unit(kind)));
                sensor.raw_value = Some(format!(" {}", unit(kind)));
                sensor.sensor_id = Some(format!("{}/{}/{index}", part.id, kind.to_lowercase()));
                sensor.kind = Some((*kind).to_owned());
                hardware.children[at].children.push(sensor);
            }
            pc.children.push(hardware);
        }

        let mut root = node("Sensor");
        root.children.push(pc);
        flatten(&root)
    }

    /// Хвост пути: `Temperatures/Core (Tdie)`. Имя машины в проверки не
    /// входит, как и в селекторы.
    fn tail(path: &Option<String>) -> Option<String> {
        path.as_deref().map(|path| path.splitn(3, '/').nth(2).unwrap_or(path).to_owned())
    }

    // ---- Образец: Zen 4, чип Nuvoton и NVIDIA ----

    const ZEN4_CPU: Part = Part {
        name: "AMD Ryzen 7 7700X",
        kind: Some("Cpu"),
        id: "/amdcpu/0",
        sensors: &[
            ("Power", "Package", 43.6),
            ("Clock", "Bus Speed", 99.8),
            ("Clock", "Cores (Average)", 4200.0),
            ("Clock", "Cores (Average Effective)", 838.0),
            ("Clock", "Core #1", 4400.0),
            ("Temperature", "Core (Tctl/Tdie)", 64.9),
            ("Temperature", "CCD1 (Tdie)", 57.5),
            ("Load", "CPU Total", 12.0),
        ],
    };

    const BOARD_CHIP: Part = Part {
        name: "Nuvoton NCT6799D",
        kind: Some("SuperIO"),
        id: "/lpc/nct6799d/0",
        sensors: &[
            ("Voltage", "Vcore", 1.272),
            ("Voltage", "+12V", 12.076),
            ("Temperature", "CPU Core", 60.0),
            ("Temperature", "Motherboard", 43.0),
            ("Fan", "Chassis Fan", 1054.0),
            ("Fan", "CPU Fan", 784.0),
            ("Fan", "Extra Flow Fan", 0.0),
            ("Fan", "AIO Pump", 2591.0),
        ],
    };

    const NVIDIA_GPU: Part = Part {
        name: "NVIDIA GeForce RTX 5070",
        kind: Some("GpuNvidia"),
        id: "/gpu-nvidia/0",
        sensors: &[
            ("Temperature", "GPU Core", 41.8),
            ("Temperature", "GPU Memory Junction", 52.0),
            ("Load", "GPU Core", 10.0),
            ("Load", "GPU Memory", 14.3),
            ("Clock", "GPU Core", 1687.0),
            ("Clock", "GPU Memory", 7001.0),
            ("Fan", "GPU Fan 1", 0.0),
            ("Fan", "GPU Fan 2", 0.0),
            ("Control", "GPU Fan 1", 30.0),
            ("Voltage", "GPU Core Voltage", 0.82),
            ("Power", "GPU Package", 30.7),
            ("SmallData", "GPU Memory Used", 2336.0),
            ("SmallData", "GPU Memory Total", 16303.0),
        ],
    };

    /// Роли образца — ровно те пути, что давали хвосты до этапа 57, и
    /// сверх них напряжение ядра с платы.
    #[test]
    fn this_machine_keeps_its_paths() {
        let roles = resolve(&machine(&[BOARD_CHIP, ZEN4_CPU, NVIDIA_GPU]));

        assert_eq!(tail(&roles.cpu.load).as_deref(), Some("Loads/CPU Total"));
        assert_eq!(roles.cpu.clock.len(), 1);
        assert!(roles.cpu.clock[0].ends_with("/Clocks/Cores (Average)"));
        assert_eq!(tail(&roles.cpu.temperature).as_deref(), Some("Temperatures/Core (Tctl/Tdie)"));
        assert_eq!(tail(&roles.cpu.power).as_deref(), Some("Powers/Package"));
        assert_eq!(tail(&roles.cpu.voltage).as_deref(), Some("Voltages/Vcore"));

        assert_eq!(tail(&roles.board.cpu_fan).as_deref(), Some("Fans/CPU Fan"));
        assert_eq!(tail(&roles.board.chassis_fan).as_deref(), Some("Fans/Chassis Fan"));
        assert_eq!(tail(&roles.board.temperature).as_deref(), Some("Temperatures/Motherboard"));

        let gpu = roles.gpu.expect("видеокарта найдена");
        assert_eq!(gpu.vendor, Vendor::Nvidia);
        assert_eq!(gpu.name, "NVIDIA GeForce RTX 5070");
        let heat = gpu.second_temperature.expect("память есть");
        assert_eq!(heat.kind, HeatKind::Memory);
        assert!(heat.path.ends_with("GPU Memory Junction"));
        // Регуляторы — не вентиляторы, хотя зовутся так же
        assert_eq!(gpu.fans.len(), 2);
        assert!(gpu.fans.iter().all(|fan| fan.contains("/Fans/")));
        assert_eq!(tail(&gpu.voltage).as_deref(), Some("Voltages/GPU Core Voltage"));
        assert_eq!(tail(&gpu.power).as_deref(), Some("Powers/GPU Package"));
        assert_eq!(tail(&gpu.memory_total).as_deref(), Some("SmallDatas/GPU Memory Total"));
    }

    /// Старый сайдкар вида не шлёт: роли те же, вид выводится из `SensorId`.
    #[test]
    fn an_old_sidecar_still_fills_the_cards() {
        let untyped = |part: Part<'static>| Part { kind: None, ..part };
        let typed = resolve(&machine(&[BOARD_CHIP, ZEN4_CPU, NVIDIA_GPU]));
        let old = resolve(&machine(&[untyped(BOARD_CHIP), untyped(ZEN4_CPU), untyped(NVIDIA_GPU)]));

        assert_eq!(old, typed);
    }

    // ---- Чужое железо по коду библиотеки ----

    #[test]
    fn intel_hybrid_averages_its_cores_and_reads_the_package() {
        let roles = resolve(&machine(&[Part {
            name: "13th Gen Intel Core i5-13600K",
            kind: Some("Cpu"),
            id: "/intelcpu/0",
            sensors: &[
                ("Load", "CPU Total", 5.0),
                ("Clock", "Bus Speed", 100.0),
                ("Clock", "P-Core #1", 5100.0),
                ("Clock", "P-Core #2", 5000.0),
                ("Clock", "E-Core #1", 3900.0),
                ("Temperature", "P-Core #1", 44.0),
                ("Temperature", "Core Max", 48.0),
                ("Temperature", "CPU Package", 50.0),
                ("Power", "CPU Cores", 20.0),
                ("Power", "CPU Package", 31.0),
                ("Voltage", "CPU Core", 1.1),
            ],
        }]));

        assert_eq!(roles.cpu.clock.len(), 3, "{:?}", roles.cpu.clock);
        assert!(roles.cpu.clock.iter().all(|path| path.contains("Core #")));
        assert_eq!(tail(&roles.cpu.temperature).as_deref(), Some("Temperatures/CPU Package"));
        assert_eq!(tail(&roles.cpu.power).as_deref(), Some("Powers/CPU Package"));
        assert_eq!(tail(&roles.cpu.voltage).as_deref(), Some("Voltages/CPU Core"));
    }

    #[test]
    fn zen_one_takes_tdie_over_tctl() {
        let roles = resolve(&machine(&[Part {
            name: "AMD Ryzen 7 1700X",
            kind: Some("Cpu"),
            id: "/amdcpu/0",
            sensors: &[("Temperature", "Core (Tctl)", 70.0), ("Temperature", "Core (Tdie)", 50.0)],
        }]));

        assert_eq!(tail(&roles.cpu.temperature).as_deref(), Some("Temperatures/Core (Tdie)"));
    }

    /// Карточка с одним кулером: `GPU Fan` без номера. Прежний селектор
    /// «GPU Fan 1» его не видел и показывал «— / —».
    #[test]
    fn a_single_fan_card_has_one_fan() {
        let roles = resolve(&machine(&[Part {
            name: "NVIDIA GeForce RTX 4060",
            kind: Some("GpuNvidia"),
            id: "/gpu-nvidia/0",
            sensors: &[
                ("Temperature", "GPU Core", 50.0),
                ("Temperature", "GPU Hot Spot", 61.0),
                ("Fan", "GPU Fan", 1200.0),
                ("SmallData", "GPU Memory Total", 8188.0),
            ],
        }]));

        let gpu = roles.gpu.expect("видеокарта найдена");
        assert_eq!(gpu.fans.len(), 1);
        assert!(gpu.fans[0].ends_with("/GPU Fan"));
        // Памяти нет — второй температурой идёт hot spot, со своей подписью
        let heat = gpu.second_temperature.expect("hot spot есть");
        assert_eq!(heat.kind, HeatKind::Hotspot);
        assert_eq!(gpu.voltage, None);
    }

    #[test]
    fn a_radeon_names_things_its_own_way() {
        let roles = resolve(&machine(&[Part {
            name: "AMD Radeon RX 7800 XT",
            kind: Some("GpuAmd"),
            id: "/gpu-amd/0",
            sensors: &[
                ("Temperature", "GPU Core", 55.0),
                ("Temperature", "GPU Hot Spot", 70.0),
                ("Temperature", "GPU Memory", 60.0),
                ("Fan", "GPU Fan", 900.0),
                ("Control", "GPU Fan", 25.0),
                ("Voltage", "GPU Core", 0.9),
                ("Power", "GPU Core", 150.0),
                ("Power", "GPU PPT", 180.0),
                ("Load", "GPU Core", 50.0),
                ("SmallData", "GPU Memory Total", 16368.0),
            ],
        }]));

        let gpu = roles.gpu.expect("видеокарта найдена");
        assert_eq!(gpu.vendor, Vendor::Amd);
        assert_eq!(gpu.second_temperature.expect("память").kind, HeatKind::Memory);
        assert_eq!(gpu.fans.len(), 1);
        assert_eq!(tail(&gpu.voltage).as_deref(), Some("Voltages/GPU Core"));
        assert_eq!(tail(&gpu.power).as_deref(), Some("Powers/GPU PPT"));
    }

    /// Встройка без дискретной — видеокарты нет, как бы та ни называлась.
    #[test]
    fn integrated_graphics_are_not_a_graphics_card() {
        let apu = |name: &'static str, megabytes: f64| Part {
            name,
            kind: Some("GpuAmd"),
            id: "/gpu-amd/0",
            sensors: Box::leak(Box::new([
                ("Load", "GPU Core", 3.0),
                ("SmallData", "GPU Memory Total", megabytes),
            ])),
        };

        // Имя на «Graphics» — даже с большим выделением памяти
        assert_eq!(resolve(&machine(&[apu("AMD Radeon(TM) Graphics", 512.0)])).gpu, None);
        assert_eq!(resolve(&machine(&[apu("AMD Radeon 8060S Graphics", 16384.0)])).gpu, None);
        // Имя иное, но памяти как у встройки
        assert_eq!(resolve(&machine(&[apu("AMD Radeon 780M", 512.0)])).gpu, None);

        let intel = Part {
            name: "Intel(R) UHD Graphics 770",
            kind: Some("GpuIntel"),
            id: "/gpu-intel-integrated/pci_8086_4680",
            sensors: &[("Load", "D3D 3D", 2.0)],
        };
        assert_eq!(resolve(&machine(&[intel])).gpu, None);
    }

    /// Ноутбук и настольный Ryzen со встройкой: встройка стоит первой, но
    /// карточка берёт дискретную.
    #[test]
    fn the_discrete_card_wins_over_the_integrated_one() {
        let igpu = Part {
            name: "AMD Radeon(TM) Graphics",
            kind: Some("GpuAmd"),
            id: "/gpu-amd/0",
            sensors: &[("Load", "GPU Core", 3.0), ("SmallData", "GPU Memory Total", 512.0)],
        };
        let intel = Part {
            name: "Intel(R) Iris(R) Xe Graphics",
            kind: Some("GpuIntel"),
            id: "/gpu-intel-integrated/pci_8086_9a49",
            sensors: &[("Load", "D3D 3D", 2.0)],
        };

        for first in [igpu, intel] {
            let roles = resolve(&machine(&[first, NVIDIA_GPU]));
            assert_eq!(roles.gpu.expect("дискретная найдена").name, "NVIDIA GeForce RTX 5070");
        }
    }

    #[test]
    fn of_two_cards_the_bigger_one_is_shown() {
        let small = Part {
            name: "AMD Radeon RX 6600",
            kind: Some("GpuAmd"),
            id: "/gpu-amd/0",
            sensors: &[("SmallData", "GPU Memory Total", 8176.0)],
        };

        let roles = resolve(&machine(&[small, NVIDIA_GPU]));
        assert_eq!(roles.gpu.expect("найдена").name, "NVIDIA GeForce RTX 5070");
    }

    /// Незнакомая плата: вентиляторы «Fan #n», температуры «Temperature #n».
    /// Угадывать нельзя — ролей нет, и ряды уйдут.
    #[test]
    fn an_unknown_board_is_not_guessed() {
        let roles = resolve(&machine(&[
            ZEN4_CPU,
            Part {
                name: "ITE IT8689E",
                kind: Some("SuperIO"),
                id: "/lpc/it8689e/0",
                sensors: &[
                    ("Voltage", "Voltage #1", 1.2),
                    ("Temperature", "Temperature #1", 40.0),
                    ("Fan", "Fan #1", 800.0),
                    ("Fan", "Fan #2", 1100.0),
                ],
            },
        ]));

        assert_eq!(roles.board, BoardRoles::default());
        assert_eq!(roles.cpu.voltage, None);
        assert_eq!(roles.gpu, None);
    }

    /// Помпа — не вентилятор корпуса, а стоящий разъём уступает вращающемуся.
    #[test]
    fn a_chassis_fan_spins_and_is_not_a_pump() {
        let roles = resolve(&machine(&[Part {
            name: "Nuvoton NCT6798D",
            kind: Some("SuperIO"),
            id: "/lpc/nct6798d/0",
            sensors: &[
                ("Fan", "System Fan #3 / Pump", 2500.0),
                ("Fan", "Chassis Fan #1", 0.0),
                ("Fan", "Chassis Fan #2", 950.0),
            ],
        }]));

        assert_eq!(tail(&roles.board.chassis_fan).as_deref(), Some("Fans/Chassis Fan #2"));
    }

    #[test]
    fn core_clocks_are_told_from_the_rest() {
        for name in ["Core #1", "Core #12", "P-Core #3", "E-Core #8", "CPU Core #1", "CPU Core"] {
            assert!(core_clock(name), "{name}");
        }
        for name in ["Core #1 (Effective)", "Cores (Average)", "Bus Speed", "Core #", "CPU Core #"] {
            assert!(!core_clock(name), "{name}");
        }
    }

    /// Intel до 12-го поколения: частоты ядер — `CPU Core #n`, средней нет.
    #[test]
    fn an_older_intel_averages_its_cpu_cores() {
        let roles = resolve(&machine(&[Part {
            name: "Intel Core i7-10700K",
            kind: Some("Cpu"),
            id: "/intelcpu/0",
            sensors: &[
                ("Clock", "Bus Speed", 100.0),
                ("Clock", "CPU Core #1", 4700.0),
                ("Clock", "CPU Core #2", 4600.0),
                ("Temperature", "CPU Core #1", 50.0),
                ("Temperature", "CPU Package", 55.0),
            ],
        }]));

        assert_eq!(roles.cpu.clock.len(), 2, "{:?}", roles.cpu.clock);
        assert_eq!(tail(&roles.cpu.temperature).as_deref(), Some("Temperatures/CPU Package"));
    }

    /// APU, чья видеопамять видна только через D3D, — всё равно встройка.
    #[test]
    fn an_apu_with_only_d3d_memory_is_integrated() {
        let apu = Part {
            name: "AMD Radeon 780M",
            kind: Some("GpuAmd"),
            id: "/gpu-amd/0",
            sensors: &[("SmallData", "D3D Dedicated Memory Total", 512.0)],
        };
        assert_eq!(resolve(&machine(&[apu])).gpu, None);
    }

    /// Плата, отдающая датчики сама, а не через чип.
    #[test]
    fn a_board_with_its_own_sensors_is_read_too() {
        let roles = resolve(&machine(&[Part {
            name: "Some Board",
            kind: Some("Motherboard"),
            id: "/motherboard",
            sensors: &[("Temperature", "Motherboard", 40.0)],
        }]));
        assert_eq!(tail(&roles.board.temperature).as_deref(), Some("Temperatures/Motherboard"));
    }

    /// Разъёмов несколько и все стоят — роли нет, пока какой-то не
    /// раскрутится; тогда её находит следующий такт. Разъём один — он и есть,
    /// даже стоящий.
    #[test]
    fn stopped_fans_are_not_guessed_but_a_single_one_is_taken() {
        let board = |first: f64, second: f64| Part {
            name: "Nuvoton NCT6798D",
            kind: Some("SuperIO"),
            id: "/lpc/nct6798d/0",
            sensors: Box::leak(Box::new([("Fan", "Chassis Fan #1", first), ("Fan", "Chassis Fan #2", second)])),
        };

        let mut seen = Seen::default();
        let quiet = seen.remember(resolve(&machine(&[board(0.0, 0.0)])));
        assert_eq!(quiet.board.chassis_fan, None);

        let spun = seen.remember(resolve(&machine(&[board(0.0, 900.0)])));
        assert_eq!(tail(&spun.board.chassis_fan).as_deref(), Some("Fans/Chassis Fan #2"));

        let single = Part {
            name: "Nuvoton NCT6799D",
            kind: Some("SuperIO"),
            id: "/lpc/nct6799d/0",
            sensors: &[("Fan", "Chassis Fan", 0.0)],
        };
        let roles = resolve(&machine(&[single]));
        assert_eq!(tail(&roles.board.chassis_fan).as_deref(), Some("Fans/Chassis Fan"));
    }

    // ---- Ручной выбор (этап 58) ----

    fn picks(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries.iter().map(|(slot, pick)| ((*slot).to_owned(), (*pick).to_owned())).collect()
    }

    /// Выбор главнее автоматики; «не показывать» снимает роль.
    #[test]
    fn a_pick_outranks_the_automatic_choice() {
        let flat = machine(&[BOARD_CHIP, ZEN4_CPU]);
        let roles = apply_picks(
            resolve(&flat),
            &flat,
            &picks(&[("chassisFan", "Nuvoton NCT6799D/Fans/Extra Flow Fan"), ("cpuVoltage", "off")]),
        );

        assert_eq!(tail(&roles.board.chassis_fan).as_deref(), Some("Fans/Extra Flow Fan"));
        assert_eq!(roles.cpu.voltage, None);
        // Нетронутое — как нашла автоматика
        assert_eq!(tail(&roles.board.cpu_fan).as_deref(), Some("Fans/CPU Fan"));
    }

    /// Выбранного датчика нет (сменили плату) — остаётся автоматика, а не
    /// пустая ячейка. Датчик чужого вида тоже не берётся.
    #[test]
    fn a_missing_or_wrong_pick_falls_back_to_automatic() {
        let flat = machine(&[BOARD_CHIP]);
        let roles = apply_picks(
            resolve(&flat),
            &flat,
            &picks(&[
                ("cpuFan", "Old Board/Old Chip/Fans/Fan #3"),
                ("boardTemp", "Nuvoton NCT6799D/Fans/CPU Fan"),
            ]),
        );

        assert_eq!(tail(&roles.board.cpu_fan).as_deref(), Some("Fans/CPU Fan"));
        assert_eq!(tail(&roles.board.temperature).as_deref(), Some("Temperatures/Motherboard"));
    }

    /// Незнакомая плата: автоматика молчит, выбор находит.
    #[test]
    fn a_pick_names_a_fan_on_an_unknown_board() {
        let flat = machine(&[Part {
            name: "ITE IT8689E",
            kind: Some("SuperIO"),
            id: "/lpc/it8689e/0",
            sensors: &[("Fan", "Fan #1", 800.0), ("Fan", "Fan #2", 1100.0)],
        }]);
        assert_eq!(resolve(&flat).board.cpu_fan, None);

        let roles = apply_picks(resolve(&flat), &flat, &picks(&[("cpuFan", "ITE IT8689E/Fans/Fan #2")]));
        assert_eq!(tail(&roles.board.cpu_fan).as_deref(), Some("Fans/Fan #2"));
    }

    /// Что предложить в выборе: всё у платы, напряжения — и у процессора.
    #[test]
    fn choices_list_the_board_and_the_cpu_voltages() {
        let flat = machine(&[
            BOARD_CHIP,
            Part {
                name: "13th Gen Intel Core i5-13600K",
                kind: Some("Cpu"),
                id: "/intelcpu/0",
                sensors: &[("Voltage", "CPU Core", 1.1), ("Temperature", "CPU Package", 50.0)],
            },
            NVIDIA_GPU,
        ]);
        let found = choices(&flat);

        assert_eq!(found.fans.len(), 4, "{:?}", found.fans);
        assert!(found.fans.iter().all(|path| path.contains("Nuvoton")));
        assert_eq!(found.temperatures.len(), 2);
        assert_eq!(found.voltages.len(), 3);
        assert!(found.voltages[0].ends_with("/CPU Core"), "процессор первым: {:?}", found.voltages);
        assert_eq!(without_machine("PC/Board/Fans/Fan #1"), "Board/Fans/Fan #1");
    }

    // ---- Увиденное за запуск ----

    /// Датчик пропал — роль держит место; видеокарта пропала — остаётся.
    #[test]
    fn what_was_seen_stays_until_exit() {
        let mut seen = Seen::default();
        let full = seen.remember(resolve(&machine(&[BOARD_CHIP, ZEN4_CPU, NVIDIA_GPU])));
        assert!(full.gpu.is_some());

        // Такт, в котором не ответили ни драйвер NVIDIA, ни плата
        let bare = seen.remember(resolve(&machine(&[ZEN4_CPU])));
        assert_eq!(bare, full);
    }

    /// Найденное позже дополняет роли, а найденное раньше не подменяется.
    #[test]
    fn later_finds_fill_gaps_but_do_not_replace() {
        let mut seen = Seen::default();
        let first = seen.remember(resolve(&machine(&[ZEN4_CPU])));
        assert_eq!(first.gpu, None);
        assert_eq!(first.board.cpu_fan, None);

        let later = seen.remember(resolve(&machine(&[BOARD_CHIP, ZEN4_CPU, NVIDIA_GPU])));
        assert!(later.gpu.is_some());
        assert!(later.board.cpu_fan.is_some());

        // Другая видеокарта при уже найденной её не подменяет
        let other = Part { name: "AMD Radeon RX 7900 XTX", kind: Some("GpuAmd"), id: "/gpu-amd/0", sensors: &[] };
        let after = seen.remember(resolve(&machine(&[ZEN4_CPU, other])));
        assert_eq!(after.gpu.expect("прежняя").name, "NVIDIA GeForce RTX 5070");
    }

    /// Форма, которую читает фронт (`Roles` в `src/lib/telemetry.ts`).
    #[test]
    fn roles_keep_the_shape_the_front_reads() {
        let roles = resolve(&machine(&[BOARD_CHIP, ZEN4_CPU, NVIDIA_GPU]));
        let json = serde_json::to_value(&roles).expect("сериализуется");

        assert!(json["cpu"]["clock"].is_array());
        assert!(json["cpu"]["voltage"].is_string());
        assert!(json["board"]["cpuFan"].is_string());
        assert!(json["board"]["chassisFan"].is_string());
        assert_eq!(json["gpu"]["vendor"], "nvidia");
        assert_eq!(json["gpu"]["secondTemperature"]["kind"], "memory");
        assert!(json["gpu"]["memoryClock"].is_string());
        assert!(json["gpu"]["fans"].is_array());

        let none = serde_json::to_value(resolve(&machine(&[ZEN4_CPU]))).expect("сериализуется");
        assert!(none["gpu"].is_null());
    }
}
