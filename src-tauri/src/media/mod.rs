//! Медиа: что играет в Windows прямо сейчас — любой плеер, любая вкладка.
//!
//! Источник — медиасессии Windows (`GlobalSystemMediaTransportControls`), те
//! же, из которых собрано всплывающее окно медиа самой системы. Это WinRT, и
//! работа с ним живёт в **отдельном потоке со своим апартаментом**
//! ([`session`]), как COM у звука: объекты WinRT не `Send`, а ждать их из
//! потока команды нельзя — приём IPC у WebView2 идёт главным потоком.
//!
//! В этом файле поэтому нет ни одного вызова Windows: только состояние,
//! команды и правила, которые можно проверить тестом.
//!
//! Опрос по правилу звука: только пока окно видно. Истории виджет не копит, и
//! показанное окно догоняет одним опросом.

#[cfg(windows)]
mod session;
pub mod store;

use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use self::store::{Source, Store, Stored};

/// Метка окна виджета: по ней опрос узнаёт, смотрит ли на него кто-нибудь.
const WIDGET: &str = "media";

/// Событие, которым снимок расходится по окнам — карточке и настройкам.
const MEDIA_EVENT: &str = "media";

/// Шаг опроса. Позиция между опросами досчитывается в окне от `measuredAt`,
/// поэтому чаще спрашивать незачем: полоса едет плавно и так.
const STEP: Duration = Duration::from_secs(1);

/// Разница между началом отсчёта Windows (1601 год) и Unix (1970), в сотнях
/// наносекунд — единицах `DateTime` у WinRT.
const WINDOWS_TO_UNIX: i64 = 116_444_736_000_000_000;

/// Одна медиасессия, как её видит карточка.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    /// `SourceAppUserModelId` — ключ источника и адрес команд.
    pub source: String,
    /// Имя источника для показа: «Zen», «Spotify».
    pub title: String,
    pub song: String,
    pub artist: String,
    pub playing: bool,
    /// Кнопки гасятся по тому, что сессия сообщает сама: веб-плеер на рекламе
    /// «следующий» не принимает, и кнопка, которая ничего не делает, читается
    /// как поломка.
    pub can_play: bool,
    pub can_next: bool,
    pub can_previous: bool,
    pub can_seek: bool,
    /// Секунды от начала трека на момент `measured_at`.
    pub position: f64,
    /// Секунды; ноль — длина неизвестна.
    pub duration: f64,
    /// Когда сессия сняла позицию, в миллисекундах Unix. От этой метки окно
    /// досчитывает позицию само.
    pub measured_at: i64,
    /// Ключ обложки: меняется вместе с треком. `None` — обложки нет.
    pub art: Option<String>,
}

/// Снимок для окон.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    /// Только разрешённые источники.
    pub sessions: Vec<Track>,
    /// Сессия, выбранная руками, — пока она ещё играет.
    pub current: Option<String>,
    /// Всё, что видели, включая выключенное: для окна настроек.
    pub sources: Vec<Source>,
}

/// Кнопка пульта.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    Play,
    Pause,
    Next,
    Previous,
}

/// Что поток медиа должен сделать помимо опроса.
enum Order {
    Control { source: String, action: Action },
    Seek { source: String, position: f64 },
}

pub struct Media {
    store: Store,
    /// Все сессии последнего опроса, ещё до фильтра источников. Фильтр
    /// применяется при каждой рассылке: снятая в настройках галочка обязана
    /// убрать источник сразу, а не на следующем опросе, которого при
    /// спрятанном окне не будет вовсе.
    all: Mutex<Vec<Track>>,
    picked: Mutex<Option<String>>,
    latest: Mutex<View>,
    orders: OnceLock<Sender<Order>>,
    /// Обложки по источникам: ключ трека и готовый `data:`-адрес.
    ///
    /// Лежат тут, а не едут в снимке, по той же причине, что значки программ у
    /// звука: они весят больше всего остального снимка, а меняются только со
    /// сменой трека. Окно спрашивает их командой.
    art: Mutex<BTreeMap<String, (String, String)>>,
}

