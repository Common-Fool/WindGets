//! Logitech HID++: заряд мышей, клавиатур и гарнитур на LIGHTSPEED, Unifying
//! и проводе.
//!
//! Протокол один на всю линейку Logitech, поэтому свой код тут окупается, в
//! отличие от протоколов прочих брендов. Говорим через вендорские коллекции
//! HID (страница 0xFF00): короткие отчёты 0x10 по 7 байт и длинные 0x11 по 20.
//! Запросы шлём длинными, а ответ ждём с обеих коллекций: приёмник отвечает
//! на вопрос о пустом или спящем слоте короткой ошибкой HID++ 1.0.
//!
//! ВАЖНО: **мы только спрашиваем.** Здесь нет ни одной команды, которая
//! меняет что-то в устройстве: версия протокола, номер функции, имя, вид и
//! заряд. Испортить мышь этим нечем.
//!
//! ВАЖНО: **ответы приходят всем, кто открыл коллекцию** — G HUB тоже. Свой
//! ответ узнаём по индексу устройства, номеру функции и своему software id в
//! младшем полубайте четвёртого байта; чужие пропускаем.

use std::time::{Duration, Instant};

use windows::core::PCWSTR;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
    SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, HDEVINFO,
    SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
};
use windows::Win32::Devices::HumanInterfaceDevice::{
    HidD_FreePreparsedData, HidD_GetAttributes, HidD_GetHidGuid, HidD_GetPreparsedData, HidP_GetCaps,
    HIDD_ATTRIBUTES, HIDP_CAPS, PHIDP_PREPARSED_DATA,
};
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForMultipleObjects, WaitForSingleObject};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

use super::{Device, Kind, Source};

const LOGITECH: u16 = 0x046d;
const VENDOR_PAGE: u16 = 0xff00;
const SHORT_USAGE: u16 = 0x0001;
const LONG_USAGE: u16 = 0x0002;

const SHORT: u8 = 0x10;
const LONG: u8 = 0x11;
const SHORT_LEN: usize = 7;
const LONG_LEN: usize = 20;

/// Свой номер отправителя. Ноль зарезервирован под уведомления устройства.
const SOFTWARE_ID: u8 = 0x0b;

/// Сколько ждём ответа. Живое устройство отвечает за миллисекунды, приёмник
/// за спящий слот — тоже; дольше ждать значит держать поток минуту.
const WAIT: Duration = Duration::from_millis(400);

/// Индекс самого устройства при проводе; у приёмника — сам приёмник.
const DIRECT: u8 = 0xff;
/// Слоты приёмника: у Unifying их шесть, у LIGHTSPEED меньше.
const SLOTS: std::ops::RangeInclusive<u8> = 1..=6;

const ROOT: u8 = 0x00;
const FEATURE_NAME: u16 = 0x0005;
const FEATURE_UNIFIED_BATTERY: u16 = 0x1004;
const FEATURE_BATTERY_STATUS: u16 = 0x1000;
const FEATURE_BATTERY_VOLTAGE: u16 = 0x1001;

pub fn devices() -> Vec<Device> {
    let mut found = Vec::new();
    for receiver in receivers() {
        let Some(channel) = Channel::open(&receiver) else { continue };
        for index in std::iter::once(DIRECT).chain(SLOTS) {
            if let Some(device) = ask_device(&channel, index, receiver.product, &receiver.key) {
                found.push(device);
            }
        }
    }
    found
}

/// Вендорские коллекции одного USB-устройства Logitech.
struct Interfaces {
    product: u16,
    /// `parent_key` донгла: различает два одинаковых приёмника.
    key: String,
    short: Option<Vec<u16>>,
    long: Option<Vec<u16>>,
}

