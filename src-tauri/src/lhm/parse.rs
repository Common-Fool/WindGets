use serde::Serialize;

/// Единица измерения, приведённая к базовой.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Unit {
    Celsius,
    Percent,
    Watt,
    Megahertz,
    Volt,
    Rpm,
    Byte,
    BytePerSecond,
    Nanosecond,
    /// Безразмерная величина (например, множители в разделе Factors).
    Raw,
}

/// Разобранное значение датчика.
/// `value == None` означает «датчик есть, значения нет» — нулём это не подменяем.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measurement {
    pub value: Option<f64>,
    pub unit: Unit,
}

const KIB: f64 = 1024.0;
const MIB: f64 = 1024.0 * KIB;
const GIB: f64 = 1024.0 * MIB;
const TIB: f64 = 1024.0 * GIB;

/// Суффикс строки → единица и множитель до базовой.
///
/// Порядок важен: «MB/s» обязан проверяться раньше «MB», а «MB» раньше «B»,
/// иначе скорость записи будет разобрана как объём. Единица в выхлопе LHM
/// зависит от величины: та же запись отдаётся то в KB/s, то в MB/s, поэтому
/// без приведения к байтам график скачет в тысячу раз на ровном месте.
const SUFFIXES: &[(&str, Unit, f64)] = &[
    ("°C", Unit::Celsius, 1.0),
    ("%", Unit::Percent, 1.0),
    ("RPM", Unit::Rpm, 1.0),
    ("ns", Unit::Nanosecond, 1.0),
    ("TB/s", Unit::BytePerSecond, TIB),
    ("GB/s", Unit::BytePerSecond, GIB),
    ("MB/s", Unit::BytePerSecond, MIB),
    ("KB/s", Unit::BytePerSecond, KIB),
    ("B/s", Unit::BytePerSecond, 1.0),
    ("TB", Unit::Byte, TIB),
    ("GB", Unit::Byte, GIB),
    ("MB", Unit::Byte, MIB),
    ("KB", Unit::Byte, KIB),
    ("B", Unit::Byte, 1.0),
    ("GHz", Unit::Megahertz, 1000.0),
    ("MHz", Unit::Megahertz, 1.0),
    ("W", Unit::Watt, 1.0),
    ("V", Unit::Volt, 1.0),
];

fn match_suffix(text: &str) -> Option<(Unit, f64)> {
    SUFFIXES
        .iter()
        .find(|(suffix, _, _)| text.ends_with(suffix))
        .map(|(_, unit, scale)| (*unit, *scale))
}

/// Приводит десятичный разделитель к точке.
///
/// LHM форматирует по локали системы: на русской выходит «62,4», на английской
/// «62.4». Разделителей разрядов он не ставит (формат вида F1/F3), поэтому
/// одиночная запятая всегда означает дробную часть. Случай с обоими знаками
/// разбираем на всякий случай: правый считаем дробным, левый — разрядным.
fn normalize_separators(digits: &str) -> String {
    match (digits.rfind(','), digits.rfind('.')) {
        (Some(comma), Some(dot)) => {
            let decimal = comma.max(dot);
            digits
                .char_indices()
                .filter_map(|(at, ch)| {
                    if at == decimal {
                        Some('.')
                    } else if ch == ',' || ch == '.' {
                        None
                    } else {
                        Some(ch)
                    }
                })
                .collect()
        }
        _ => digits.replace(',', "."),
    }
}

fn parse_number(text: &str) -> Option<f64> {
    let digits: String = text
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || matches!(ch, '-' | '+' | ',' | '.'))
        .collect();

    if digits.is_empty() {
        return None;
    }

    // Пропуск значения LHM обозначает прочерком — из take_while он выходит
    // как «-» и до числа не дотягивает
    normalize_separators(&digits).parse::<f64>().ok()
}

/// Разбирает пару полей узла LHM в значение с нормализованной единицей.
///
/// Единицу ищем сначала в `Value`, затем в `RawValue`: у пустого датчика
/// `Value` равен «-», зато `RawValue` сохраняет единицу (например « B/s»),
/// и благодаря ей виджет знает, что показывать при появлении данных.
pub fn parse(value: Option<&str>, raw_value: Option<&str>) -> Measurement {
    let (unit, scale) = [value, raw_value]
        .into_iter()
        .flatten()
        .find_map(|text| match_suffix(text.trim()))
        .unwrap_or((Unit::Raw, 1.0));

    let number = value.and_then(|text| parse_number(text.trim()));

    Measurement {
        value: number.map(|number| number * scale),
        unit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measure(value: &str, raw: &str) -> Measurement {
        parse(Some(value), Some(raw))
    }

    #[test]
    fn reads_temperature_with_comma() {
        let result = measure("62,4 °C", "62,4 °C");
        assert_eq!(result.unit, Unit::Celsius);
        assert_eq!(result.value, Some(62.4));
    }

    #[test]
    fn reads_temperature_with_dot() {
        // На английской локали LHM отдаёт точку — парсер обязан принять оба вида
        assert_eq!(measure("62.4 °C", "62.4 °C").value, Some(62.4));
    }

    #[test]
    fn normalizes_speed_units_to_bytes() {
        let kilobytes = measure("88,3 KB/s", "88,3 KB/s");
        let megabytes = measure("38,7 MB/s", "38,7 MB/s");
        assert_eq!(kilobytes.unit, Unit::BytePerSecond);
        assert_eq!(megabytes.unit, Unit::BytePerSecond);
        assert_eq!(kilobytes.value, Some(88.3 * KIB));
        assert_eq!(megabytes.value, Some(38.7 * MIB));
        // Мегабайты в секунду обязаны оказаться больше килобайт, а не наоборот
        assert!(megabytes.value > kilobytes.value);
    }

    #[test]
    fn keeps_unit_when_value_is_missing() {
        // Так выглядит датчик скорости у некоторых NVMe: значения нет, единица есть
        let result = measure("-", " B/s");
        assert_eq!(result.value, None);
        assert_eq!(result.unit, Unit::BytePerSecond);
    }

    #[test]
    fn reads_video_memory_as_bytes() {
        let result = measure("2326,0 MB", "2326,0 MB");
        assert_eq!(result.unit, Unit::Byte);
        assert_eq!(result.value, Some(2326.0 * MIB));
    }

    #[test]
    fn reads_dimensionless_factor() {
        let result = measure("50,500", "50,500");
        assert_eq!(result.unit, Unit::Raw);
        assert_eq!(result.value, Some(50.5));
    }

    #[test]
    fn reads_remaining_hardware_units() {
        assert_eq!(measure("1084 RPM", "1084 RPM").unit, Unit::Rpm);
        assert_eq!(measure("5040,0 MHz", "5040,0 MHz").value, Some(5040.0));
        assert_eq!(measure("12,076 V", "12,076 V").unit, Unit::Volt);
        assert_eq!(measure("39,2 W", "39,2 W").value, Some(39.2));
        assert_eq!(measure("295,000 ns", "295,000 ns").unit, Unit::Nanosecond);
    }

    #[test]
    fn survives_grouping_separator() {
        // Подстраховка на случай локали с разделителем разрядов
        assert_eq!(measure("1,234.5 MB", "1,234.5 MB").value, Some(1234.5 * MIB));
    }

    #[test]
    fn percent_stays_as_is() {
        let result = measure("12,6 %", "12,6 %");
        assert_eq!(result.unit, Unit::Percent);
        assert_eq!(result.value, Some(12.6));
    }
}