impl Media {
    pub fn new(stored: Stored) -> Self {
        let latest = compose(&[], None, &stored);
        Self {
            store: Store::new(stored),
            all: Mutex::new(Vec::new()),
            picked: Mutex::new(None),
            latest: Mutex::new(latest),
            orders: OnceLock::new(),
            art: Mutex::new(BTreeMap::new()),
        }
    }

    fn snapshot(&self) -> View {
        // Отравленный мьютекс не повод падать: данные внутри осмысленные
        self.latest
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn send(&self, order: Order) -> Result<(), String> {
        self.orders
            .get()
            .ok_or_else(|| "медиа ещё не завелись".to_owned())?
            .send(order)
            .map_err(|_| "поток медиа не отвечает".to_owned())
    }
}

/// Снимок для окна, открытого между опросами.
#[tauri::command]
pub fn media_state(state: State<'_, Media>) -> View {
    state.snapshot()
}

/// Кладёт кнопку в очередь потока медиа и возвращается сразу: ждать WinRT из
/// потока команды нельзя.
#[tauri::command]
pub fn media_control(state: State<'_, Media>, source: String, action: Action) -> Result<(), String> {
    state.send(Order::Control { source, action })
}

/// Перемотка, в секундах от начала трека.
#[tauri::command]
pub fn media_seek(state: State<'_, Media>, source: String, position: f64) -> Result<(), String> {
    if !position.is_finite() {
        return Err("позиция не число".to_owned());
    }
    state.send(Order::Seek { source, position })
}

/// Какую сессию показывать, когда играют двое разом.
///
/// Потоку медиа не пишет: выбор — это правило рассылки, а не вызов Windows,
/// и снимок пересобирается прямо здесь.
#[tauri::command]
pub fn pick_media_session(app: AppHandle, state: State<'_, Media>, source: String) {
    *state
        .picked
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(source);
    publish(&app, &state);
}

/// Показывать ли источник в карточке. Раздел настроек.
#[tauri::command]
pub fn set_media_source(
    app: AppHandle,
    state: State<'_, Media>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    let (found, stored) = state.store.update(|stored| stored.set_enabled(&id, enabled));
    if !found {
        return Err("такого источника нет в списке".to_owned());
    }

    store::save(&app, &stored)?;
    publish(&app, &state);
    Ok(())
}

/// Обложка источника: `data:`-адрес или `null`.
///
/// Ходит в карту, а не в Windows: обложку туда положил поток медиа, когда
/// заметил новый трек.
#[tauri::command]
pub fn media_art(state: State<'_, Media>, source: String) -> Option<String> {
    state
        .art
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&source)
        .map(|(_, url)| url.clone())
}

/// Поднимает поток медиа. Живёт до конца работы приложения.
pub fn spawn(app: AppHandle) {
    let (orders, inbox) = mpsc::channel();

    if app.state::<Media>().orders.set(orders).is_err() {
        eprintln!("[медиа] поток уже поднят");
        return;
    }

    // Свой поток ОС, а не задача рантайма: апартамент живёт в потоке, и
    // переезд задачи на другой поток пула отобрал бы его
    if let Err(error) = std::thread::Builder::new()
        .name("media".to_owned())
        .spawn(move || work(app, inbox))
    {
        eprintln!("[медиа] поток не поднялся: {error}");
    }
}

/// Смотрит ли кто-нибудь на виджет. Спрятанное окно опрашивать незачем.
fn watched(app: &AppHandle) -> bool {
    app.get_webview_window(WIDGET)
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false)
}

/// Собирает снимок из нынешнего состояния и рассылает, если он поменялся.
fn publish(app: &AppHandle, state: &Media) {
    let next = {
        let all = state.all.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let picked = state
            .picked
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        compose(&all, picked.as_deref(), &state.store.get())
    };

    {
        let mut stored = state
            .latest
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        // Ничего не поменялось — молчим: рассылка раз в секунду во все окна
        // впустую стоит дороже этого сравнения
        if *stored == next {
            return;
        }
        *stored = next.clone();
    }

    // Неудачу рассылки глотаем: окно могли спрятать прямо в этот момент
    let _ = app.emit(MEDIA_EVENT, next);
}