/// Пути коллекций HID++ всех подключённых устройств Logitech.
fn receivers() -> Vec<Interfaces> {
    let mut found: Vec<Interfaces> = Vec::new();
    for path in hid_paths() {
        // Путь коллекции несёт VID и PID, и по нему же дешевле всего отсеять
        // всё не-Logitech, не открывая ни одного устройства
        let lower = String::from_utf16_lossy(&path).to_lowercase();
        if !lower.contains("vid_046d") {
            continue;
        }
        let Some((vendor, product, page, usage)) = describe(&path) else { continue };
        // Прочие коллекции вендорской страницы нам не нужны, и пустой записи
        // без единого пути в списке быть не должно
        if vendor != LOGITECH || page != VENDOR_PAGE || !matches!(usage, SHORT_USAGE | LONG_USAGE) {
            continue;
        }
        // Коллекции одного устройства отличаются хвостом `&col0N`; ключ — путь
        // без него, чтобы короткая и длинная легли в одну запись
        let key = parent_key(&lower);
        let entry = match found.iter().position(|known| known.key == key) {
            Some(at) => &mut found[at],
            None => {
                found.push(Interfaces { product, key: key.clone(), short: None, long: None });
                found.last_mut().expect("только что положили")
            }
        };
        match usage {
            SHORT_USAGE => entry.short = Some(path),
            LONG_USAGE => entry.long = Some(path),
            _ => {}
        }
    }
    found.retain(|interfaces| interfaces.long.is_some());
    found
}

/// Ключ устройства для `hidden_batteries`: модель донгла, сам донгл и слот.
/// Донгл — короткий отпечаток его `parent_key`: два одинаковых приёмника иначе
/// дали бы один ключ, и `merge` выбросил бы второе устройство.
fn device_id(product: u16, receiver: &str, index: u8) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(receiver.as_bytes());
    format!("lg:{product:04x}:{:02x}{:02x}{:02x}{:02x}:{index:02x}", digest[0], digest[1], digest[2], digest[3])
}

/// Ключ устройства по пути коллекции: `vid…&pid…&mi…` без `&colNN` и
/// экземпляр без последнего звена. У короткой и длинной коллекций одного
/// донгла они совпадают, у двух одинаковых донглов — нет.
fn parent_key(path: &str) -> String {
    let parts: Vec<&str> = path.split('#').collect();
    let interface = parts.get(1).map_or("", |part| part.split("&col").next().unwrap_or(part));
    let instance = parts.get(2).map_or("", |part| part.rsplit_once('&').map_or(*part, |(head, _)| head));
    format!("{interface}#{instance}")
}

struct DeviceList(HDEVINFO);

impl Drop for DeviceList {
    fn drop(&mut self) {
        let _ = unsafe { SetupDiDestroyDeviceInfoList(self.0) };
    }
}

