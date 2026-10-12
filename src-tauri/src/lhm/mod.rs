mod flatten;
mod parse;
pub mod roles;

pub use flatten::{flatten, Flat, HardwareKind, Reading};
pub use parse::Unit;

use serde::Deserialize;

/// Узел дерева из `data.json`.
///
/// Все значения LHM отдаёт строками, уже отформатированными по локали системы:
/// «62,4 °C», «88,3 KB/s», «-». Разбор этих строк — в [`parse`].
#[derive(Debug, Deserialize)]
pub struct Node {
    #[serde(rename = "Text", default)]
    pub text: String,
    #[serde(rename = "Value", default)]
    pub value: Option<String>,
    /// У пустого датчика `Value` равен «-», а здесь остаётся одна единица.
    #[serde(rename = "RawValue", default)]
    pub raw_value: Option<String>,
    /// Есть только у настоящих датчиков — по нему отличаем их от групп.
    #[serde(rename = "SensorId", default)]
    pub sensor_id: Option<String>,
    #[serde(rename = "Type", default)]
    pub kind: Option<String>,
    /// Вид железа — только у узлов железа и только у своего сайдкара: у
    /// веб-сервера LHM этого поля не было. Нет его — вид выводится из
    /// `SensorId` (см. [`flatten`]).
    #[serde(rename = "HardwareType", default)]
    pub hardware_type: Option<String>,
    /// Идентификатор железа, например `/gpu-nvidia/0`.
    #[serde(rename = "HardwareId", default)]
    pub hardware_id: Option<String>,
    #[serde(rename = "Children", default)]
    pub children: Vec<Node>,
}
