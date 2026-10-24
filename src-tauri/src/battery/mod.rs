//! Заряд периферии: мыши, клавиатуры, гарнитуры, геймпады, телефоны.
//!
//! Общего способа узнать заряд у Windows нет. Сама система знает его только
//! у Bluetooth — у любого бренда, — а устройства на своих донглах отдают его
//! лишь своему протоколу. Поэтому источников три:
//!
//! - **Windows** (`pnp.rs`) — ключ заряда, который Bluetooth-стек держит у
//!   узла устройства;
//! - **Logitech** (`hidpp.rs`) — HID++, один протокол на всю линейку:
//!   LIGHTSPEED, Unifying, провод;
//! - **гарнитуры** (`headset.rs`) — докачиваемый по согласию HeadsetControl,
//!   который знает гарнитуры десятка брендов.
//!
//! В снимок попадает только подключённое устройство, которое сейчас сообщает
//! заряд: спящих и отключённых строк нет (слово пользователя).
//!
//! Устройство то же, что у звука: свой поток, опрос только при видимом окне,
//! событие в окно. В этом файле вызовов Windows нет — только состояние, слияние
//! источников и правила, которые проверяются тестом.

#[cfg(windows)]
mod headset;
#[cfg(windows)]
mod hidpp;
#[cfg(windows)]
mod pnp;

use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

/// Метка окна виджета.
const WIDGET: &str = "battery";

/// Событие, которым снимок расходится по окнам.
const BATTERY_EVENT: &str = "battery";

/// Шаг цикла: так быстро замечаем, что окно показали.
const STEP: Duration = Duration::from_secs(1);

/// Как часто спрашиваем заряд. Он меняется на проценты за десятки минут, а
/// каждый вопрос к мыши — это обмен по радио.
const PERIOD: Duration = Duration::from_secs(60);

/// Через сколько переспрашиваем, если знакомое устройство не ответило.
///
/// Один пропущенный ответ — не уход: мышь занята, G HUB перебил вопрос,
/// HeadsetControl не уложился. Строка пропадала бы на целую минуту и
/// возвращалась, поэтому устройство держится до повторного вопроса и уходит
/// только со вторым промахом подряд.
const RETRY: Duration = Duration::from_secs(5);

/// Вид устройства — ради значка в строке.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Mouse,
    Keyboard,
    Headset,
    Gamepad,
    Phone,
    Other,
}

/// Откуда пришёл заряд.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    Windows,
    Logitech,
    Headset,
}

/// Устройство с зарядом.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    /// Ключ, стабильный между запусками: по нему устройство снимают с
    /// карточки (`hidden_batteries`). `bt:<адрес>`, `lg:<модель>`,
    /// `hs:<vid>:<pid>`.
    pub id: String,
    pub name: String,
    pub kind: Kind,
    /// Проценты, 0–100.
    pub level: u8,
    /// `None` — источник не говорит, заряжается ли.
    pub charging: Option<bool>,
    pub source: Source,
    /// USB-идентификатор донгла или устройства: по нему одна гарнитура не
    /// встаёт двумя строками, если её знают и HID++, и HeadsetControl.
    #[serde(skip)]
    pub usb: Option<(u16, u16)>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    /// Первый опрос прошёл: до него пустой список — не «ничего нет», а «ещё
    /// не спросили».
    pub ready: bool,
    pub devices: Vec<Device>,
}

#[derive(Default)]
pub struct Battery {
    view: Mutex<View>,
}