/// Пути всех интерфейсов HID, с завершающим нулём.
fn hid_paths() -> Vec<Vec<u16>> {
    let guid = unsafe { HidD_GetHidGuid() };
    let Ok(list) = (unsafe { SetupDiGetClassDevsW(Some(&guid), PCWSTR::null(), None, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE) }) else {
        return Vec::new();
    };
    let list = DeviceList(list);

    let mut paths = Vec::new();
    for index in 0.. {
        let mut data = SP_DEVICE_INTERFACE_DATA { cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32, ..Default::default() };
        if unsafe { SetupDiEnumDeviceInterfaces(list.0, None, &guid, index, &mut data) }.is_err() {
            break;
        }
        let mut required = 0u32;
        let _ = unsafe { SetupDiGetDeviceInterfaceDetailW(list.0, &data, None, 0, Some(&mut required), None) };
        if required < size_of::<u32>() as u32 {
            continue;
        }
        // Буфер под структуру с хвостом переменной длины, выровненный по u32
        let mut buffer = vec![0u32; (required as usize).div_ceil(4)];
        let detail = buffer.as_mut_ptr().cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
        unsafe { (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32 };
        if unsafe { SetupDiGetDeviceInterfaceDetailW(list.0, &data, Some(detail), required, None, None) }.is_err() {
            continue;
        }
        let start = unsafe { std::ptr::addr_of!((*detail).DevicePath) }.cast::<u16>();
        let room = (required as usize - std::mem::offset_of!(SP_DEVICE_INTERFACE_DETAIL_DATA_W, DevicePath)) / 2;
        let wide = unsafe { std::slice::from_raw_parts(start, room) };
        let length = wide.iter().position(|unit| *unit == 0).unwrap_or(room);
        let mut path = wide[..length].to_vec();
        path.push(0);
        paths.push(path);
    }
    paths
}

/// Открытый хендл; закрывается сам.
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn open(path: &[u16], access: u32) -> Option<Handle> {
    unsafe {
        CreateFileW(
            PCWSTR(path.as_ptr()),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED,
            None,
        )
    }
    .ok()
    .map(Handle)
}

/// VID, PID, страница и назначение коллекции. Открывается без прав доступа —
/// так атрибуты читаются и у занятых кем-то коллекций.
fn describe(path: &[u16]) -> Option<(u16, u16, u16, u16)> {
    let handle = open(path, 0)?;
    let mut attributes = HIDD_ATTRIBUTES { Size: size_of::<HIDD_ATTRIBUTES>() as u32, ..Default::default() };
    if !unsafe { HidD_GetAttributes(handle.0, &mut attributes) } {
        return None;
    }
    let mut preparsed = PHIDP_PREPARSED_DATA::default();
    if !unsafe { HidD_GetPreparsedData(handle.0, &mut preparsed) } {
        return None;
    }
    let mut caps = HIDP_CAPS::default();
    let status = unsafe { HidP_GetCaps(preparsed, &mut caps) };
    let _ = unsafe { HidD_FreePreparsedData(preparsed) };
    status.is_ok().then_some((attributes.VendorID, attributes.ProductID, caps.UsagePage, caps.Usage))
}

/// Коллекция, открытая на чтение и запись, с одним чтением в полёте.
struct Port {
    handle: Handle,
    event: Handle,
    overlapped: Box<OVERLAPPED>,
    buffer: Box<[u8; 64]>,
    pending: bool,
}

impl Port {
    fn open(path: &[u16]) -> Option<Self> {
        let handle = open(path, (GENERIC_READ | GENERIC_WRITE).0)?;
        let event = Handle(unsafe { CreateEventW(None, true, false, None) }.ok()?);
        Some(Self { handle, event, overlapped: Box::default(), buffer: Box::new([0; 64]), pending: false })
    }

    /// Ставит чтение, если его ещё нет.
    fn arm(&mut self) {
        if self.pending {
            return;
        }
        *self.overlapped = OVERLAPPED { hEvent: self.event.0, ..Default::default() };
        // Ответ «в работе» — обычное дело: чтение закончится, когда придёт
        // отчёт, и событие встанет
        let _ = unsafe { ReadFile(self.handle.0, Some(&mut self.buffer[..]), None, Some(&mut *self.overlapped)) };
        self.pending = true;
    }

    /// Забирает пришедший отчёт.
    fn take(&mut self) -> Option<Vec<u8>> {
        let mut read = 0u32;
        let done = unsafe { GetOverlappedResult(self.handle.0, &*self.overlapped, &mut read, false) };
        self.pending = false;
        done.ok()?;
        Some(self.buffer[..read as usize].to_vec())
    }

    fn write(&self, report: &[u8]) -> bool {
        let event = match unsafe { CreateEventW(None, true, false, None) } {
            Ok(event) => Handle(event),
            Err(_) => return false,
        };
        let mut overlapped = OVERLAPPED { hEvent: event.0, ..Default::default() };
        let _ = unsafe { WriteFile(self.handle.0, Some(report), None, Some(&mut overlapped)) };
        if unsafe { WaitForSingleObject(event.0, WAIT.as_millis() as u32) } != WAIT_OBJECT_0 {
            let _ = unsafe { CancelIoEx(self.handle.0, Some(&overlapped)) };
            let mut written = 0;
            let _ = unsafe { GetOverlappedResult(self.handle.0, &overlapped, &mut written, true) };
            return false;
        }
        let mut written = 0u32;
        unsafe { GetOverlappedResult(self.handle.0, &overlapped, &mut written, false) }.is_ok()
    }
}

impl Drop for Port {
    fn drop(&mut self) {
        if self.pending {
            // Чтение в полёте пишет в наш буфер: снять его и дождаться, иначе
            // система писала бы в освобождённую память
            let _ = unsafe { CancelIoEx(self.handle.0, Some(&*self.overlapped)) };
            let mut read = 0;
            let _ = unsafe { GetOverlappedResult(self.handle.0, &*self.overlapped, &mut read, true) };
        }
    }
}

/// Длинная коллекция для вопросов и короткая — ради ошибок приёмника.
struct Channel {
    long: std::cell::RefCell<Port>,
    short: Option<std::cell::RefCell<Port>>,
}

/// Чем ответило устройство.
#[derive(Debug, PartialEq, Eq)]
enum Reply {
    Data([u8; 16]),
    /// Устройство или приёмник отказали: нет функции, слот пуст, спит.
    Refused,
}

impl Channel {
    fn open(interfaces: &Interfaces) -> Option<Self> {
        let long = Port::open(interfaces.long.as_ref()?)?;
        let short = interfaces.short.as_ref().and_then(|path| Port::open(path));
        Some(Self { long: long.into(), short: short.map(Into::into) })
    }

    /// Шлёт длинный запрос и ждёт свой ответ. `None` — ответа не дождались.
    fn ask(&self, index: u8, feature: u8, function: u8, params: &[u8]) -> Option<Reply> {
        let report = request(index, feature, function, params);

        // Чтения ставим до записи: ответ приходит за миллисекунды и иначе
        // проскочил бы мимо
        self.long.borrow_mut().arm();
        if let Some(short) = &self.short {
            short.borrow_mut().arm();
        }
        if !self.long.borrow().write(&report) {
            return None;
        }

        let deadline = Instant::now() + WAIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            let mut events = vec![self.long.borrow().event.0];
            if let Some(short) = &self.short {
                events.push(short.borrow().event.0);
            }
            let woke = unsafe { WaitForMultipleObjects(&events, false, left.as_millis() as u32) };
            let which = woke.0.wrapping_sub(WAIT_OBJECT_0.0) as usize;
            let port = match which {
                0 => &self.long,
                1 => self.short.as_ref()?,
                _ => return None,
            };
            let received = port.borrow_mut().take();
            port.borrow_mut().arm();
            if let Some(reply) = received.and_then(|report| answer(&report, index, feature, function)) {
                return Some(reply);
            }
        }
    }

    /// Номер функции по её идентификатору; `None` — её нет или устройство молчит.
    fn feature(&self, index: u8, id: u16) -> Option<u8> {
        match self.ask(index, ROOT, 0, &id.to_be_bytes())? {
            Reply::Data(data) if data[0] != 0 => Some(data[0]),
            _ => None,
        }
    }
}