/// Снимок для окон из всех сессий, выбора и списка источников.
///
/// Выключенные источники отсеиваются; выбранная руками сессия остаётся
/// выбранной, только пока она есть среди показанных, — иначе карточка
/// держалась бы за закрытый плеер.
fn compose(all: &[Track], picked: Option<&str>, stored: &Stored) -> View {
    let sessions: Vec<Track> = all
        .iter()
        .filter(|track| stored.allows(&track.source))
        .cloned()
        .collect();

    let current = picked
        .filter(|source| sessions.iter().any(|track| track.source == *source))
        .map(str::to_owned);

    View {
        sessions,
        current,
        sources: stored.sources.clone(),
    }
}

/// Какие сессии оставить: по одной на источник.
///
/// ВАЖНО: **у Chrome и Edge на одно приложение бывает по сессии на вкладку,**
/// и ключ у них один. Карточке и настройкам нужна одна строка на приложение —
/// выключают его тоже целиком, — поэтому из сессий одного источника остаётся
/// играющая, а если не играет ни одна — первая. Тем же правилом кнопки ищут
/// сессию в `session.rs`: нажатие обязано попасть в ту, что на карточке.
///
/// Отдаёт номера в исходном порядке — порядке, в котором их отдала Windows.
fn keep(tracks: &[Track]) -> Vec<usize> {
    let mut chosen: Vec<usize> = Vec::new();

    for (at, track) in tracks.iter().enumerate() {
        match chosen
            .iter()
            .position(|&kept| tracks[kept].source == track.source)
        {
            None => chosen.push(at),
            // Играющая вытесняет стоящую того же источника, но не наоборот
            Some(slot) if track.playing && !tracks[chosen[slot]].playing => chosen[slot] = at,
            Some(_) => {}
        }
    }

    chosen.sort_unstable();
    chosen
}

/// Держит последнюю настоящую полосу трека, когда сессия её стёрла.
///
/// ВАЖНО: **Zen (и Firefox под ним) стирает собственную полосу.** Замерено
/// зондом на YouTube: сессия отдала позицию 17,1 из 53,6 секунды, а через
/// десять секунд у того же ролика — длину 0 и позицию 0, однажды и длину минус
/// девятьсот миллиардов секунд. Поверь карточка последнему снимку, полоса
/// пропадала бы посреди ролика — что живая проверка и поймала, на коротких
/// роликах и на длинных.
///
/// Поэтому снимок без длины у **того же** трека (источник, название,
/// исполнитель) берёт полосу из прошлого снимка. Сменился трек — прошлое
/// забывается: чужая полоса на новом ролике была бы враньём хуже пустой.
///
/// Пока играние не менялось, прошлая точка отсчёта остаётся как есть — окно
/// досчитывает от неё само, и снимок не меняется каждый такт. Сменилось —
/// точка переносится на «сейчас» с позицией, досчитанной по прошлому
/// состоянию: иначе пауза, пролежавшая час, прибавила бы к позиции этот час.
fn steady(track: &mut Track, last: Option<&Track>, now: i64) {
    if track.duration > 0.0 {
        return;
    }
    let Some(last) = last else {
        return;
    };
    let same = last.source == track.source && last.song == track.song && last.artist == track.artist;
    if !same || last.duration <= 0.0 {
        return;
    }

    if last.playing == track.playing {
        track.position = last.position;
        track.measured_at = last.measured_at;
    } else {
        let elapsed = if last.playing {
            (now - last.measured_at).max(0) as f64 / 1000.0
        } else {
            0.0
        };
        track.position = (last.position + elapsed).min(last.duration);
        track.measured_at = now;
    }
    track.duration = last.duration;
    track.can_seek = last.can_seek;
}

/// Переносит полосу источника туда, куда её только что перемотали.
///
/// ВАЖНО: **после перемотки Zen снова стирает полосу**, и [`steady`] честно
/// держал бы последнюю настоящую — то есть ту, что была ДО перемотки. Видео
/// уезжало, а полоса оставалась на месте: поймано живой проверкой. Куда
/// перематывали, мы знаем сами, и это знание точнее любого снимка сессии.
///
/// Сессия, приславшая после перемотки настоящую полосу, её и перебьёт:
/// [`steady`] берёт прошлое только вместо пустого.
fn moved(all: &mut [Track], source: &str, position: f64, now: i64) {
    let Some(track) = all.iter_mut().find(|track| track.source == source) else {
        return;
    };
    if track.duration <= 0.0 {
        return;
    }
    track.position = position.clamp(0.0, track.duration);
    track.measured_at = now;
}