impl Battery {
    fn locked(&self) -> MutexGuard<'_, View> {
        self.view.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Снимок для окна, открытого между опросами.
#[tauri::command]
pub fn battery_state(state: State<'_, Battery>) -> View {
    state.locked().clone()
}

/// Сводит источники в один список.
///
/// Гарнитура, которую узнали и HID++, и HeadsetControl, остаётся строкой
/// HeadsetControl: он знает её протокол целиком. Заряд выше 100 зажимается —
/// источники бывают щедры. Порядок — по виду, затем по имени: строки не
/// прыгают от того, какой источник ответил первым.
fn merge(windows: Vec<Device>, logitech: Vec<Device>, headsets: Vec<Device>) -> Vec<Device> {
    let taken: Vec<(u16, u16)> = headsets.iter().filter_map(|device| device.usb).collect();
    let mut devices: Vec<Device> = windows
        .into_iter()
        .chain(logitech.into_iter().filter(|device| device.usb.is_none_or(|usb| !taken.contains(&usb))))
        .chain(headsets)
        .map(|mut device| {
            device.level = device.level.min(100);
            device
        })
        .collect();

    // Один ключ — одна строка: два узла одного Bluetooth-устройства или два
    // интерфейса одного донгла не должны дать двух строк
    let mut seen = Vec::new();
    devices.retain(|device| {
        let fresh = !seen.contains(&device.id);
        if fresh {
            seen.push(device.id.clone());
        }
        fresh
    });

    order(&mut devices);
    devices
}

/// По виду, затем по имени: строки не прыгают от того, кто ответил первым.
fn order(devices: &mut [Device]) {
    devices.sort_by(|a, b| {
        (a.kind as u8).cmp(&(b.kind as u8)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

/// Сводит свежий опрос с прошлым снимком.
///
/// Знакомое устройство, не ответившее впервые, остаётся с прежним зарядом и
/// попадает в список промахов; не ответившее второй раз подряд уходит.
/// Возвращает снимок и новые промахи: пустые — переспрашивать некого.
fn settle(previous: &[Device], fresh: Vec<Device>, missed: &[String]) -> (Vec<Device>, Vec<String>) {
    let mut devices = fresh;
    let mut misses = Vec::new();
    for old in previous {
        if devices.iter().any(|device| device.id == old.id) || missed.contains(&old.id) {
            continue;
        }
        misses.push(old.id.clone());
        devices.push(old.clone());
    }
    order(&mut devices);
    (devices, misses)
}

/// Окно показано: только тогда и спрашиваем (правило звука).
fn watched(app: &AppHandle) -> bool {
    app.get_webview_window(WIDGET)
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false)
}

/// Поднимает поток заряда. Живёт до конца работы приложения.
pub fn spawn(app: AppHandle) {
    if let Err(error) = std::thread::Builder::new()
        .name("battery".to_owned())
        .spawn(move || work(app))
    {
        eprintln!("[заряд] поток не поднялся: {error}");
    }
}

fn work(app: AppHandle) {
    let mut last: Option<Instant> = None;
    let mut was_watched = false;
    let mut missed: Vec<String> = Vec::new();
    // Первый вопрос после показа окна переспрашивается так же, как промах:
    // при запуске HeadsetControl и донглы бывают не готовы, и окно стояло бы
    // пустым целую минуту
    let mut recheck = false;

    loop {
        std::thread::sleep(STEP);

        let now_watched = watched(&app);
        // Окно только что показали — спрашиваем сразу, а не через минуту
        let shown = now_watched && !was_watched;
        let wait = if missed.is_empty() && !recheck { PERIOD } else { RETRY };
        let due = last.is_none_or(|at| at.elapsed() >= wait) || shown;
        was_watched = now_watched;
        if !now_watched || !due {
            continue;
        }
        last = Some(Instant::now());
        recheck = shown;

        let fresh = poll(&app);
        let state = app.state::<Battery>();
        let previous = state.locked().devices.clone();
        let (devices, misses) = settle(&previous, fresh, &missed);
        missed = misses;
        let view = View { ready: true, devices };
        let changed = {
            let mut current = state.locked();
            let changed = *current != view;
            *current = view.clone();
            changed
        };
        if changed {
            let _ = app.emit_to(WIDGET, BATTERY_EVENT, view.clone());
            // Страница виджета в настройках показывает тот же список
            let _ = app.emit_to("settings", BATTERY_EVENT, view);
        }
    }
}

#[cfg(windows)]
fn poll(app: &AppHandle) -> Vec<Device> {
    let headsets = match app.state::<crate::tools::Tools>().locate(crate::tools::catalog::Component::HeadsetControl) {
        Some(exe) => headset::devices(&exe),
        None => Vec::new(),
    };
    merge(pnp::devices(), hidpp::devices(), headsets)
}

#[cfg(not(windows))]
fn poll(_app: &AppHandle) -> Vec<Device> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str, name: &str, kind: Kind, source: Source, usb: Option<(u16, u16)>) -> Device {
        Device { id: id.to_owned(), name: name.to_owned(), kind, level: 50, charging: None, source, usb }
    }

    #[test]
    fn a_headset_known_twice_stays_one_row_from_headsetcontrol() {
        let logitech = vec![
            device("lg:g-pro-x", "PRO X Wireless", Kind::Headset, Source::Logitech, Some((0x046d, 0x0aba))),
            device("lg:mouse", "Mouse", Kind::Mouse, Source::Logitech, Some((0x046d, 0xc547))),
        ];
        let headsets = vec![device("hs:046d:0aba", "Logitech PRO X", Kind::Headset, Source::Headset, Some((0x046d, 0x0aba)))];

        let merged = merge(Vec::new(), logitech, headsets);

        assert_eq!(merged.iter().map(|device| device.id.as_str()).collect::<Vec<_>>(), ["lg:mouse", "hs:046d:0aba"]);
    }

    #[test]
    fn one_key_is_one_row() {
        let windows = vec![
            device("bt:AABBCCDDEEFF", "Phone", Kind::Phone, Source::Windows, None),
            device("bt:AABBCCDDEEFF", "Phone HF", Kind::Phone, Source::Windows, None),
        ];
        assert_eq!(merge(windows, Vec::new(), Vec::new()).len(), 1);
    }

    #[test]
    fn rows_are_ordered_by_kind_then_name_and_levels_are_clamped() {
        let mut loud = device("bt:2", "zeta pad", Kind::Gamepad, Source::Windows, None);
        loud.level = 140;
        let windows = vec![loud, device("bt:1", "Alpha pad", Kind::Gamepad, Source::Windows, None)];
        let logitech = vec![device("lg:1", "Mouse", Kind::Mouse, Source::Logitech, None)];

        let merged = merge(windows, logitech, Vec::new());

        assert_eq!(merged.iter().map(|device| device.name.as_str()).collect::<Vec<_>>(), ["Mouse", "Alpha pad", "zeta pad"]);
        assert_eq!(merged[2].level, 100);
    }

    #[test]
    fn a_device_leaves_only_on_the_second_miss_in_a_row() {
        let mouse = device("lg:1", "Mouse", Kind::Mouse, Source::Logitech, None);
        let headset = device("hs:1", "Headset", Kind::Headset, Source::Headset, None);
        let previous = vec![mouse.clone(), headset.clone()];

        let (kept, missed) = settle(&previous, vec![headset.clone()], &[]);
        assert_eq!(kept, previous, "первый промах строку не снимает");
        assert_eq!(missed, ["lg:1"]);

        let (left, missed) = settle(&kept, vec![headset.clone()], &missed);
        assert_eq!(left, [headset.clone()], "второй подряд — снимает");
        assert!(missed.is_empty());

        let (back, missed) = settle(&kept, vec![mouse.clone(), headset], &["lg:1".to_owned()]);
        assert_eq!(back, previous, "ответившее снова — на месте и без промаха");
        assert!(missed.is_empty());
    }

    #[test]
    fn the_view_arrives_in_camel_case_without_usb() {
        let view = View { ready: true, devices: vec![device("lg:1", "Mouse", Kind::Mouse, Source::Logitech, Some((1, 2)))] };
        let json = serde_json::to_string(&view).expect("сериализуется");
        assert!(json.contains(r#""kind":"mouse""#), "{json}");
        assert!(json.contains(r#""source":"logitech""#), "{json}");
        assert!(!json.contains("usb"), "{json}");
    }
}