fn request(index: u8, feature: u8, function: u8, params: &[u8]) -> [u8; LONG_LEN] {
    let mut report = [0u8; LONG_LEN];
    report[0] = LONG;
    report[1] = index;
    report[2] = feature;
    report[3] = (function << 4) | SOFTWARE_ID;
    let room = (LONG_LEN - 4).min(params.len());
    report[4..4 + room].copy_from_slice(&params[..room]);
    report
}

/// Узнаёт свой ответ среди всех отчётов коллекции.
///
/// - `11 idx feat fn|sw data…` — ответ HID++ 2.0;
/// - `11 idx FF feat fn|sw err` — отказ HID++ 2.0;
/// - `10 idx 8F feat fn|sw err` — отказ HID++ 1.0: так приёмник отвечает за
///   пустой или спящий слот.
fn answer(report: &[u8], index: u8, feature: u8, function: u8) -> Option<Reply> {
    let address = (function << 4) | SOFTWARE_ID;
    match report {
        [LONG, idx, 0xff, feat, addr, ..] if report.len() >= LONG_LEN && *idx == index && *feat == feature && *addr == address => Some(Reply::Refused),
        [SHORT, idx, 0x8f, feat, addr, ..] if report.len() >= SHORT_LEN && *idx == index && *feat == feature && *addr == address => Some(Reply::Refused),
        [LONG, idx, feat, addr, rest @ ..] if report.len() >= LONG_LEN && *idx == index && *feat == feature && *addr == address => {
            let mut data = [0u8; 16];
            data.copy_from_slice(&rest[..16]);
            Some(Reply::Data(data))
        }
        _ => None,
    }
}