/// Имя источника из его ключа, когда система своего не дала.
///
/// Обычная программа представляется именем exe («Spotify.exe»), иногда с
/// путём; пакетная — `Семейство_хеш!Приложение`. Берётся последний кусок пути
/// без `.exe`, а у пакетной — имя семейства без хеша издателя.
fn name_from_id(id: &str) -> String {
    let tail = id.rsplit(['\\', '/']).next().unwrap_or(id);

    let name = match tail.split_once('!') {
        Some((family, _)) => family.split('_').next().unwrap_or(family),
        None => tail,
    };

    let name = match name.len().checked_sub(4) {
        Some(cut) if name.is_char_boundary(cut) && name[cut..].eq_ignore_ascii_case(".exe") => &name[..cut],
        _ => name,
    };

    if name.is_empty() {
        id.to_owned()
    } else {
        name.to_owned()
    }
}

/// Ключ обложки: меняется вместе с треком.
///
/// Хеш, а не сами строки: он уезжает в окно в каждом снимке, и названия там
/// уже есть. Надёжности тут не нужно никакой — совпадение ключей у двух
/// разных треков значило бы лишь, что обложка обновится на трек позже.
fn art_key(song: &str, artist: &str, album: &str) -> String {
    let mut hasher = DefaultHasher::new();
    (song, artist, album).hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Сотни наносекунд WinRT — в секунды.
fn seconds(ticks: i64) -> f64 {
    ticks as f64 / 10_000_000.0
}

/// Метка времени WinRT — в миллисекунды Unix.
///
/// Нулевая или из будущего метка значит, что источник её не ведёт: такой
/// сессии веры нет, и позиция считается снятой прямо сейчас. Иначе полоса
/// досчитала бы к позиции лишние часы — или годы — и упёрлась бы в конец.
fn unix_millis(universal: i64, now: i64) -> i64 {
    if universal <= WINDOWS_TO_UNIX {
        return now;
    }
    let millis = (universal - WINDOWS_TO_UNIX) / 10_000;
    if millis > now {
        now
    } else {
        millis
    }
}

/// Сейчас, в миллисекундах Unix.
fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(windows)]
fn work(app: AppHandle, inbox: Receiver<Order>) {
    use std::sync::mpsc::RecvTimeoutError;

    let mut sessions = match session::Sessions::open() {
        Ok(sessions) => sessions,
        Err(error) => {
            eprintln!("[медиа] медиасессии недоступны: {error}");
            return;
        }
    };

    loop {
        // Пришла команда — исполняем и тут же перечитываем: карточка обязана
        // догнать нажатие, а не ждать следующего такта
        let mut acted = match inbox.recv_timeout(STEP) {
            Ok(order) => {
                handle(&app, &sessions, order);
                true
            }
            Err(RecvTimeoutError::Timeout) => false,
            // Приложение уходит: держатель отправителя умер вместе с ним
            Err(RecvTimeoutError::Disconnected) => return,
        };

        while let Ok(order) = inbox.try_recv() {
            handle(&app, &sessions, order);
            acted = true;
        }

        if !acted && !watched(&app) {
            continue;
        }

        let state = app.state::<Media>();
        let now = now_millis();
        let mut tracks = sessions.read(now);

        {
            let previous = state.all.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            for track in &mut tracks {
                let last = previous.iter().find(|last| last.source == track.source);
                steady(track, last, now);
            }
        }

        remember(&app, &state, &tracks, now / 1000);
        fetch_art(&sessions, &state, &mut tracks);

        *state.all.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = tracks;
        publish(&app, &state);
    }
}

