use std::collections::HashMap;

use serde::Serialize;

use super::{parse, Node, Unit};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reading {
    /// Полный путь по дереву — ключ датчика.
    pub path: String,
    /// Последний сегмент пути, например «Core (Tctl/Tdie)».
    pub name: String,
    /// Идентификатор LHM. Только справочно: он не уникален.
    pub sensor_id: String,
    /// Тип датчика по версии LHM: Temperature, Load, Throughput и так далее.
    pub kind: String,
    pub value: Option<f64>,
    pub unit: Unit,
    /// Номер железа в [`Flat::hardware`]. В окна не едет: железо ищет Rust
    /// (`roles.rs`), виджетам достаются уже найденные пути.
    #[serde(skip)]
    pub hardware: usize,
}

/// Вид железа. Нужен ролям датчиков: сперва находится железо, потом датчик
/// внутри него.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardwareKind {
    Cpu,
    GpuNvidia,
    GpuAmd,
    GpuIntel,
    Memory,
    Motherboard,
    SuperIo,
    EmbeddedController,
    Cooler,
    Storage,
    Network,
    Other,
}

impl HardwareKind {
    /// Из поля `HardwareType` сайдкара — имени перечисления LHM.
    fn from_type(name: &str) -> Self {
        match name {
            "Cpu" => Self::Cpu,
            "GpuNvidia" => Self::GpuNvidia,
            "GpuAmd" => Self::GpuAmd,
            "GpuIntel" => Self::GpuIntel,
            "Memory" => Self::Memory,
            "Motherboard" => Self::Motherboard,
            "SuperIO" => Self::SuperIo,
            "EmbeddedController" => Self::EmbeddedController,
            "Cooler" => Self::Cooler,
            "Storage" => Self::Storage,
            "Network" => Self::Network,
            _ => Self::Other,
        }
    }

    /// Из первого сегмента идентификатора: `/amdcpu/0`, `/gpu-nvidia/0`,
    /// `/lpc/nct6799d/0`.
    ///
    /// Запасной путь, а не основной (защита этапа 57): сайдкар, собранный до
    /// появления `HardwareType`, иначе оставил бы все роли пустыми, и
    /// карточки опустели бы без единой ошибки. Имена сегментов — внутренность
    /// LHM, поэтому первым всегда идёт поле.
    fn from_id(id: &str) -> Self {
        let first = id.trim_start_matches('/').split('/').next().unwrap_or("");
        match first {
            "amdcpu" | "intelcpu" | "genericcpu" | "cpu" => Self::Cpu,
            "gpu-nvidia" => Self::GpuNvidia,
            "gpu-amd" => Self::GpuAmd,
            intel if intel.starts_with("gpu-intel") => Self::GpuIntel,
            "ram" | "vram" | "memory" => Self::Memory,
            "motherboard" => Self::Motherboard,
            "lpc" => Self::SuperIo,
            "ec" | "embeddedcontroller" => Self::EmbeddedController,
            "nvme" | "hdd" | "ssd" | "storage" => Self::Storage,
            "nic" => Self::Network,
            _ => Self::Other,
        }
    }
}

/// Железо снимка: процессор, видеокарта, чип платы и прочие.
#[derive(Debug, Clone)]
pub struct Hardware {
    /// Путь узла железа, с именем машины в начале.
    pub path: String,
    /// Имя, например «NVIDIA GeForce RTX 5070».
    pub name: String,
    pub kind: HardwareKind,
    /// Идентификатор LHM, например `/gpu-nvidia/0`.
    pub id: String,
}

/// Развёрнутый снимок: датчики списком и железо, к которому они относятся.
#[derive(Debug, Clone, Default)]
pub struct Flat {
    pub readings: Vec<Reading>,
    pub hardware: Vec<Hardware>,
}

/// Разворачивает дерево LHM в плоский список.
///
/// Ключом служит полный путь, а не `SensorId`: в выхлопе этой машины
/// «Load/GPU Memory» и «Load/GPU Bus» имеют один и тот же идентификатор
/// `/gpu-nvidia/0/load/3`. Полагаться на индексы датчиков тоже нельзя —
/// сопоставляем строго по именам.
pub fn flatten(root: &Node) -> Flat {
    let mut flat = Flat::default();

    // Корень — служебная шапка («Sensor» со значением «Value»), в путь не входит
    for child in &root.children {
        walk(child, Level::TOP, Level::TOP, None, &mut flat);
    }

    disambiguate(&mut flat.readings);
    flat
}

/// Узел над текущим: его путь и имя.
#[derive(Clone, Copy)]
struct Level<'a> {
    path: &'a str,
    name: &'a str,
}

impl Level<'_> {
    const TOP: Level<'static> = Level { path: "", name: "" };
}

