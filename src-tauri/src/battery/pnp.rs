//! Заряд Bluetooth из свойств узлов Windows.

use std::mem::size_of;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::{GUID, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW,
    SetupDiGetDeviceInstanceIdW, SetupDiGetDevicePropertyW, DIGCF_ALLCLASSES,
    DIGCF_PRESENT, HDEVINFO, SP_DEVINFO_DATA,
};
use windows::Win32::Devices::Properties::{
    DEVPKEY_Device_FriendlyName, DEVPROPTYPE, DEVPROP_TYPE_BYTE,
    DEVPROP_TYPE_STRING, DEVPROP_TYPE_UINT16, DEVPROP_TYPE_UINT32,
};
use windows::Win32::Foundation::{
    DEVPROPKEY, ERROR_INSUFFICIENT_BUFFER, ERROR_NOT_FOUND, ERROR_NO_MORE_ITEMS,
};

use super::{Device, Kind, Source};

const BATTERY_LEVEL: DEVPROPKEY = DEVPROPKEY {
    fmtid: GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5), pid: 2,
};
const BLUETOOTH_FLAGS: DEVPROPKEY = DEVPROPKEY {
    fmtid: GUID::from_u128(0x2bd67d8b_8beb_48d5_87e0_6cda3428040a), pid: 3,
};
const CLASS_OF_DEVICE: DEVPROPKEY = DEVPROPKEY {
    fmtid: GUID::from_u128(0x2bd67d8b_8beb_48d5_87e0_6cda3428040a), pid: 10,
};
// PKEY_Devices_Aep_Bluetooth_Le_Appearance (UInt16). Не каждый стек
// публикует его на PnP-узле; отсутствие означает неизвестный вид.
const APPEARANCE: DEVPROPKEY = DEVPROPKEY {
    fmtid: GUID::from_u128(0x995ef0b0_7eb3_4a8b_b9ce_068bb3f4af69), pid: 1,
};

// BDIF_CONNECTED и BDIF_LE_CONNECTED из bthdef.h. DN_STARTED говорит о
// работе драйвера спаренного устройства и не доказывает радиосоединение.
const BDIF_CONNECTED: u32 = 0x0000_0020;
const BDIF_LE_CONNECTED: u32 = 0x0100_0000;
static WARNED: AtomicBool = AtomicBool::new(false);

struct DeviceSet(HDEVINFO);

impl Drop for DeviceSet {
    fn drop(&mut self) {
        unsafe { let _ = SetupDiDestroyDeviceInfoList(self.0); }
    }
}

struct Property {
    kind: DEVPROPTYPE,
    bytes: Vec<u8>,
}

impl Property {
    fn number(&self) -> Option<u32> {
        match (self.kind, self.bytes.as_slice()) {
            (DEVPROP_TYPE_BYTE, [value]) => Some(u32::from(*value)),
            (DEVPROP_TYPE_UINT16, [a, b]) => Some(u32::from(u16::from_le_bytes([*a, *b]))),
            (DEVPROP_TYPE_UINT32, [a, b, c, d]) => Some(u32::from_le_bytes([*a, *b, *c, *d])),
            _ => None,
        }
    }

    fn text(&self) -> Option<String> {
        if self.kind != DEVPROP_TYPE_STRING || !self.bytes.len().is_multiple_of(2) {
            return None;
        }
        let wide: Vec<u16> = self.bytes.chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .take_while(|unit| *unit != 0)
            .collect();
        let text = String::from_utf16(&wide).ok()?;
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_owned())
    }
}

struct Node {
    address: String,
    root: bool,
    le: bool,
    name: Option<String>,
    level: Option<u8>,
    flags: Option<u32>,
    class: Option<u32>,
    appearance: Option<u16>,
}

pub fn devices() -> Vec<Device> {
    match read_nodes() {
        Ok(nodes) => collect(&nodes),
        Err(error) => {
            if !WARNED.swap(true, Ordering::Relaxed) {
                eprintln!("[заряд] не удалось прочитать устройства Bluetooth: {error}");
            }
            Vec::new()
        }
    }
}