#[cfg(windows)]
fn handle(app: &AppHandle, sessions: &session::Sessions, order: Order) {
    let done = match order {
        Order::Control { source, action } => sessions.control(&source, action),
        Order::Seek { source, position } => sessions.seek(&source, position).map(|()| {
            let state = app.state::<Media>();
            let mut all = state.all.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            moved(&mut all, &source, position, now_millis());
        }),
    };

    // Отказ пишем в лог и только: карточка увидит, что ничего не поменялось,
    // а кнопки, которые источник не принимает, и так погашены его же флагами
    if let Err(error) = done {
        eprintln!("[медиа] {error}");
    }
}

/// Заводит увиденные источники в список настроек.
///
/// На диск пишется, только когда список действительно поменялся: новый
/// источник или новое имя. Время последней встречи копится в памяти.
#[cfg(windows)]
fn remember(app: &AppHandle, state: &Media, tracks: &[Track], now: i64) {
    let (changed, stored) = state.store.update(|stored| {
        // Циклом, а не `any`: тот остановился бы на первом новом источнике, и
        // остальные попали бы в список только на следующем такте
        let mut changed = false;
        for track in tracks {
            changed |= stored.remember(&track.source, &track.title, now);
        }
        changed
    });

    if changed {
        if let Err(error) = store::save(app, &stored) {
            eprintln!("[медиа] список источников не записан: {error}");
        }
    }
}

/// Снимает обложки для сменившихся треков — до рассылки, чтобы окно,
/// увидев новый ключ, сразу нашло под ним картинку.
///
/// Не далась — ключ снимается с трека: карточка покажет пустую плашку, а
/// следующий опрос попробует снова. Плееры в браузере кладут обложку в сессию
/// позже названия, и одна попытка теряла бы её до смены трека.
#[cfg(windows)]
fn fetch_art(sessions: &session::Sessions, state: &Media, tracks: &mut [Track]) {
    let mut art = state.art.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    for track in tracks.iter_mut() {
        let Some(key) = track.art.clone() else {
            art.remove(&track.source);
            continue;
        };

        let known = art
            .get(&track.source)
            .is_some_and(|(have, _)| *have == key);
        if known {
            continue;
        }

        match sessions.thumbnail(&track.source) {
            Some(url) => {
                art.insert(track.source.clone(), (key, url));
            }
            None => {
                art.remove(&track.source);
                track.art = None;
            }
        }
    }
}

