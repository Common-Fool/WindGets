//! Звук: громкость вывода, молчание микрофона и выбор устройства.
//!
//! ВАЖНО: всё это COM, а на COM же держится WebView2 — тот самый, из-за
//! которого `opener` однажды оставил без ответа все окна разом. Поэтому вся
//! работа с устройствами живёт в **отдельном потоке со своим MTA**
//! ([`work`]), а наружу ходят только простые данные через канал.
//!
//! Держать `IAudioEndpointVolume` в состоянии Tauri нельзя и технически:
//! COM-объекты в windows-rs не `Send`. Это не придирка компилятора, а само
//! устройство COM — объект принадлежит апартаменту, в котором создан, и
//! главный поток занят WebView2 в STA.
//!
//! В этом файле поэтому нет ни одного вызова Windows: только состояние,
//! команды и правила, которые можно проверить тестом.

mod appicon;
#[cfg(windows)]
mod endpoints;
#[cfg(windows)]
mod policy;
#[cfg(windows)]
mod sessions;

use std::collections::BTreeMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

/// Метка окна виджета: по ней опрос узнаёт, смотрит ли на него кто-нибудь.
const WIDGET: &str = "sound";

/// Событие, которым состояние расходится по окнам.
const SOUND_EVENT: &str = "sound";

/// Шаг опроса. Полсекунды — чтобы громкость, выкрученная медиаклавишами,
/// догоняла полосу быстрее, чем это успеет надоесть.
const STEP: Duration = Duration::from_millis(500);

/// Какую громкость возвращает значок, когда звук включают обратно.
///
/// Не прежнюю, а полную, и это выбор, а не упрощение: значок нажимают, чтобы
/// услышать, — а прежней громкостью бывает и тройка процентов, выкрученная
/// когда-то под ночной фильм.
const RESTORE_VOLUME: u8 = 100;

/// Тип устройства. По нему виджет выбирает значок — ровно так же, как это
/// делает системный флайаут.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Speakers,
    Headphones,
    /// Звук уходит в монитор по HDMI или DisplayPort.
    Display,
    Other,
}

/// Строка микшера: одна звучащая программа.
///
/// Значок сюда не едет: он весит вчетверо больше всей остальной строки, а
/// меняться ему не с чего. Окно спрашивает его отдельно и один раз — командой
/// [`sound_icon`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    /// Имя процесса без расширения — оно же ключ строки и адрес команды.
    pub id: String,
    pub name: String,
    /// Проценты, 0…100.
    pub volume: u8,
    pub muted: bool,
    /// Путь к exe: по нему берётся значок. Наружу не уходит.
    #[serde(skip)]
    pub path: Option<String>,
}

/// Какая панель раскрыта в окне виджета.
///
/// ВАЖНО: **список собирается только под раскрытую панель.** Перечисление
/// устройств — это полтора десятка COM-вызовов, а перечисление сессий микшера
/// дороже вдвое; платить за них, пока панель свёрнута, незачем. Плитке хватает
/// текущего устройства, а ротация по кругу спрашивает список сама, в момент
/// нажатия.
///
/// Панель раскрыта ровно одна: место в карточке одно, и две сразу в него не
/// поместятся.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Panel {
    /// Всё свёрнуто — спрашиваем только громкость и молчание.
    #[default]
    None,
    Outputs,
    Inputs,
    Mixer,
}

/// Устройство, каким его отдала система: ещё без подписи и без признака
/// текущего.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub id: String,
    pub name: String,
    pub kind: Kind,
}