fn read_nodes() -> windows::core::Result<Vec<Node>> {
    let mut nodes = Vec::new();
    for enumerator in ["BTHENUM", "BTHLE", "BTHLEDEVICE", "BTHHFENUM"] {
        let wide: Vec<u16> = enumerator.encode_utf16().chain(Some(0)).collect();
        let set = DeviceSet(unsafe {
            SetupDiGetClassDevsW(None, PCWSTR(wide.as_ptr()), None, DIGCF_ALLCLASSES | DIGCF_PRESENT)?
        });
        for index in 0..u32::MAX {
            let mut info = SP_DEVINFO_DATA { cbSize: size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
            if let Err(error) = unsafe { SetupDiEnumDeviceInfo(set.0, index, &mut info) } {
                if error.code() == ERROR_NO_MORE_ITEMS.to_hresult() {
                    break;
                }
                return Err(error);
            }
            let instance = instance_id(&set, &info)?;
            let Some(address) = bluetooth_address(&instance) else { continue };
            let root = is_device_node(&instance);
            let le = matches!(enumerator, "BTHLE" | "BTHLEDEVICE");
            let level = property(&set, &info, &BATTERY_LEVEL)?
                .filter(|value| value.kind == DEVPROP_TYPE_BYTE)
                .and_then(|value| value.number())
                .filter(|value| *value <= 100)
                .map(|value| value as u8);
            let name = property(&set, &info, &DEVPKEY_Device_FriendlyName)?
                .and_then(|value| value.text());
            let flags = property(&set, &info, &BLUETOOTH_FLAGS)?
                .filter(|value| value.kind == DEVPROP_TYPE_UINT32)
                .and_then(|value| value.number());
            let class = property(&set, &info, &CLASS_OF_DEVICE)?
                .filter(|value| value.kind == DEVPROP_TYPE_UINT32)
                .and_then(|value| value.number());
            let appearance = property(&set, &info, &APPEARANCE)?
                .filter(|value| value.kind == DEVPROP_TYPE_UINT16)
                .and_then(|value| value.number())
                .map(|value| value as u16);
            nodes.push(Node { address, root, le, name, level, flags, class, appearance });
        }
    }
    Ok(nodes)
}

fn instance_id(set: &DeviceSet, info: &SP_DEVINFO_DATA) -> windows::core::Result<String> {
    let mut needed = 0;
    let first = unsafe { SetupDiGetDeviceInstanceIdW(set.0, info, None, Some(&mut needed)) };
    if let Err(error) = first {
        if error.code() != ERROR_INSUFFICIENT_BUFFER.to_hresult() {
            return Err(error);
        }
    }
    if needed == 0 || needed > 65_536 {
        return Err(windows::core::Error::new(windows::core::HRESULT(0x8007000Du32 as i32), "неверная длина id устройства"));
    }
    let mut wide = vec![0u16; needed as usize];
    unsafe { SetupDiGetDeviceInstanceIdW(set.0, info, Some(&mut wide), Some(&mut needed))?; }
    let end = wide.iter().position(|unit| *unit == 0).unwrap_or(wide.len());
    Ok(String::from_utf16_lossy(&wide[..end]))
}

fn property(set: &DeviceSet, info: &SP_DEVINFO_DATA, key: &DEVPROPKEY) -> windows::core::Result<Option<Property>> {
    let mut kind = DEVPROPTYPE::default();
    let mut needed = 0;
    let first = unsafe { SetupDiGetDevicePropertyW(set.0, info, key, &mut kind, None, Some(&mut needed), 0) };
    if let Err(error) = first {
        if error.code() == ERROR_NOT_FOUND.to_hresult() {
            return Ok(None);
        }
        if error.code() != ERROR_INSUFFICIENT_BUFFER.to_hresult() {
            return Err(error);
        }
    }
    if needed == 0 || needed > 65_536 {
        return Ok(None);
    }
    let mut bytes = vec![0u8; needed as usize];
    if let Err(error) = unsafe {
        SetupDiGetDevicePropertyW(set.0, info, key, &mut kind, Some(&mut bytes), Some(&mut needed), 0)
    } {
        if error.code() == ERROR_NOT_FOUND.to_hresult() {
            return Ok(None);
        }
        return Err(error);
    }
    bytes.truncate(needed as usize);
    Ok(Some(Property { kind, bytes }))
}

/// Адрес ищем в сегменте DEV_ и отдельных токенах экземпляра службы:
/// GUID службы и VID/PID не должны случайно стать адресом устройства.
fn bluetooth_address(instance: &str) -> Option<String> {
    let upper = instance.to_ascii_uppercase();
    let mut parts = upper.split('\\');
    let enumerator = parts.next()?;
    if !matches!(enumerator, "BTHENUM" | "BTHLE" | "BTHLEDEVICE" | "BTHHFENUM") {
        return None;
    }
    let device = parts.next()?;
    let tail = parts.next()?;
    if let Some(address) = device.strip_prefix("DEV_") {
        if address.len() == 12 && address.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Some(address.to_owned());
        }
        return None;
    }
    for section in [device, tail] {
        for (index, _) in section.match_indices(['_', '&']) {
            let candidate = &section[index + 1..];
            let end = candidate.find(['_', '&']).unwrap_or(candidate.len());
            let address = &candidate[..end];
            if address.len() == 12 && address.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Some(address.to_owned());
            }
        }
    }
    None
}