/// Одно устройство за индексом: имя, вид и заряд. `None` — его там нет, оно
/// спит или не говорит о заряде.
fn ask_device(channel: &Channel, index: u8, product: u16, receiver: &str) -> Option<Device> {
    // Версия протокола — заодно проверка, что за индексом кто-то живой и
    // говорит на HID++ 2.0. Приёмник за своим индексом отвечает отказом 1.0
    match channel.ask(index, ROOT, 1, &[0, 0, 0x5a])? {
        Reply::Data(data) if data[0] >= 2 => {}
        _ => return None,
    }

    let (name, kind) = name_and_kind(channel, index);
    if kind == Some(DeviceType::Receiver) {
        return None;
    }
    let (level, charging) = battery(channel, index)?;

    Some(Device {
        id: device_id(product, receiver, index),
        name: name.unwrap_or_else(|| "Logitech".to_owned()),
        kind: kind.map_or(Kind::Other, DeviceType::kind),
        level,
        charging,
        source: Source::Logitech,
        usb: Some((LOGITECH, product)),
    })
}

/// Вид устройства из функции 0x0005.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeviceType {
    Keyboard,
    Mouse,
    Headset,
    Gamepad,
    Receiver,
    Other,
}

impl DeviceType {
    fn from_code(code: u8) -> Self {
        match code {
            0 | 2 => DeviceType::Keyboard,
            3..=5 => DeviceType::Mouse,
            7 => DeviceType::Receiver,
            8 => DeviceType::Headset,
            11 | 12 => DeviceType::Gamepad,
            _ => DeviceType::Other,
        }
    }

    fn kind(self) -> Kind {
        match self {
            DeviceType::Keyboard => Kind::Keyboard,
            DeviceType::Mouse => Kind::Mouse,
            DeviceType::Headset => Kind::Headset,
            DeviceType::Gamepad => Kind::Gamepad,
            DeviceType::Receiver | DeviceType::Other => Kind::Other,
        }
    }
}

fn name_and_kind(channel: &Channel, index: u8) -> (Option<String>, Option<DeviceType>) {
    let Some(feature) = channel.feature(index, FEATURE_NAME) else {
        return (None, None);
    };
    let kind = match channel.ask(index, feature, 2, &[]) {
        Some(Reply::Data(data)) => Some(DeviceType::from_code(data[0])),
        _ => None,
    };
    let length = match channel.ask(index, feature, 0, &[]) {
        Some(Reply::Data(data)) => data[0] as usize,
        _ => return (None, kind),
    };
    let mut name = Vec::new();
    while name.len() < length.min(64) {
        let Some(Reply::Data(chunk)) = channel.ask(index, feature, 1, &[name.len() as u8]) else { break };
        let take = (length - name.len()).min(chunk.len());
        name.extend_from_slice(&chunk[..take]);
    }
    let name = String::from_utf8_lossy(&name).trim_end_matches('\0').trim().to_owned();
    ((!name.is_empty()).then_some(name), kind)
}

/// Заряд в процентах и «заряжается»: сперва единая функция заряда, затем
/// старая с процентами, затем напряжение по кривой.
fn battery(channel: &Channel, index: u8) -> Option<(u8, Option<bool>)> {
    if let Some(feature) = channel.feature(index, FEATURE_UNIFIED_BATTERY) {
        // Точный процент есть не у всех: его наличие — флаг 0x02 во втором
        // байте `get_capabilities`. Без него первый байт статуса — ноль, а не
        // пустая батарея
        let exact = match channel.ask(index, feature, 0, &[]) {
            Some(Reply::Data(data)) => data[1] & 0x02 != 0,
            _ => false,
        };
        if let Some(Reply::Data(data)) = channel.ask(index, feature, 1, &[]) {
            return unified(&data, exact);
        }
    }
    if let Some(feature) = channel.feature(index, FEATURE_BATTERY_STATUS) {
        if let Some(Reply::Data(data)) = channel.ask(index, feature, 0, &[]) {
            return status(&data);
        }
    }
    if let Some(feature) = channel.feature(index, FEATURE_BATTERY_VOLTAGE) {
        if let Some(Reply::Data(data)) = channel.ask(index, feature, 0, &[]) {
            return voltage(&data);
        }
    }
    None
}