/// `parent` — узел над текущим, `grand` — узел над ним. У датчика родитель —
/// группа («Temperatures»), а над ней железо, то есть `grand`. Отрезать железо
/// от пути датчика по косой нельзя — она бывает в самом имени, как в
/// «Core (Tctl/Tdie)» или в модели планки памяти.
fn walk(node: &Node, parent: Level, grand: Level, owner: Option<usize>, flat: &mut Flat) {
    let path = if parent.path.is_empty() {
        node.text.clone()
    } else {
        format!("{}/{}", parent.path, node.text)
    };

    // Узел железа несёт свой вид — и передаёт его всем датчикам ниже, в том
    // числе подчинённому железу, пока у того не найдётся своего
    let owner = match &node.hardware_type {
        Some(kind) => {
            flat.hardware.push(Hardware {
                path: path.clone(),
                name: node.text.clone(),
                kind: HardwareKind::from_type(kind),
                id: node.hardware_id.clone().unwrap_or_default(),
            });
            Some(flat.hardware.len() - 1)
        }
        None => owner,
    };

    // Настоящий датчик отличается от группирующего узла наличием SensorId
    if let Some(sensor_id) = &node.sensor_id {
        let measurement = parse::parse(node.value.as_deref(), node.raw_value.as_deref());
        let hardware = owner.unwrap_or_else(|| guessed_owner(grand, sensor_id, flat));
        flat.readings.push(Reading {
            path: path.clone(),
            name: node.text.clone(),
            sensor_id: sensor_id.clone(),
            kind: node.kind.clone().unwrap_or_default(),
            value: measurement.value,
            unit: measurement.unit,
            hardware,
        });
    }

    let here = Level { path: &path, name: &node.text };
    for child in &node.children {
        walk(child, here, parent, owner, flat);
    }
}

/// Железо датчика, когда сайдкар не прислал вида: путь железа — путь узла
/// над группой, а идентификатор — `SensorId` без типа и номера
/// (`/amdcpu/0/temperature/2` → `/amdcpu/0`).
fn guessed_owner(hardware: Level, sensor_id: &str, flat: &mut Flat) -> usize {
    if let Some(found) = flat.hardware.iter().position(|known| known.path == hardware.path) {
        return found;
    }

    let id = cut(cut(sensor_id)).to_owned();
    flat.hardware.push(Hardware {
        path: hardware.path.to_owned(),
        name: hardware.name.to_owned(),
        kind: HardwareKind::from_id(&id),
        id,
    });
    flat.hardware.len() - 1
}

/// Идентификатор без последнего сегмента. В идентификаторах косая — только
/// разделитель, в отличие от имён.
fn cut(id: &str) -> &str {
    id.rfind('/').map_or("", |slash| &id[..slash])
}