fn is_device_node(instance: &str) -> bool {
    let upper = instance.to_ascii_uppercase();
    upper.starts_with("BTHENUM\\DEV_") || upper.starts_with("BTHLE\\DEV_")
}

fn connected(flags: Option<u32>, le: bool) -> bool {
    flags.is_some_and(|flags| flags & if le { BDIF_LE_CONNECTED } else { BDIF_CONNECTED } != 0)
}

fn class_kind(class: u32) -> Kind {
    match (class >> 8) & 0x1f {
        0x02 => Kind::Phone,
        0x04 => Kind::Headset,
        0x05 => {
            let minor = class & 0xfc;
            if minor & 0x40 != 0 {
                Kind::Keyboard
            } else if minor & 0x80 != 0 {
                Kind::Mouse
            } else if matches!(minor & 0x3c, 0x04 | 0x08) {
                Kind::Gamepad
            } else {
                Kind::Other
            }
        }
        _ => Kind::Other,
    }
}

fn appearance_kind(appearance: u16) -> Kind {
    match appearance {
        0x0040..=0x007f => Kind::Phone,
        0x03c1 => Kind::Keyboard,
        0x03c2 => Kind::Mouse,
        0x03c3 | 0x03c4 => Kind::Gamepad,
        // Категория 0x25 — носимое аудио; 0x0941 — наушники, не телефон.
        0x0940..=0x097f => Kind::Headset,
        _ => Kind::Other,
    }
}

fn node_kind(node: &Node) -> Kind {
    let appearance = node.appearance.map(appearance_kind).unwrap_or(Kind::Other);
    if appearance != Kind::Other {
        appearance
    } else {
        node.class.map(class_kind).unwrap_or(Kind::Other)
    }
}