/// 0x1004, `get_status`: процент, флаги ступени, состояние зарядки.
///
/// Без точного процента берём ступень — полная, хорошая, низкая, критическая
/// — серединой её диапазона: это приблизительно, но не выдумка, как был бы
/// ноль из пустого байта.
fn unified(data: &[u8; 16], exact: bool) -> Option<(u8, Option<bool>)> {
    let percent = if exact {
        data[0]
    } else {
        match data[1] {
            flags if flags & 0x08 != 0 => 100,
            flags if flags & 0x04 != 0 => 60,
            flags if flags & 0x02 != 0 => 20,
            flags if flags & 0x01 != 0 => 5,
            _ => return None,
        }
    };
    if percent > 100 {
        return None;
    }
    // 1 — заряжается, 2 — заряжается медленно, 3 — полна на зарядке
    Some((percent, Some(matches!(data[2], 1..=3))))
}

/// 0x1000, `GetBatteryLevelStatus`: процент, следующий порог, состояние.
fn status(data: &[u8; 16]) -> Option<(u8, Option<bool>)> {
    let percent = data[0];
    // 5 и 6 — батарея неисправна или перегрета: процент тогда не верен
    if percent == 0 || percent > 100 || data[2] >= 5 {
        return None;
    }
    Some((percent, Some(matches!(data[2], 1..=4))))
}

/// 0x1001, `GetBatteryVoltage`: милливольты и флаги; процент — по кривой
/// разряда литиевой ячейки.
fn voltage(data: &[u8; 16]) -> Option<(u8, Option<bool>)> {
    let millivolts = u16::from_be_bytes([data[0], data[1]]);
    let charging = data[2] & 0x80 != 0;
    Some((percent_of(millivolts)?, Some(charging)))
}

/// Кривая разряда литий-ионной ячейки: милливольты → проценты.
const CURVE: [(u16, u8); 13] = [
    (4186, 100), (4067, 90), (3989, 80), (3922, 70), (3859, 60), (3811, 50),
    (3778, 40), (3751, 30), (3717, 20), (3671, 10), (3646, 5), (3579, 2), (3500, 0),
];