#[cfg(not(windows))]
fn work(_app: AppHandle, _inbox: Receiver<Order>) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(source: &str, playing: bool) -> Track {
        Track {
            source: source.to_owned(),
            title: source.to_owned(),
            song: "Song".to_owned(),
            artist: "Artist".to_owned(),
            playing,
            can_play: true,
            can_next: true,
            can_previous: true,
            can_seek: true,
            position: 12.0,
            duration: 200.0,
            measured_at: 1_700_000_000_000,
            art: None,
        }
    }

    fn stored_with(id: &str, enabled: bool) -> Stored {
        Stored {
            sources: vec![Source {
                id: id.to_owned(),
                title: id.to_owned(),
                enabled,
                seen_at: 1,
            }],
        }
    }

    /// Снимок уезжает в окно, и имена полей там — контракт с `media.ts`.
    #[test]
    fn the_view_speaks_camel_case() {
        let view = compose(&[track("Spotify.exe", true)], None, &Stored::default());
        let json = serde_json::to_string(&view).expect("снимок сериализуется");

        for field in ["canPlay", "canNext", "canPrevious", "canSeek", "measuredAt", "sessions", "current", "sources"] {
            assert!(json.contains(field), "в снимке нет поля {field}: {json}");
        }
        assert!(!json.contains("can_play"), "змеиный регистр уехал в окно");
    }

    #[test]
    fn a_switched_off_source_leaves_the_card() {
        let all = [track("Spotify.exe", true), track("Zen", false)];
        let view = compose(&all, None, &stored_with("Spotify.exe", false));

        assert_eq!(view.sessions.len(), 1);
        assert_eq!(view.sessions[0].source, "Zen");
        assert_eq!(view.sources.len(), 1, "в настройках выключенный остаётся");
    }

    /// Незнакомый источник показывается: список растёт сам.
    #[test]
    fn an_unknown_source_is_shown() {
        let view = compose(&[track("Zen", true)], None, &Stored::default());
        assert_eq!(view.sessions.len(), 1);
    }

    /// Выбор держится за источник, только пока тот на карточке: закрытый или
    /// выключенный плеер не должен оставаться «выбранным» в пустоте.
    #[test]
    fn the_pick_holds_only_while_its_source_is_shown() {
        let all = [track("Spotify.exe", true), track("Zen", false)];

        let kept = compose(&all, Some("Zen"), &Stored::default());
        assert_eq!(kept.current.as_deref(), Some("Zen"));

        let gone = compose(&all, Some("Vlc.exe"), &Stored::default());
        assert_eq!(gone.current, None, "закрытый плеер выбранным не остаётся");

        let hidden = compose(&all, Some("Zen"), &stored_with("Zen", false));
        assert_eq!(hidden.current, None, "выключенный тоже");
    }

    /// Вкладки Chrome приходят сессиями с одним ключом, а строка на карточке
    /// одна — и это играющая вкладка, а не первая по списку.
    #[test]
    fn one_session_per_source_and_the_playing_one_wins() {
        let tracks = [
            track("Chrome", false),
            track("Spotify.exe", false),
            track("Chrome", true),
            track("Chrome", false),
        ];

        let kept = keep(&tracks);
        assert_eq!(kept, vec![1, 2], "Спотифай на месте, из Chrome — играющая вкладка");
    }

    /// Не играет ни одна — остаётся первая: порядок Windows не перемешивается.
    #[test]
    fn with_nothing_playing_the_first_session_stays() {
        let tracks = [track("Chrome", false), track("Chrome", false)];
        assert_eq!(keep(&tracks), vec![0]);
    }

    #[test]
    fn a_name_is_made_out_of_the_key() {
        assert_eq!(name_from_id("Spotify.exe"), "Spotify");
        assert_eq!(name_from_id("C:\\Program Files\\VLC\\vlc.EXE"), "vlc");
        assert_eq!(
            name_from_id("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"),
            "Microsoft.ZuneMusic"
        );
        assert_eq!(name_from_id("Chrome"), "Chrome");
        assert_eq!(name_from_id(".exe"), ".exe", "пустое имя не заменяет ключ");
    }

    #[test]
    fn the_art_key_follows_the_track() {
        let one = art_key("Song", "Artist", "Album");
        assert_eq!(one, art_key("Song", "Artist", "Album"), "тот же трек — тот же ключ");
        assert_ne!(one, art_key("Other", "Artist", "Album"), "новый трек — новый ключ");
        assert_eq!(one.len(), 16);
    }

    #[test]
    fn winrt_ticks_become_seconds() {
        assert!((seconds(10_000_000) - 1.0).abs() < f64::EPSILON);
        assert!((seconds(15_000_000) - 1.5).abs() < f64::EPSILON);
    }

    /// Стёртая полоса того же трека берётся из прошлого снимка.
    #[test]
    fn a_wiped_timeline_keeps_the_last_real_one() {
        let last = track("Zen", true);
        let mut wiped = Track { position: 0.0, duration: 0.0, measured_at: 1_700_000_009_000, can_seek: false, ..track("Zen", true) };

        steady(&mut wiped, Some(&last), 1_700_000_010_000);

        assert!((wiped.duration - 200.0).abs() < f64::EPSILON, "длина вернулась");
        assert!((wiped.position - 12.0).abs() < f64::EPSILON, "позиция та же");
        assert_eq!(wiped.measured_at, last.measured_at, "точка отсчёта прежняя — снимок не меняется");
        assert!(wiped.can_seek, "перемотка вернулась вместе с длиной");
    }

    /// Чужая полоса на новом ролике хуже пустой.
    #[test]
    fn a_new_track_does_not_inherit_the_timeline() {
        let last = track("Zen", true);
        let mut next = Track { song: "Next".to_owned(), position: 0.0, duration: 0.0, ..track("Zen", true) };

        steady(&mut next, Some(&last), 1_700_000_010_000);
        assert!(next.duration <= 0.0, "длина осталась неизвестной");
    }

    /// Настоящая свежая полоса главнее прошлой.
    #[test]
    fn a_real_timeline_wins() {
        let last = track("Zen", true);
        let mut fresh = Track { position: 40.0, duration: 180.0, ..track("Zen", true) };

        steady(&mut fresh, Some(&last), 1_700_000_010_000);
        assert!((fresh.position - 40.0).abs() < f64::EPSILON);
        assert!((fresh.duration - 180.0).abs() < f64::EPSILON);
    }

    /// Пауза, стёршая полосу, замораживает позицию там, где игла была к этому
    /// мгновению, а не там, где её сняли в прошлый раз.
    #[test]
    fn a_pause_freezes_where_the_needle_got_to() {
        // Играл: 12 секунд на отметке measured_at
        let last = track("Zen", true);
        let now = last.measured_at + 5_000;
        let mut paused = Track { position: 0.0, duration: 0.0, ..track("Zen", false) };

        steady(&mut paused, Some(&last), now);

        assert!((paused.position - 17.0).abs() < 1e-9, "12 плюс пять секунд игры");
        assert_eq!(paused.measured_at, now, "отсчёт перенесён на сейчас");
    }

    /// Возобновление после долгой паузы не прибавляет эту паузу к позиции.
    #[test]
    fn a_resume_does_not_add_the_pause() {
        let last = Track { measured_at: 1_700_000_000_000, ..track("Zen", false) };
        let now = last.measured_at + 3_600_000;
        let mut resumed = Track { position: 0.0, duration: 0.0, ..track("Zen", true) };

        steady(&mut resumed, Some(&last), now);

        assert!((resumed.position - 12.0).abs() < f64::EPSILON, "позиция стоит, где стояла");
        assert_eq!(resumed.measured_at, now);
    }

    /// Перемотанная полоса встаёт туда, куда перематывали, и отсчёт идёт
    /// оттуда — даже если сессия потом снова её сотрёт.
    #[test]
    fn a_seek_moves_the_kept_timeline() {
        let mut all = vec![track("Spotify.exe", true), track("Zen", true)];
        let now = 1_700_000_050_000;

        moved(&mut all, "Zen", 150.0, now);
        assert!((all[1].position - 150.0).abs() < f64::EPSILON);
        assert_eq!(all[1].measured_at, now);
        assert!((all[0].position - 12.0).abs() < f64::EPSILON, "чужой источник не тронут");

        // Сессия стёрла полосу — держится новая позиция, а не прежняя
        let mut wiped = Track { position: 0.0, duration: 0.0, ..track("Zen", true) };
        steady(&mut wiped, Some(&all[1]), now + 1_000);
        assert!((wiped.position - 150.0).abs() < f64::EPSILON);
    }

    /// За край не перематывают, а без длины перематывать некуда.
    #[test]
    fn a_seek_stays_inside_the_track() {
        let mut all = vec![track("Zen", true)];
        moved(&mut all, "Zen", 999.0, 1);
        assert!((all[0].position - 200.0).abs() < f64::EPSILON);

        let mut unknown = vec![Track { duration: 0.0, ..track("Zen", true) }];
        moved(&mut unknown, "Zen", 50.0, 1);
        assert!((unknown[0].position - 12.0).abs() < f64::EPSILON, "без длины не трогаем");
    }

    /// Метка WinRT — сотни наносекунд от 1601 года.
    #[test]
    fn a_winrt_moment_becomes_unix_millis() {
        let now = 1_800_000_000_000;
        // 1 января 2024, 00:00:00 UTC
        let moment = WINDOWS_TO_UNIX + 1_704_067_200_000 * 10_000;
        assert_eq!(unix_millis(moment, now), 1_704_067_200_000);
    }

    /// Источник, не ведущий метку, досчитывал бы к позиции годы.
    #[test]
    fn a_missing_or_future_moment_means_now() {
        let now = 1_700_000_000_000;
        assert_eq!(unix_millis(0, now), now, "нулевая метка");
        let ahead = WINDOWS_TO_UNIX + (now + 60_000) * 10_000;
        assert_eq!(unix_millis(ahead, now), now, "метка из будущего");
    }
}