/// Разводит совпадающие пути номерами.
///
/// У видеокарты шесть датчиков подряд называются «D3D Copy» и лежат в одной
/// группе, так что полный путь у них общий. Ни один виджет их не использует,
/// но список должен оставаться адресуемым, поэтому повторам добавляем номер.
fn disambiguate(readings: &mut [Reading]) {
    let mut seen: HashMap<String, u32> = HashMap::new();

    for reading in readings.iter_mut() {
        let seen_times = seen.entry(reading.path.clone()).or_insert(0);
        *seen_times += 1;
        if *seen_times > 1 {
            reading.path = format!("{} #{}", reading.path, seen_times);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Фрагмент настоящего выхлопа: шапка, машина, железо, группа, датчики.
    // Вида железа тут нет — так выглядел и веб-сервер LHM, и сайдкар до этапа 57
    const SAMPLE: &str = r#"{
      "Text": "Sensor", "Value": "Value", "Children": [
        { "Text": "MYPC", "Children": [
          { "Text": "AMD Ryzen 7 7700X", "Children": [
            { "Text": "Temperatures", "Children": [
              { "Text": "Core (Tctl/Tdie)", "Value": "62,4 °C", "RawValue": "62,4 °C",
                "SensorId": "/amdcpu/0/temperature/2", "Type": "Temperature", "Children": [] }
            ]}
          ]},
          { "Text": "NVIDIA GeForce RTX 5070", "Children": [
            { "Text": "Load", "Children": [
              { "Text": "D3D Copy", "Value": "0,0 %", "RawValue": "0,0 %",
                "SensorId": "/gpu-nvidia/0/load/8", "Type": "Load", "Children": [] },
              { "Text": "D3D Copy", "Value": "0,1 %", "RawValue": "0,1 %",
                "SensorId": "/gpu-nvidia/0/load/9", "Type": "Load", "Children": [] }
            ]}
          ]}
        ]}
      ]
    }"#;

    /// Тот же фрагмент от нынешнего сайдкара: у узлов железа вид и
    /// идентификатор, а у платы подчинённый чип со своим видом.
    const TYPED: &str = r#"{
      "Text": "Sensor", "Children": [
        { "Text": "MYPC", "Children": [
          { "Text": "ASUS TUF GAMING B650-PLUS", "HardwareType": "Motherboard",
            "HardwareId": "/motherboard", "Children": [
            { "Text": "Nuvoton NCT6799D", "HardwareType": "SuperIO",
              "HardwareId": "/lpc/nct6799d/0", "Children": [
              { "Text": "Fans", "Children": [
                { "Text": "CPU Fan", "Value": "784 RPM", "RawValue": " RPM",
                  "SensorId": "/lpc/nct6799d/0/fan/1", "Type": "Fan", "Children": [] }
              ]}
            ]}
          ]},
          { "Text": "NVIDIA GeForce RTX 5070", "HardwareType": "GpuNvidia",
            "HardwareId": "/gpu-nvidia/0", "Children": [
            { "Text": "Load", "Children": [
              { "Text": "GPU Core", "Value": "10.0 %", "RawValue": " %",
                "SensorId": "/gpu-nvidia/0/load/0", "Type": "Load", "Children": [] }
            ]}
          ]}
        ]}
      ]
    }"#;

    fn sample(json: &str) -> Flat {
        let root: Node = serde_json::from_str(json).expect("образец разбирается");
        flatten(&root)
    }

    #[test]
    fn builds_full_path_without_root_segment() {
        let readings = sample(SAMPLE).readings;
        let cpu = &readings[0];
        assert_eq!(
            cpu.path,
            "MYPC/AMD Ryzen 7 7700X/Temperatures/Core (Tctl/Tdie)"
        );
        assert_eq!(cpu.name, "Core (Tctl/Tdie)");
        assert_eq!(cpu.kind, "Temperature");
        assert_eq!(cpu.value, Some(62.4));
    }

    #[test]
    fn skips_grouping_nodes() {
        // Три датчика, а узлов в дереве заметно больше
        assert_eq!(sample(SAMPLE).readings.len(), 3);
    }

    #[test]
    fn numbers_repeated_paths() {
        let readings = sample(SAMPLE).readings;
        assert!(readings[1].path.ends_with("D3D Copy"));
        assert!(readings[2].path.ends_with("D3D Copy #2"));
    }

    /// Датчик знает своё железо, а подчинённый чип платы — свой вид, а не
    /// вид платы над ним.
    #[test]
    fn readings_know_their_hardware() {
        let flat = sample(TYPED);
        let [fan, load] = &flat.readings[..] else { panic!("два датчика") };

        let chip = &flat.hardware[fan.hardware];
        assert_eq!(chip.kind, HardwareKind::SuperIo);
        assert_eq!(chip.id, "/lpc/nct6799d/0");
        assert_eq!(chip.path, "MYPC/ASUS TUF GAMING B650-PLUS/Nuvoton NCT6799D");

        let gpu = &flat.hardware[load.hardware];
        assert_eq!(gpu.kind, HardwareKind::GpuNvidia);
        assert_eq!(gpu.name, "NVIDIA GeForce RTX 5070");
    }

    /// Старый сайдкар вида не шлёт — он выводится из `SensorId`, а датчики
    /// одного железа сходятся к одной записи.
    #[test]
    fn hardware_without_a_type_is_guessed_from_the_sensor_id() {
        let flat = sample(SAMPLE);
        assert_eq!(flat.hardware.len(), 2, "{:?}", flat.hardware);

        let cpu = &flat.hardware[flat.readings[0].hardware];
        assert_eq!(cpu.kind, HardwareKind::Cpu);
        assert_eq!(cpu.id, "/amdcpu/0");
        assert_eq!(cpu.name, "AMD Ryzen 7 7700X");

        assert_eq!(flat.readings[1].hardware, flat.readings[2].hardware);
        assert_eq!(flat.hardware[flat.readings[1].hardware].kind, HardwareKind::GpuNvidia);
    }

    /// Вид — только по первому сегменту: встройка Intel опознаётся и так,
    /// хотя её идентификатор длиннее.
    #[test]
    fn kinds_from_ids() {
        assert_eq!(HardwareKind::from_id("/intelcpu/0"), HardwareKind::Cpu);
        assert_eq!(HardwareKind::from_id("/gpu-amd/1"), HardwareKind::GpuAmd);
        assert_eq!(
            HardwareKind::from_id("/gpu-intel-integrated/pci_8086"),
            HardwareKind::GpuIntel
        );
        assert_eq!(HardwareKind::from_id("/lpc/it8689e/0"), HardwareKind::SuperIo);
        assert_eq!(HardwareKind::from_id("/nvme/0"), HardwareKind::Storage);
        assert_eq!(HardwareKind::from_id("/something-new/0"), HardwareKind::Other);
    }
}