/// Устройство, каким его видит виджет.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    /// Полное имя из системы: «Динамики (Redmi 电脑音箱)».
    pub name: String,
    /// Подпись на плитке — обычно короткая, до первой скобки.
    pub short: String,
    pub kind: Kind,
    pub current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Output {
    /// Проценты, 0…100.
    pub volume: u8,
    pub muted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Microphone {
    pub muted: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    /// `null` — устройства вывода нет вовсе. Именно `null`, а не нулевая
    /// громкость: «нет» и «выкручено в ноль» это разные вещи.
    pub output: Option<Output>,
    pub microphone: Option<Microphone>,
    /// Куда идёт звук — это и есть подпись третьей плитки.
    ///
    /// Отдельным полем, а не поиском в списке: список приходит только под
    /// раскрытую панель, а плитка подписана всегда.
    pub current: Option<Device>,
    /// Устройства вывода. Пусто, пока их панель свёрнута.
    pub devices: Vec<Device>,
    /// Устройства ввода. Пусто, пока их панель свёрнута.
    pub inputs: Vec<Device>,
    /// Кто сейчас звучит. Пусто, пока микшер свёрнут.
    pub sessions: Vec<Track>,
    /// Переключать вывод нечем: `IPolicyConfig` не завёлся. Плитка устройства
    /// при этом остаётся — она показывает, куда идёт звук.
    pub can_switch: bool,
    /// Громкость, которую вернёт значок, когда звук включат обратно.
    ///
    /// Едет на фронт, чтобы тот предсказал нажатие и перекрасился сразу, не
    /// дожидаясь тика опроса. Своим числом там не обойтись: два одинаковых
    /// числа по разные стороны однажды разойдутся.
    pub restore: u8,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Action {
    /// Поставить громкость. Ноль допустим и молчания не ставит: тишина и так
    /// полная, а два признака тишины дали бы значку два состояния для отмены.
    Volume { percent: u8 },
    ToggleMute,
    ToggleMicrophone,
    /// Следующее устройство по кругу — левая кнопка на плитке.
    NextDevice,
    /// Следующий микрофон по кругу — подпись микрофона у столбика.
    NextInput,
    /// Устройство вывода из раскрытого списка.
    Device { id: String },
    /// Устройство ввода из раскрытого списка.
    Input { id: String },
    /// Громкость одной программы в микшере.
    SessionVolume { id: String, percent: u8 },
    /// Молчание одной программы в микшере.
    SessionMute { id: String, muted: bool },
    /// Какая панель раскрыта. Под неё и собираются списки.
    Panel { panel: Panel },
}

/// Что делать по нажатию на значок динамика.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Switch {
    /// Поставить молчание: сейчас звучит.
    Mute,
    /// Снять молчание и вывести громкость на полную.
    UnmuteTo(u8),
}

/// Последнее состояние и труба к потоку звука.
#[derive(Default)]
pub struct Sound {
    latest: Mutex<View>,
    orders: OnceLock<Sender<Action>>,
    /// Значки программ, снятые потоком звука, по именам строк микшера.
    ///
    /// Лежат тут, а не едут в событии: точки значка весят вчетверо больше
    /// всей остальной строки, а меняться им не с чего. Окно спрашивает их
    /// командой — по разу на программу за всё время работы.
    icons: Mutex<BTreeMap<String, appicon::Icon>>,
}

impl Sound {
    fn snapshot(&self) -> View {
        // Отравленный мьютекс не повод падать: данные внутри осмысленные
        self.latest
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// Снимок для окна, открытого между тиками: иначе оно ждало бы данных до
/// следующего опроса.
#[tauri::command]
pub fn sound_state(state: State<'_, Sound>) -> View {
    state.snapshot()
}

/// Значок программы для строки микшера.
///
/// Ходит в карту, а не в оболочку Windows: значок туда положил поток звука,
/// когда собирал микшер. `null` — значка у программы нет или его не дали.
#[tauri::command]
pub fn sound_icon(state: State<'_, Sound>, name: String) -> Option<appicon::Icon> {
    state
        .icons
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&name)
        .cloned()
}

/// Кладёт команду в очередь потока звука и возвращается сразу.
///
/// ВАЖНО: ждать COM из потока команды нельзя — приём IPC у WebView2 идёт
/// главным потоком, и всё, что его задерживает, выглядит как «нажатия
/// проваливаются во всех окнах разом».
#[tauri::command]
pub fn sound_control(state: State<'_, Sound>, action: Action) -> Result<(), String> {
    let orders = state
        .orders
        .get()
        .ok_or_else(|| "звук ещё не завёлся".to_owned())?;

    orders
        .send(action)
        .map_err(|_| "поток звука не отвечает".to_owned())
}

/// Поднимает поток звука. Живёт до конца работы приложения.
pub fn spawn(app: AppHandle) {
    let (orders, inbox) = mpsc::channel();

    if app.state::<Sound>().orders.set(orders).is_err() {
        eprintln!("[звук] поток уже поднят");
        return;
    }

    // Свой поток ОС, а не задача рантайма: COM живёт в потоке, а не в задаче,
    // и переезд задачи на другой поток пула отобрал бы у неё апартамент
    if let Err(error) = std::thread::Builder::new()
        .name("sound".to_owned())
        .spawn(move || work(app, inbox))
    {
        eprintln!("[звук] поток не поднялся: {error}");
    }
}

/// Смотрит ли кто-нибудь на виджет.
///
/// Спрятанное окно опрашивать незачем. У Спотифая то же правило по другой
/// причине — там лимит запросов, а тут просто нет смысла будить COM.
fn watched(app: &AppHandle) -> bool {
    app.get_webview_window(WIDGET)
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false)
}

fn publish(app: &AppHandle, state: &Sound, next: View) {
    {
        let mut stored = state
            .latest
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        // Ничего не поменялось — молчим: две рассылки в секунду во все окна
        // впустую стоят дороже, чем это сравнение
        if *stored == next {
            return;
        }
        *stored = next.clone();
    }

    // Неудачу рассылки глотаем: окно могли спрятать прямо в этот момент
    let _ = app.emit(SOUND_EVENT, next);
}

/// Имя до первой скобки: «Динамики (Redmi 电脑音箱)» → «Динамики».
fn shorten(name: &str) -> &str {
    match name.split_once(" (") {
        Some((head, _)) if !head.is_empty() => head,
        _ => name,
    }
}

/// Раздаёт подписи и отмечает текущее устройство.
///
/// Совпавшие короткие имена разворачиваются обратно в полные: одинаковая
/// подпись у двух плиток хуже длинной. Два выхода HDMI на одной видеокарте —
/// ровно такой случай.
fn label(found: Vec<Found>, current: Option<&str>) -> Vec<Device> {
    let short: Vec<&str> = found.iter().map(|device| shorten(&device.name)).collect();

    found
        .iter()
        .enumerate()
        .map(|(index, device)| {
            let clashes = short
                .iter()
                .enumerate()
                .any(|(other, name)| other != index && *name == short[index]);

            Device {
                id: device.id.clone(),
                name: device.name.clone(),
                short: if clashes {
                    device.name.clone()
                } else {
                    short[index].to_owned()
                },
                kind: device.kind,
                current: current == Some(device.id.as_str()),
            }
        })
        .collect()
}

/// Одно устройство — для плитки, без разрешения совпавших подписей.
///
/// Совпадения важны в списке, где строки стоят рядом; на плитке устройство
/// одно, и сравнивать его не с чем.
fn one(device: &Found) -> Device {
    Device {
        id: device.id.clone(),
        name: device.name.clone(),
        short: shorten(&device.name).to_owned(),
        kind: device.kind,
        current: true,
    }
}

/// Тихо ли сейчас. Ноль громкости — такая же тишина, как и молчание.
fn silent(output: &Output) -> bool {
    output.muted || output.volume == 0
}

/// Что сделает нажатие на значок динамика.
///
/// Тишина снимается одинаково, чем бы она ни была вызвана: и снятым молчанием,
/// и нулём на полосе. Разбирать эти два случая порознь незачем — нажимают
/// значок ради одного и того же.
fn switch(output: &Output) -> Switch {
    if silent(output) {
        Switch::UnmuteTo(RESTORE_VOLUME)
    } else {
        Switch::Mute
    }
}

/// Следующее устройство по кругу.
///
/// Текущего в списке нет — начинаем сначала: устройство могли выдернуть, и
/// «переключать некуда» тут было бы неправдой.
fn next_device(devices: &[Found], current: Option<&str>) -> Option<String> {
    if devices.is_empty() {
        return None;
    }

    let at = current.and_then(|id| devices.iter().position(|device| device.id == id));
    let next = match at {
        Some(index) => (index + 1) % devices.len(),
        None => 0,
    };

    Some(devices[next].id.clone())
}

#[cfg(windows)]
fn work(app: AppHandle, inbox: Receiver<Action>) {
    use std::sync::mpsc::RecvTimeoutError;

    let endpoints = match endpoints::Endpoints::open() {
        Ok(endpoints) => endpoints,
        Err(error) => {
            eprintln!("[звук] устройства недоступны: {error}");
            return;
        }
    };

    // Переключатель вывода может не завестись вовсе: интерфейс
    // недокументированный. Виджет от этого не ломается — гаснет одна плитка
    let switcher = policy::Switcher::open();
    if switcher.is_none() {
        eprintln!("[звук] IPolicyConfig не завёлся: переключение вывода выключено");
    }

    // Что раскрыто в окне: под это и собираем списки
    let mut panel = Panel::None;

    loop {
        // Пришла команда — исполняем и тут же перечитываем: значение обязано
        // догнать нажатие, а не ждать полсекунды до тика
        let mut acted = match inbox.recv_timeout(STEP) {
            Ok(action) => {
                handle(&endpoints, switcher.as_ref(), &mut panel, action);
                true
            }
            Err(RecvTimeoutError::Timeout) => false,
            // Приложение уходит: держатель отправителя умер вместе с ним
            Err(RecvTimeoutError::Disconnected) => return,
        };

        // ВАЖНО: разбираем всю накопившуюся очередь и только потом рассылаем.
        //
        // Полоса громкости шлёт сообщение на каждое движение мыши — за секунду
        // их набегают десятки. Рассылка после каждого топила окно в событиях,
        // и виджет отставал от мыши: полоса вставала на место, потом на
        // мгновение откатывалась и вставала снова, догоняя очередь.
        while let Ok(action) = inbox.try_recv() {
            handle(&endpoints, switcher.as_ref(), &mut panel, action);
            acted = true;
        }

        if !acted && !watched(&app) {
            continue;
        }

        let output = endpoints.output();
        let microphone = endpoints.microphone();

        let playing = output.as_ref().map(|(device, _, _)| device.id.as_str());
        let listening = microphone.as_ref().map(|(device, _)| device.id.as_str());

        // Списки — только под раскрытую панель. Свёрнутая стоит нам ровно
        // двух вызовов: текущий выход и текущий вход
        let devices = match panel {
            Panel::Outputs => label(endpoints.outputs(), playing),
            _ => Vec::new(),
        };
        let inputs = match panel {
            Panel::Inputs => label(endpoints.inputs(), listening),
            _ => Vec::new(),
        };
        let sessions = match panel {
            Panel::Mixer => {
                let playing = endpoints.mixer().map(|mixer| sessions::tracks(&mixer));
                remember_icons(&app, playing.as_deref().unwrap_or_default());
                playing.unwrap_or_default()
            }
            _ => Vec::new(),
        };

        publish(
            &app,
            &app.state::<Sound>(),
            View {
                output: output
                    .as_ref()
                    .map(|(_, volume, muted)| Output {
                        volume: *volume,
                        muted: *muted,
                    }),
                microphone: microphone
                    .as_ref()
                    .map(|(_, muted)| Microphone { muted: *muted }),
                current: output.as_ref().map(|(device, _, _)| one(device)),
                devices,
                inputs,
                sessions,
                can_switch: switcher.is_some(),
                restore: RESTORE_VOLUME,
            },
        );
    }
}

/// Снимает значки программ, которых ещё не видели.
///
/// ВАЖНО: **только новых.** Поход в оболочку за значком — самое дорогое, что
/// здесь есть, и повторять его дважды в секунду для одних и тех же программ
/// незачем: значок у exe не меняется.
#[cfg(windows)]
fn remember_icons(app: &AppHandle, tracks: &[Track]) {
    let Some(state) = app.try_state::<Sound>() else {
        return;
    };

    for track in tracks {
        let known = state
            .icons
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains_key(&track.id);
        if known {
            continue;
        }

        let Some(path) = track.path.as_deref() else {
            continue;
        };
        let Some(icon) = appicon::of(path) else {
            continue;
        };

        state
            .icons
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(track.id.clone(), icon);
    }
}

/// Разбирает команду: правка панели остаётся здесь, всё остальное уходит в COM.
#[cfg(windows)]
fn handle(
    endpoints: &endpoints::Endpoints,
    switcher: Option<&policy::Switcher>,
    panel: &mut Panel,
    action: Action,
) {
    match action {
        Action::Panel { panel: next } => *panel = next,
        other => obey(endpoints, switcher, other),
    }
}

/// Исполняет одну команду. Отказы логируем, но поток не роняем.
#[cfg(windows)]
fn obey(
    endpoints: &endpoints::Endpoints,
    switcher: Option<&policy::Switcher>,
    action: Action,
) {
    let outcome = match action {
        // Полоса при молчании стоит на нуле, поэтому поднятая полоса означает
        // «хочу звук»: не снять молчание тут значит отбросить её обратно на
        // ноль и оставить тишину
        Action::Volume { percent } => endpoints.set_volume(percent).and_then(|()| {
            if percent > 0 {
                endpoints.set_muted(false)
            } else {
                Ok(())
            }
        }),
        Action::ToggleMute => match endpoints.output() {
            Some((_, volume, muted)) => match switch(&Output { volume, muted }) {
                Switch::Mute => endpoints.set_muted(true),
                Switch::UnmuteTo(percent) => endpoints
                    .set_muted(false)
                    .and_then(|()| endpoints.set_volume(percent)),
            },
            None => Ok(()),
        },
        Action::ToggleMicrophone => match endpoints.microphone() {
            Some((_, muted)) => endpoints.set_microphone_muted(!muted),
            None => Ok(()),
        },
        Action::NextDevice => {
            // Список спрашиваем здесь, а не держим наготове: ротация — это
            // нажатие раз в неделю, а перечисление шло бы дважды в секунду
            let current = endpoints.output().map(|(device, _, _)| device.id);
            match next_device(&endpoints.outputs(), current.as_deref()) {
                Some(id) => make_default(switcher, &id),
                None => Ok(()),
            }
        }
        Action::NextInput => {
            // Микрофон один — нажатие ничего не делает: назначать его заново
            // незачем, а «переключилось на то же самое» было бы враньём
            let current = endpoints.microphone().map(|(device, _)| device.id);
            match next_device(&endpoints.inputs(), current.as_deref()) {
                Some(id) if current.as_deref() != Some(id.as_str()) => make_default(switcher, &id),
                _ => Ok(()),
            }
        }
        Action::Device { id } | Action::Input { id } => make_default(switcher, &id),
        Action::SessionVolume { id, percent } => match endpoints.mixer() {
            Some(mixer) => sessions::set_volume(&mixer, &id, percent),
            None => Ok(()),
        },
        Action::SessionMute { id, muted } => match endpoints.mixer() {
            Some(mixer) => sessions::set_muted(&mixer, &id, muted),
            None => Ok(()),
        },
        // Панель разбирается выше и до COM не доходит
        Action::Panel { .. } => Ok(()),
    };

    if let Err(error) = outcome {
        eprintln!("[звук] {error}");
    }
}

#[cfg(windows)]
fn make_default(switcher: Option<&policy::Switcher>, id: &str) -> Result<(), String> {
    match switcher {
        Some(switcher) => switcher.make_default(id),
        None => Err("переключать вывод нечем".to_owned()),
    }
}

#[cfg(not(windows))]
fn work(_app: AppHandle, _inbox: Receiver<Action>) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(id: &str, name: &str) -> Found {
        Found {
            id: id.to_owned(),
            name: name.to_owned(),
            kind: Kind::Other,
        }
    }

    #[test]
    fn short_name_stops_at_the_bracket() {
        assert_eq!(shorten("Динамики (Redmi 电脑音箱)"), "Динамики");
        assert_eq!(shorten("Наушники (8- Arctis Nova 7X)"), "Наушники");
        assert_eq!(shorten("Mi Monitor (NVIDIA High Definition Audio)"), "Mi Monitor");
    }

    /// Скобок нет вовсе — имя остаётся целым.
    #[test]
    fn name_without_brackets_survives() {
        assert_eq!(shorten("Odyssey G65B"), "Odyssey G65B");
    }

    /// Имя, которое начинается со скобки, обрезать нечем: пустая подпись хуже
    /// длинной.
    #[test]
    fn name_starting_with_a_bracket_stays_whole() {
        assert_eq!(shorten(" (NVIDIA High Definition Audio)"), " (NVIDIA High Definition Audio)");
    }

    /// Две панели одной видеокарты дали бы одинаковую подпись — тогда обе
    /// возвращаются к полному имени, а остальные остаются короткими.
    #[test]
    fn clashing_names_go_back_to_full() {
        let devices = label(
            vec![
                found("1", "Динамики (Redmi 电脑音箱)"),
                found("2", "Цифровое аудио (HDMI)"),
                found("3", "Цифровое аудио (DisplayPort)"),
            ],
            Some("1"),
        );

        assert_eq!(devices[0].short, "Динамики");
        assert_eq!(devices[1].short, "Цифровое аудио (HDMI)");
        assert_eq!(devices[2].short, "Цифровое аудио (DisplayPort)");
    }

    #[test]
    fn current_device_is_marked() {
        let devices = label(vec![found("1", "Первое"), found("2", "Второе")], Some("2"));

        assert!(!devices[0].current);
        assert!(devices[1].current);
    }

    #[test]
    fn rotation_goes_round() {
        let devices = vec![found("1", "Первое"), found("2", "Второе"), found("3", "Третье")];

        assert_eq!(next_device(&devices, Some("1")).as_deref(), Some("2"));
        assert_eq!(next_device(&devices, Some("2")).as_deref(), Some("3"));
        // С последнего — на первое
        assert_eq!(next_device(&devices, Some("3")).as_deref(), Some("1"));
    }

    /// Наушники выдернули: текущего в списке нет, но переключать есть куда.
    #[test]
    fn rotation_without_a_current_starts_over() {
        let devices = vec![found("1", "Первое"), found("2", "Второе")];

        assert_eq!(next_device(&devices, Some("нет такого")).as_deref(), Some("1"));
        assert_eq!(next_device(&devices, None).as_deref(), Some("1"));
    }

    #[test]
    fn rotation_of_one_device_stays_put() {
        let devices = vec![found("1", "Единственное")];
        assert_eq!(next_device(&devices, Some("1")).as_deref(), Some("1"));
    }

    #[test]
    fn rotation_of_nothing_gives_nothing() {
        assert_eq!(next_device(&[], Some("1")), None);
    }

    /// Ноль громкости — такая же тишина, как и молчание: значок краснеет и от
    /// того, и от другого.
    #[test]
    fn zero_volume_is_silence() {
        assert!(silent(&Output { volume: 0, muted: false }));
        assert!(silent(&Output { volume: 40, muted: true }));
        assert!(!silent(&Output { volume: 40, muted: false }));
    }

    #[test]
    fn pressing_a_sounding_speaker_mutes_it() {
        assert_eq!(switch(&Output { volume: 40, muted: false }), Switch::Mute);
    }

    /// Звук возвращается на полную, а не на прежнюю громкость: значок нажимают,
    /// чтобы услышать. Откуда взялась тишина — молчание, ноль на полосе или
    /// оба разом, — на это не влияет.
    #[test]
    fn unmuting_always_goes_to_full_volume() {
        for output in [
            Output { volume: 40, muted: true },
            Output { volume: 0, muted: false },
            Output { volume: 0, muted: true },
        ] {
            assert_eq!(switch(&output), Switch::UnmuteTo(100), "{output:?}");
        }
    }

    /// Разметка уходит на фронт как есть: snake_case там уже однажды всплывал.
    #[test]
    fn view_arrives_in_camel_case() {
        let json = serde_json::to_string(&View {
            output: Some(Output { volume: 42, muted: false }),
            microphone: Some(Microphone { muted: true }),
            current: Some(one(&found("1", "Динамики (Redmi)"))),
            devices: label(vec![found("1", "Динамики (Redmi)")], Some("1")),
            inputs: Vec::new(),
            sessions: vec![Track {
                id: "spotify".to_owned(),
                name: "spotify".to_owned(),
                volume: 80,
                muted: false,
                path: Some(r"C:\Spotify.exe".to_owned()),
            }],
            can_switch: true,
            restore: RESTORE_VOLUME,
        })
        .expect("сериализуется");

        assert!(json.contains(r#""canSwitch":true"#), "{json}");
        // Путь к exe нужен только нам — наружу он не уходит
        assert!(!json.contains("Spotify.exe"), "{json}");
        assert!(json.contains(r#""volume":42"#), "{json}");
        assert!(json.contains(r#""short":"Динамики""#), "{json}");
        assert!(json.contains(r#""kind":"other""#), "{json}");
    }

    /// Без устройства виджет обязан показать прочерк, а не ноль громкости.
    #[test]
    fn missing_devices_are_null_not_zero() {
        let json = serde_json::to_string(&View::default()).expect("сериализуется");

        assert_eq!(
            json,
            r#"{"output":null,"microphone":null,"current":null,"devices":[],"inputs":[],"sessions":[],"canSwitch":false,"restore":0}"#
        );
    }

    /// Команды приходят с фронта размеченными по `kind`.
    #[test]
    fn actions_are_tagged_by_kind() {
        let volume: Action =
            serde_json::from_str(r#"{"kind":"volume","percent":70}"#).expect("разбирается");
        assert!(matches!(volume, Action::Volume { percent: 70 }));

        let next: Action = serde_json::from_str(r#"{"kind":"nextDevice"}"#).expect("разбирается");
        assert!(matches!(next, Action::NextDevice));

        let next: Action = serde_json::from_str(r#"{"kind":"nextInput"}"#).expect("разбирается");
        assert!(matches!(next, Action::NextInput));

        // Раскрытая панель — тоже команда: под неё собираются списки
        let panel: Action =
            serde_json::from_str(r#"{"kind":"panel","panel":"mixer"}"#).expect("разбирается");
        assert!(matches!(panel, Action::Panel { panel: Panel::Mixer }));

        let closed: Action =
            serde_json::from_str(r#"{"kind":"panel","panel":"none"}"#).expect("разбирается");
        assert!(matches!(closed, Action::Panel { panel: Panel::None }));

        let input: Action =
            serde_json::from_str(r#"{"kind":"input","id":"микрофон"}"#).expect("разбирается");
        assert!(matches!(input, Action::Input { .. }));

        let loud: Action = serde_json::from_str(r#"{"kind":"sessionVolume","id":"msedge","percent":30}"#)
            .expect("разбирается");
        assert!(matches!(loud, Action::SessionVolume { percent: 30, .. }));

        let hush: Action = serde_json::from_str(r#"{"kind":"sessionMute","id":"msedge","muted":true}"#)
            .expect("разбирается");
        assert!(matches!(hush, Action::SessionMute { muted: true, .. }));

        let pick: Action =
            serde_json::from_str(r#"{"kind":"device","id":"{0.0.0}"}"#).expect("разбирается");
        assert!(matches!(pick, Action::Device { .. }));
    }
}