fn percent_of(millivolts: u16) -> Option<u8> {
    // Ниже двух и выше пяти вольт — не ячейка, а мусор в ответе
    if !(2000..=5000).contains(&millivolts) {
        return None;
    }
    if millivolts >= CURVE[0].0 {
        return Some(100);
    }
    for pair in CURVE.windows(2) {
        let ((high_mv, high), (low_mv, low)) = (pair[0], pair[1]);
        if millivolts >= low_mv {
            let span = u32::from(high_mv - low_mv);
            let over = u32::from(millivolts - low_mv);
            return Some(low + ((u32::from(high - low) * over + span / 2) / span) as u8);
        }
    }
    Some(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn long_reply(index: u8, feature: u8, function: u8, data: &[u8]) -> Vec<u8> {
        let mut report = vec![LONG, index, feature, (function << 4) | SOFTWARE_ID];
        report.extend_from_slice(data);
        report.resize(LONG_LEN, 0);
        report
    }

    #[test]
    fn a_request_carries_our_software_id() {
        let report = request(2, 0x00, 0, &[0x10, 0x04]);
        assert_eq!(&report[..6], &[LONG, 2, 0x00, SOFTWARE_ID, 0x10, 0x04]);
        assert_eq!(report.len(), LONG_LEN);
    }

    #[test]
    fn our_reply_is_recognised_and_strangers_are_skipped() {
        let reply = long_reply(1, 5, 1, &[42]);
        assert!(matches!(answer(&reply, 1, 5, 1), Some(Reply::Data(data)) if data[0] == 42));

        // Ответ G HUB: тот же вопрос, другой отправитель
        let mut foreign = reply.clone();
        foreign[3] = (1 << 4) | 0x01;
        assert_eq!(answer(&foreign, 1, 5, 1), None);
        // Чужое устройство и чужая функция
        assert_eq!(answer(&long_reply(2, 5, 1, &[42]), 1, 5, 1), None);
        assert_eq!(answer(&long_reply(1, 6, 1, &[42]), 1, 5, 1), None);
        // Уведомление устройства (software id 0)
        let mut event = reply.clone();
        event[3] = 1 << 4;
        assert_eq!(answer(&event, 1, 5, 1), None);
    }

    #[test]
    fn refusals_of_both_protocol_versions_are_recognised() {
        let hidpp2 = vec![LONG, 3, 0xff, 0x07, (1 << 4) | SOFTWARE_ID, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(answer(&hidpp2, 3, 0x07, 1), Some(Reply::Refused));
        let hidpp1 = vec![SHORT, 3, 0x8f, 0x00, (1 << 4) | SOFTWARE_ID, 0x09, 0];
        assert_eq!(answer(&hidpp1, 3, 0x00, 1), Some(Reply::Refused));
        // Отказ на чужой вопрос — не наш
        let other = vec![SHORT, 3, 0x8f, 0x00, (1 << 4) | 0x02, 0x09, 0];
        assert_eq!(answer(&other, 3, 0x00, 1), None);
    }

    #[test]
    fn a_short_report_is_never_mistaken_for_data() {
        let truncated = vec![LONG, 1, 5, (1 << 4) | SOFTWARE_ID, 42];
        assert_eq!(answer(&truncated, 1, 5, 1), None);
    }

    #[test]
    fn battery_functions_decode() {
        let mut data = [0u8; 16];
        data[0] = 64;
        data[2] = 1;
        assert_eq!(unified(&data, true), Some((64, Some(true))));
        data[2] = 0;
        assert_eq!(unified(&data, true), Some((64, Some(false))));
        data[0] = 180;
        assert_eq!(unified(&data, true), None);
        // Без точного процента — ступень, а не ноль из пустого байта
        let mut data = [0u8; 16];
        data[1] = 0x08;
        assert_eq!(unified(&data, false), Some((100, Some(false))));
        data[1] = 0x02;
        assert_eq!(unified(&data, false), Some((20, Some(false))));
        data[1] = 0;
        assert_eq!(unified(&data, false), None);

        let mut data = [0u8; 16];
        data[0] = 30;
        data[2] = 0;
        assert_eq!(status(&data), Some((30, Some(false))));
        data[2] = 6;
        assert_eq!(status(&data), None, "перегретая батарея — не процент");

        let mut data = [0u8; 16];
        data[..2].copy_from_slice(&3811u16.to_be_bytes());
        data[2] = 0x80;
        assert_eq!(voltage(&data), Some((50, Some(true))));
    }

    #[test]
    fn the_voltage_curve_interpolates_and_clamps() {
        assert_eq!(percent_of(4300), Some(100));
        assert_eq!(percent_of(4186), Some(100));
        assert_eq!(percent_of(3811), Some(50));
        assert_eq!(percent_of(3835), Some(55));
        assert_eq!(percent_of(3400), Some(0));
        assert_eq!(percent_of(0), None);
        assert_eq!(percent_of(9000), None);
    }

    #[test]
    fn twin_receivers_give_twin_keys() {
        let one = device_id(0xc54d, "vid_046d&pid_c54d&mi_02#8&9e8af32&0", 1);
        let two = device_id(0xc54d, "vid_046d&pid_c54d&mi_02#8&2222222&0", 1);
        assert_ne!(one, two);
        assert!(one.starts_with("lg:c54d:") && one.ends_with(":01"), "{one}");
    }

    #[test]
    fn device_types_map_to_kinds() {
        assert_eq!(DeviceType::from_code(3).kind(), Kind::Mouse);
        assert_eq!(DeviceType::from_code(0).kind(), Kind::Keyboard);
        assert_eq!(DeviceType::from_code(8).kind(), Kind::Headset);
        assert_eq!(DeviceType::from_code(7), DeviceType::Receiver);
        assert_eq!(DeviceType::from_code(19).kind(), Kind::Other);
    }

    #[test]
    fn collections_of_one_device_share_a_key() {
        let short = r"\\?\hid#vid_046d&pid_c54d&mi_02&col01#8&9e8af32&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        let long = r"\\?\hid#vid_046d&pid_c54d&mi_02&col02#8&9e8af32&0&0001#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        assert_eq!(parent_key(short), parent_key(long));
        let other = r"\\?\hid#vid_046d&pid_c547&mi_02&col02#8&1111111&0&0001#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        assert_ne!(parent_key(short), parent_key(other));
        let twin = r"\\?\hid#vid_046d&pid_c54d&mi_02&col02#8&2222222&0&0001#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        assert_ne!(parent_key(long), parent_key(twin), "два одинаковых донгла — два ключа");
    }

    /// Живой опрос: печатает, что видно сейчас.
    #[test]
    #[ignore = "живое железо"]
    fn live_devices() {
        println!("{:#?}", devices());
    }
}
