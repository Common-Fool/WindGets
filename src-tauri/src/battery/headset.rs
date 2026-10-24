//! Гарнитуры через докачиваемый HeadsetControl.

use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::Value;

use super::{Device, Kind, Source};

static WARNED: AtomicBool = AtomicBool::new(false);

fn warn(message: &str) {
    if !WARNED.swap(true, Ordering::Relaxed) {
        eprintln!("[заряд] HeadsetControl: {message}");
    }
}

pub fn devices(exe: &Path) -> Vec<Device> {
    let mut command = Command::new(exe);
    command.args(["-b", "-o", "json"]);
    let (_, lines) = match crate::child::run(command, Duration::from_secs(10)) {
        Ok(output) => output,
        Err(error) => {
            warn(&format!("не удалось прочитать заряд: {error}"));
            return Vec::new();
        }
    };
    // stderr может содержать диагностику даже при правильном JSON в stdout.
    // Код выхода не проверяем: отсутствие гарнитур бывает штатным отказом.
    let text = lines.iter()
        .filter(|line| matches!(line, crate::child::Line::Out(_)))
        .map(crate::child::Line::text)
        .collect::<Vec<_>>()
        .join("\n");
    if serde_json::from_str::<Value>(&text).is_err() {
        warn("не удалось разобрать JSON");
        return Vec::new();
    }
    parse(&text)
}

/// Чистый разбор: неизвестный статус или неполная запись не дают строки.
fn parse(text: &str) -> Vec<Device> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let Some(devices) = value.get("devices").and_then(Value::as_array) else {
        return Vec::new();
    };
    devices.iter().filter_map(|device| {
        if device.get("status")?.as_str()? != "success" {
            return None;
        }
        let battery = device.get("battery")?;
        let charging = match battery.get("status")?.as_str()? {
            "BATTERY_AVAILABLE" => false,
            "BATTERY_CHARGING" => true,
            _ => return None,
        };
        let level = battery.get("level")?.as_u64()?;
        if level > 100 {
            return None;
        }
        let vid = usb_id(device.get("id_vendor")?.as_str()?)?;
        let pid = usb_id(device.get("id_product")?.as_str()?)?;
        let name = device.get("product").and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .or_else(|| device.get("device").and_then(Value::as_str))?
            .trim();
        if name.is_empty() {
            return None;
        }
        Some(Device {
            id: format!("hs:{vid:04x}:{pid:04x}"),
            name: name.to_owned(),
            kind: Kind::Headset,
            level: level as u8,
            charging: Some(charging),
            source: Source::Headset,
            usb: Some((vid, pid)),
        })
    }).collect()
}

fn usb_id(text: &str) -> Option<u16> {
    let digits = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X"))?;
    if digits.is_empty() || digits.len() > 4 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    u16::from_str_radix(digits, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample(status: &str, level: Value) -> Value {
        json!({
            "status": "success", "device": "Wireless Headset", "product": "Headset",
            "id_vendor": "0x1234", "id_product": "0xabcd",
            "battery": { "status": status, "level": level }
        })
    }

    fn document(device: Value) -> String {
        json!({"name": "HeadsetControl", "device_count": 1, "devices": [device]}).to_string()
    }

    #[test]
    fn available_battery_uses_product_and_usb_identity() {
        let devices = parse(&document(sample("BATTERY_AVAILABLE", json!(80))));
        assert_eq!(devices, vec![Device {
            id: "hs:1234:abcd".to_owned(), name: "Headset".to_owned(),
            kind: Kind::Headset, level: 80, charging: Some(false),
            source: Source::Headset, usb: Some((0x1234, 0xabcd)),
        }]);
    }

    #[test]
    fn charging_battery_and_empty_product_use_model_name() {
        let mut device = sample("BATTERY_CHARGING", json!(100));
        device["product"] = json!("  ");
        let devices = parse(&document(device));
        assert_eq!(devices[0].name, "Wireless Headset");
        assert_eq!(devices[0].charging, Some(true));
        assert_eq!(devices[0].level, 100);
    }

    #[test]
    fn unavailable_error_timeout_and_unknown_status_are_absent() {
        for status in ["BATTERY_UNAVAILABLE", "BATTERY_ERROR", "BATTERY_TIMEOUT", "unknown"] {
            assert!(parse(&document(sample(status, json!(80)))).is_empty(), "{status}");
        }
        let mut device = sample("BATTERY_AVAILABLE", json!(80));
        device["status"] = json!("error");
        assert!(parse(&document(device)).is_empty());
    }

    #[test]
    fn invalid_levels_and_missing_fields_are_absent() {
        for level in [json!(-1), json!(101), json!(256), json!(50.5), json!("80"), Value::Null] {
            assert!(parse(&document(sample("BATTERY_AVAILABLE", level))).is_empty());
        }
        assert_eq!(parse(&document(sample("BATTERY_AVAILABLE", json!(0))))[0].level, 0);
        let mut device = sample("BATTERY_AVAILABLE", json!(80));
        device["id_vendor"] = json!("0x10000");
        assert!(parse(&document(device)).is_empty());
        assert!(parse(&document(json!({}))).is_empty());
    }

    #[test]
    fn garbage_and_empty_lists_are_absent() {
        for text in ["garbage", "{", "null", "[]", "{}", r#"{"devices":[]}"#] {
            assert!(parse(text).is_empty(), "{text}");
        }
    }

    #[test]
    fn two_devices_are_independent() {
        let first = sample("BATTERY_AVAILABLE", json!(80));
        let mut second = sample("BATTERY_CHARGING", json!(25));
        second["id_product"] = json!("0xABCE");
        let devices = parse(&json!({"devices": [first, second]}).to_string());
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[1].id, "hs:1234:abce");
        assert_eq!(devices[1].level, 25);
        assert_eq!(devices[1].charging, Some(true));
    }
}
