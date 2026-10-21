//! Медиасессии Windows через WinRT: чтение, кнопки, перемотка и обложка.
//!
//! Единственный файл виджета, который ходит в Windows. Живёт в потоке медиа
//! (`super::work`) и из него не выходит: объекты WinRT, как и COM у звука, не
//! `Send` и принадлежат апартаменту, в котором созданы.
//!
//! Асинхронные вызовы WinRT тут ждутся на месте (`.get()`). Это законно ровно
//! потому, что поток свой: в потоке команды такое ожидание держало бы приём
//! IPC у WebView2.

use std::collections::{HashMap, HashSet};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use windows::core::HSTRING;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as SessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Storage::Streams::{Buffer, DataReader, InputStreamOptions};
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_MULTITHREADED};
use windows::Win32::UI::Shell::{
    IShellItem, SHCreateItemInKnownFolder, FOLDERID_AppsFolder, KF_FLAG_DEFAULT,
    SIGDN_NORMALDISPLAY,
};

use super::{art_key, keep, name_from_id, seconds, unix_millis, Action, Track};

/// Обложка тяжелее этого не берётся вовсе.
///
/// Плееры отдают миниатюры в десятки килобайт; мегабайты значат, что источник
/// сунул в сессию исходник, и гнать его в окно base64 на каждой смене трека
/// незачем — пустая плашка честнее.
const MAX_ART: u64 = 2 * 1024 * 1024;

pub struct Sessions {
    manager: SessionManager,
    /// Имена источников по их ключам. Спрашивать систему на каждом такте
    /// незачем: имя приложения за время работы не меняется.
    names: HashMap<String, String>,
    failures: ReadFailures,
}

impl Sessions {
    /// Заводит апартамент в этом потоке и достаёт менеджер сессий.
    ///
    /// `CoUninitialize` не зовём, как и у звука: поток живёт до выхода.
    pub fn open() -> Result<Self, String> {
        // S_FALSE значит «уже заведён» и отказом не является
        let started = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if started.is_err() {
            return Err(format!("COM не завёлся: {started:?}"));
        }

        let manager = SessionManager::RequestAsync()
            .and_then(|operation| operation.get())
            .map_err(|error| format!("менеджер медиасессий не поднялся: {error}"))?;

        Ok(Self {
            manager,
            names: HashMap::new(),
            failures: ReadFailures::default(),
        })
    }

    /// Все сессии, по одной на источник.
    ///
    /// Сессия, которую не удалось прочитать, пропускается молча: плеер мог
    /// закрыться между перечислением и чтением, и это не беда виджета.
    pub fn read(&mut self, now: i64) -> Vec<Track> {
        let all = match self.manager.GetSessions() {
            Ok(all) => all,
            Err(error) => {
                // Молча — это «ничего не играет» до перезапуска без единой
                // строки в журнале
                eprintln!("[медиа] сессии не перечислились: {error}");
                return Vec::new();
            }
        };
        let count = all.Size().unwrap_or(0);

        let mut tracks = Vec::new();
        for index in 0..count {
            let Ok(session) = all.GetAt(index) else {
                continue;
            };
            let source = match session.SourceAppUserModelId() {
                Ok(source) => source.to_string(),
                Err(error) => {
                    self.report_failure("", &error);
                    continue;
                }
            };
            match self.track(&session, &source, now) {
                Ok(track) => tracks.push(track),
                Err(error) => self.report_failure(&source, &error),
            }
        }

        keep(&tracks).into_iter().map(|at| tracks[at].clone()).collect()
    }

    fn report_failure(&mut self, source: &str, error: &windows::core::Error) {
        let detail = error.to_string();
        if self.failures.first(source, &detail) {
            eprintln!("[медиа] сессия «{source}» не прочиталась: {detail}");
        }
    }