fn collect(nodes: &[Node]) -> Vec<Device> {
    nodes.iter().filter_map(|node| {
        let level = node.level?;
        // Проверяем корневой узел именно этого транспорта: подключение LE
        // не оживляет старый заряд службы HFP того же адреса и наоборот.
        let root = nodes.iter().find(|candidate| candidate.root && candidate.le == node.le
            && candidate.address == node.address && connected(candidate.flags, candidate.le))?;
        let name = root.name.as_ref()
            .or_else(|| nodes.iter().filter(|candidate| candidate.root && candidate.address == node.address)
                .find_map(|candidate| candidate.name.as_ref()))
            .or(node.name.as_ref())?;
        let kind = [root, node].into_iter().map(node_kind)
            .find(|kind| *kind != Kind::Other).unwrap_or(Kind::Other);
        Some(Device {
            id: format!("bt:{}", node.address), name: name.clone(), kind, level,
            charging: None, source: Source::Windows, usb: None,
        })
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_from_device_and_hfp_service_are_uppercase() {
        for instance in [
            r"BTHLE\Dev_aabbccddeeff\1&2&0&AABBCCDDEEFF",
            r"BTHENUM\DEV_AABBCCDDEEFF\1&2&0&BLUETOOTHDEVICE_AABBCCDDEEFF",
            r"BTHENUM\{0000111F-0000-1000-8000-00805F9B34FB}_VID&00011234_PID&5678\1&2&0&AABBCCDDEEFF_C00000000",
            r"BTHHFENUM\SERVICE\1_AABBCCDDEEFF_C00000000",
            r"BTHLEDEVICE\SERVICE\1_AABBCCDDEEFF&0&0000",
            r"BTHLEDEVICE\{0000180F-0000-1000-8000-00805F9B34FB}_DEV_VID&021234_PID&5678_REV&0001_AABBCCDDEEFF\1&2&0&0001",
        ] {
            assert_eq!(bluetooth_address(instance).as_deref(), Some("AABBCCDDEEFF"), "{instance}");
        }
    }

    #[test]
    fn malformed_or_unrelated_ids_have_no_address() {
        for instance in [
            "", r"USB\DEV_AABBCCDDEEFF\0", r"BTHLE\DEV_AABBCCDDEEF\0",
            r"BTHLE\DEV_AABBCCDDEEFF0\0", r"BTHLE\DEV_GABBCCDDEEFF\0",
            r"BTHENUM\{0000111F-0000-1000-8000-00805F9B34FB}\1&2&0&0000000000000_C00000000",
        ] {
            assert!(bluetooth_address(instance).is_none(), "{instance}");
        }
    }

    #[test]
    fn class_and_appearance_classify_without_model_names() {
        for (class, kind) in [
            (0x5a020c, Kind::Phone), (0x240404, Kind::Headset),
            (0x0540, Kind::Keyboard), (0x0580, Kind::Mouse),
            (0x05c0, Kind::Keyboard), (0x0504, Kind::Gamepad), (0x0508, Kind::Gamepad),
            (0x0510, Kind::Other), (0, Kind::Other), (0x0100, Kind::Other),
        ] {
            assert_eq!(class_kind(class), kind, "{class:x}");
        }
        for (appearance, kind) in [
            (0x0040, Kind::Phone), (0x007f, Kind::Phone),
            (0x03c1, Kind::Keyboard), (0x03c2, Kind::Mouse),
            (0x03c3, Kind::Gamepad), (0x03c4, Kind::Gamepad),
            (0x0941, Kind::Headset), (0x0942, Kind::Headset),
            (0x03c5, Kind::Other), (0, Kind::Other), (0xffff, Kind::Other),
        ] {
            assert_eq!(appearance_kind(appearance), kind, "{appearance:x}");
        }
    }

    fn node(root: bool, flags: Option<u32>, level: Option<u8>) -> Node {
        Node {
            address: "AABBCCDDEEFF".to_owned(), root, le: false,
            name: Some(if root { "Phone" } else { "Phone HF" }.to_owned()),
            level, flags, class: Some(0x020c), appearance: None,
        }
    }

    #[test]
    fn cached_battery_requires_a_connected_root_of_the_same_transport() {
        for flags in [None, Some(0), Some(0x08), Some(BDIF_LE_CONNECTED)] {
            assert!(collect(&[node(true, flags, None), node(false, Some(BDIF_CONNECTED), Some(80))]).is_empty());
        }
        assert!(collect(&[node(false, Some(BDIF_CONNECTED), Some(80))]).is_empty());
        let devices = collect(&[node(true, Some(BDIF_CONNECTED), None), node(false, None, Some(80))]);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "Phone");
        assert_eq!(devices[0].kind, Kind::Phone);
        assert_eq!(devices[0].id, "bt:AABBCCDDEEFF");
        assert_eq!(devices[0].charging, None);
        assert_eq!(devices[0].source, Source::Windows);
        assert_eq!(devices[0].usb, None);
        let mut service = node(false, None, Some(80));
        service.le = true;
        assert!(collect(&[node(true, Some(BDIF_CONNECTED), None), service]).is_empty());
    }

    #[test]
    fn le_connection_uses_its_own_bit_and_can_fall_back_to_battery_node_name() {
        assert!(!connected(None, true));
        assert!(!connected(Some(BDIF_CONNECTED), true));
        assert!(connected(Some(BDIF_LE_CONNECTED), true));
        let mut root = node(true, Some(BDIF_LE_CONNECTED), None);
        root.le = true;
        root.name = None;
        root.class = None;
        root.appearance = Some(0x03c4);
        let mut battery = node(false, None, Some(100));
        battery.le = true;
        battery.name = Some("Wireless Controller".to_owned());
        let devices = collect(&[root, battery]);
        assert_eq!(devices[0].name, "Wireless Controller");
        assert_eq!(devices[0].kind, Kind::Gamepad);
    }

    #[test]
    fn property_types_and_lengths_are_checked() {
        assert_eq!(Property { kind: DEVPROP_TYPE_BYTE, bytes: vec![100] }.number(), Some(100));
        assert_eq!(Property { kind: DEVPROP_TYPE_UINT16, bytes: vec![0xc4, 3] }.number(), Some(0x03c4));
        assert_eq!(Property { kind: DEVPROP_TYPE_UINT32, bytes: vec![0x0c, 2, 0x5a, 0] }.number(), Some(0x5a020c));
        assert_eq!(Property { kind: DEVPROP_TYPE_BYTE, bytes: vec![0, 1] }.number(), None);
        assert_eq!(Property { kind: DEVPROP_TYPE_STRING, bytes: vec![80, 0, 0, 0] }.text().as_deref(), Some("P"));
        assert_eq!(Property { kind: DEVPROP_TYPE_STRING, bytes: vec![80] }.text(), None);
    }

    #[test]
    #[ignore = "живое железо"]
    fn live_devices() {
        println!("{:#?}", devices());
    }
}