    fn track(&mut self, session: &Session, source: &str, now: i64) -> windows::core::Result<Track> {
        let title = self.name(source);

        let properties = session.TryGetMediaPropertiesAsync()?.get()?;
        let song = properties.Title()?.to_string();
        let artist = properties.Artist()?.to_string();
        let album = properties.AlbumTitle()?.to_string();
        // Ссылки на обложку нет — пустое свойство приходит отказом
        let art = properties
            .Thumbnail()
            .is_ok()
            .then(|| art_key(&song, &artist, &album));

        let info = session.GetPlaybackInfo()?;
        let playing = info.PlaybackStatus()? == Status::Playing;
        let controls = info.Controls()?;

        // Радио и поток могут не иметь полосы вовсе. Отказ её чтения не
        // должен скрывать сам трек и доступные ему кнопки.
        let timeline = (|| -> windows::core::Result<_> {
            let timeline = session.GetTimelineProperties()?;
            Ok((timeline.StartTime()?.Duration, timeline.EndTime()?.Duration,
                timeline.Position()?.Duration, timeline.LastUpdatedTime()?.UniversalTime))
        })();
        let (duration, position, measured_at) = timeline_values(timeline.ok(), now);

        Ok(Track {
            source: source.to_owned(),
            title,
            song,
            artist,
            playing,
            // Одна кнопка на два действия, и разрешение у неё — того, что она
            // сейчас сделает: у играющей — паузы, у стоящей — пуска. «Или»
            // оставляло живой паузу, которую источник запретил
            can_play: if playing { controls.IsPauseEnabled()? } else { controls.IsPlayEnabled()? },
            can_next: controls.IsNextEnabled()?,
            can_previous: controls.IsPreviousEnabled()?,
            can_seek: controls.IsPlaybackPositionEnabled()? && duration > 0.0,
            position,
            duration,
            measured_at,
            art,
        })
    }

    /// Человеческое имя источника: «Zen», а не хеш, которым представляется
    /// Firefox и всё, что на нём собрано.
    ///
    /// Не нашлось в папке приложений — имя выводится из самого ключа.
    fn name(&mut self, source: &str) -> String {
        if let Some(known) = self.names.get(source) {
            return known.clone();
        }

        let found = from_apps_folder(source).unwrap_or_else(|| name_from_id(source));

        self.names.insert(source.to_owned(), found.clone());
        found
    }

    /// Сессия источника — та же, что показана в карточке.
    ///
    /// Правило выбора обязано совпадать с [`keep`]: у Chrome и Edge на одно
    /// приложение бывает по сессии на вкладку, и кнопка обязана попасть в ту,
    /// что на карточке, а не в первую попавшуюся.
    fn find(&self, source: &str) -> Option<Session> {
        let all = self.manager.GetSessions().ok()?;
        let mut first = None;

        for index in 0..all.Size().ok()? {
            let Ok(session) = all.GetAt(index) else {
                continue;
            };
            let mine = session
                .SourceAppUserModelId()
                .map(|id| id == source)
                .unwrap_or(false);
            if !mine {
                continue;
            }

            let playing = session
                .GetPlaybackInfo()
                .and_then(|info| info.PlaybackStatus())
                .map(|status| status == Status::Playing)
                .unwrap_or(false);
            if playing {
                return Some(session);
            }
            first.get_or_insert(session);
        }

        first
    }

    /// Кнопка пульта. Отказ источника — ошибка: кнопка, которая молча ничего
    /// не сделала, читается как поломка виджета, а не плеера.
    pub fn control(&self, source: &str, action: Action) -> Result<(), String> {
        let session = self
            .find(source)
            .ok_or_else(|| "источник больше не играет".to_owned())?;

        let operation = match action {
            Action::Play => session.TryPlayAsync(),
            Action::Pause => session.TryPauseAsync(),
            Action::Next => session.TrySkipNextAsync(),
            Action::Previous => session.TrySkipPreviousAsync(),
        };

        let accepted = operation
            .and_then(|operation| operation.get())
            .map_err(|error| format!("команда не дошла: {error}"))?;

        if accepted {
            Ok(())
        } else {
            Err("источник команду не принял".to_owned())
        }
    }

    /// Перемотка, в секундах от начала трека.
    pub fn seek(&self, source: &str, position: f64) -> Result<(), String> {
        let session = self
            .find(source)
            .ok_or_else(|| "источник больше не играет".to_owned())?;

        // Позиция сессии отсчитывается от её начала, а не от нуля
        let start = session
            .GetTimelineProperties()
            .and_then(|timeline| timeline.StartTime())
            .map(|start| start.Duration)
            .unwrap_or(0);
        let ticks = start + (position.max(0.0) * 10_000_000.0) as i64;

        let accepted = session
            .TryChangePlaybackPositionAsync(ticks)
            .and_then(|operation| operation.get())
            .map_err(|error| format!("перемотка не дошла: {error}"))?;

        if accepted {
            Ok(())
        } else {
            Err("источник перемотку не принял".to_owned())
        }
    }

    /// Обложка источника готовым `data:`-адресом.
    ///
    /// Сессия отдаёт её байтами, а не ссылкой, — отсюда `data:` в CSP. Любая
    /// беда по дороге значит «обложки нет»: карточка покажет пустую плашку.
    pub fn thumbnail(&self, source: &str) -> Option<String> {
        let session = self.find(source)?;
        let properties = session.TryGetMediaPropertiesAsync().ok()?.get().ok()?;
        let stream = properties.Thumbnail().ok()?.OpenReadAsync().ok()?.get().ok()?;

        let size = stream.Size().ok()?;
        if size == 0 || size > MAX_ART {
            return None;
        }
        let size = size as u32;

        let buffer = Buffer::Create(size).ok()?;
        let filled = stream
            .ReadAsync(&buffer, size, InputStreamOptions::None)
            .ok()?
            .get()
            .ok()?;

        let mut bytes = vec![0u8; filled.Length().ok()? as usize];
        DataReader::FromBuffer(&filled).ok()?.ReadBytes(&mut bytes).ok()?;
        if bytes.is_empty() {
            return None;
        }

        // Тип берём у потока, но только картиночный: иначе CSP всё равно не
        // пустит, а угадывать формат по байтам ради редкого случая незачем
        let kind = stream
            .ContentType()
            .map(|kind| kind.to_string())
            .ok()
            .filter(|kind| kind.starts_with("image/"))
            .unwrap_or_else(|| "image/png".to_owned());

        Some(format!("data:{kind};base64,{}", BASE64.encode(&bytes)))
    }
}

/// Имя из папки приложений (`shell:AppsFolder`) — там же его берёт всплывающее
/// окно Windows.
///
/// ВАЖНО: **не `AppInfo`, и это замер, а не вкус.** Первая редакция спрашивала
/// `AppInfo::GetFromAppUserModelId`, и Zen в карточке и настройках вышел
/// ключом `F0DC299D809B9700`: `AppInfo` знает только пакетные приложения из
/// магазина, а Zen — обычная программа. Папка приложений отдала ему «Zen», и
/// пакетные приложения в ней лежат под теми же ключами.
fn from_apps_folder(source: &str) -> Option<String> {
    let key = HSTRING::from(source);
    let item: IShellItem =
        unsafe { SHCreateItemInKnownFolder(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, &key) }.ok()?;

    let raw = unsafe { item.GetDisplayName(SIGDN_NORMALDISPLAY) }.ok()?;
    let name = unsafe { raw.to_string() }.ok();
    // Строку выделила оболочка, и освободить её обязаны мы
    unsafe { CoTaskMemFree(Some(raw.0.cast())) };

    // Имя, совпавшее с ключом, — это тот же хеш, только из другого места
    name.filter(|name| !name.trim().is_empty() && name != source)
}

// Одна строка на источник и причину: неисправная сессия иначе засоряет
// журнал каждую секунду, пряча остальные события приложения.
#[derive(Default)]
struct ReadFailures(HashSet<(String, String)>);

impl ReadFailures {
    fn first(&mut self, source: &str, error: &str) -> bool {
        self.0.insert((source.to_owned(), error.to_owned()))
    }
}

fn timeline_values(timeline: Option<(i64, i64, i64, i64)>, now: i64) -> (f64, f64, i64) {
    let Some((start, end, position, updated)) = timeline else {
        return (0.0, 0.0, now);
    };
    // У некоторых браузеров приходят крайние отрицательные значения:
    // обычное вычитание переполнилось бы ещё до отсечения нуля.
    (seconds(end.saturating_sub(start)).max(0.0),
        seconds(position.saturating_sub(start)).max(0.0), unix_millis(updated, now))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_source_and_error_is_reported_once() {
        let mut failures = ReadFailures::default();
        assert!(failures.first("player-a", "не отвечает"));
        assert!(!failures.first("player-a", "не отвечает"));
        assert!(failures.first("player-b", "не отвечает"));
        assert!(failures.first("player-a", "закрыт"));
        assert!(!failures.first("player-a", "не отвечает"));
    }

    #[test]
    fn missing_timeline_keeps_an_unknown_position_at_now() {
        assert_eq!(timeline_values(None, 1234), (0.0, 0.0, 1234));
    }

    #[test]
    fn timeline_subtraction_handles_extreme_signed_ticks() {
        let (duration, position, measured) = timeline_values(
            Some((i64::MAX, i64::MIN, i64::MIN, 0)), 1234);
        assert_eq!((duration, position, measured), (0.0, 0.0, 1234));
        let (duration, position, _) = timeline_values(
            Some((i64::MIN, i64::MAX, i64::MAX, 0)), 1234);
        assert_eq!(duration, seconds(i64::MAX));
        assert_eq!(position, duration);
        assert_eq!(timeline_values(Some((10_000_000, 40_000_000, 20_000_000, 0)), 1234),
            (3.0, 1.0, 1234));
    }
}
