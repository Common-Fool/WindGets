//! Список медиаисточников на диске: `media.json`.
//!
//! Хранит известные приложения, которые воспроизводили звук, и переключатель
//! их показа в карточке виджета. Список наполняется автоматически по мере
//! обнаружения медиасессий Windows.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

const FILE: &str = "media.json";

/// Источник медиасессии: приложение, которое хоть раз что-то играло.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Source {
    /// `SourceAppUserModelId` сессии Windows. Он и есть ключ.
    pub id: String,
    /// Чем подписать источник в настройках.
    pub title: String,
    /// Показывать ли его в карточке.
    pub enabled: bool,
    /// Когда видели последний раз, в секундах эпохи.
    pub seen_at: i64,
}

impl Default for Source {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: String::new(),
            enabled: true,
            seen_at: 0,
        }
    }
}

/// Содержимое файла `media.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stored {
    pub sources: Vec<Source>,
}

impl Stored {
    /// Показывать ли источник в карточке.
    ///
    /// Незнакомый — да. Список растёт сам по мере того, как приложения играют,
    /// и заранее знать их имена невозможно; человек гасит лишнее.
    pub fn allows(&self, id: &str) -> bool {
        self.sources
            .iter()
            .find(|source| source.id == id)
            .map(|source| source.enabled)
            .unwrap_or(true)
    }

    /// Запомнить увиденный источник.
    ///
    /// `seen_at` обновляется всегда. Возвращает `true` ТОЛЬКО если список
    /// изменился так, что его стоит записать на диск: источник новый или у него
    /// сменилось имя. Иначе диск переписывался бы на каждом такте опроса, то
    /// есть раз в секунду круглые сутки.
    pub fn remember(&mut self, id: &str, title: &str, now: i64) -> bool {
        let clean = title.trim();
        if let Some(source) = self.sources.iter_mut().find(|source| source.id == id) {
            source.seen_at = now;
            // Пустое имя не затирает уже известное непустое
            if !clean.is_empty() && source.title != clean {
                source.title = clean.to_owned();
                true
            } else {
                false
            }
        } else {
            self.sources.push(Source {
                id: id.to_owned(),
                title: clean.to_owned(),
                enabled: true,
                seen_at: now,
            });
            true
        }
    }

    /// Включить или выключить источник. `false`, если такого нет — новых записей
    /// этот метод не заводит.
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> bool {
        if let Some(source) = self.sources.iter_mut().find(|source| source.id == id) {
            source.enabled = enabled;
            true
        } else {
            false
        }
    }
}

/// Потокобезопасная обёртка вокруг списка источников в памяти.
pub struct Store(Mutex<Stored>);

impl Store {
    pub fn new(sources: Stored) -> Self {
        Self(Mutex::new(sources))
    }

    pub fn get(&self) -> Stored {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Правка под замком: прочитать, поменять и положить обратно одним шагом.
    ///
    /// ВАЖНО: **`get` и `set` врозь тут теряли бы правки.** Список правят двое
    /// — поток медиа, заводящий увиденные источники, и команда настроек,
    /// снимающая галочку, — и снятая между чужими `get` и `set` галочка
    /// вернулась бы на место сама.
    ///
    /// Отдаёт и ответ правки, и копию списка после неё — ту, что пишут на диск.
    pub fn update<R>(&self, change: impl FnOnce(&mut Stored) -> R) -> (R, Stored) {
        let mut stored = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let answer = change(&mut stored);
        (answer, stored.clone())
    }
}

fn file_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("не найден каталог настроек: {error}"))?;
    Ok(dir.join(FILE))
}

/// Читает медиаисточники с диска. При любой беде возвращает пустой список:
/// потерянный файл — это «источников нет», а не повод не подняться.
pub fn load(app: &AppHandle) -> Stored {
    let Ok(path) = file_path(app) else {
        return Stored::default();
    };
    let Ok(text) = fs::read_to_string(&path) else {
        return Stored::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// Атомарно сохраняет медиаисточники на диск.
pub fn save(app: &AppHandle, sources: &Stored) -> Result<(), String> {
    let path = file_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("не создать каталог настроек: {error}"))?;
    }
    let text = serde_json::to_string_pretty(sources)
        .map_err(|error| format!("не сериализовать медиаисточники: {error}"))?;
    crate::disk::write_atomically(&path, &text)
        .map_err(|error| format!("не записать медиаисточники: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Незнакомый источник разрешён по умолчанию: список растёт сам.
    #[test]
    fn an_unknown_source_is_allowed() {
        let stored = Stored::default();
        assert!(stored.allows("com.spotify.client"));
    }

    /// Выключенный пользователем источник карточка показывать не должна.
    #[test]
    fn a_disabled_source_is_not_allowed() {
        let stored = Stored {
            sources: vec![Source {
                id: "com.spotify.client".to_owned(),
                title: "Spotify".to_owned(),
                enabled: false,
                seen_at: 100,
            }],
        };
        assert!(!stored.allows("com.spotify.client"));
        assert!(stored.allows("org.videolan.vlc"), "чужой источник остаётся разрешённым");
    }

    /// Первый раз источник новый — нужно сохранить; повтор с теми же данными
    /// не должен дёргать диск.
    #[test]
    fn remember_adds_a_new_source_and_reports_change_once() {
        let mut stored = Stored::default();
        let changed = stored.remember("com.spotify.client", "Spotify", 100);
        assert!(changed, "первое появление источника обязано сообщать об изменении");
        assert_eq!(stored.sources.len(), 1);
        assert_eq!(stored.sources[0].id, "com.spotify.client");
        assert_eq!(stored.sources[0].title, "Spotify");
        assert!(stored.sources[0].enabled);
        assert_eq!(stored.sources[0].seen_at, 100);

        let repeat = stored.remember("com.spotify.client", "Spotify", 100);
        assert!(!repeat, "повторный вызов с теми же данными не должен сообщать об изменении");
    }

    /// Диск не трогаем, но время последнего появления обновляем на каждом такте.
    #[test]
    fn remember_updates_seen_at_even_without_changes_to_report() {
        let mut stored = Stored::default();
        stored.remember("com.spotify.client", "Spotify", 100);

        let changed = stored.remember("com.spotify.client", "Spotify", 250);
        assert!(!changed, "сообщать об изменении не нужно");
        assert_eq!(stored.sources[0].seen_at, 250, "seen_at обязан обновиться");
    }

    /// Сменившееся имя источника запоминается и запрашивает запись на диск.
    #[test]
    fn a_changed_title_is_remembered_and_reported() {
        let mut stored = Stored::default();
        stored.remember("com.spotify.client", "Spotify", 100);

        let changed = stored.remember("com.spotify.client", "Spotify Music", 200);
        assert!(changed, "смена имени обязана сообщать об изменении");
        assert_eq!(stored.sources[0].title, "Spotify Music");
        assert_eq!(stored.sources[0].seen_at, 200);
    }

    /// Сессия Windows иногда не отдаёт имя — пустота не должна стирать уже известное.
    #[test]
    fn an_empty_title_does_not_overwrite_a_known_one() {
        let mut stored = Stored::default();
        stored.remember("com.spotify.client", "Spotify", 100);

        let changed = stored.remember("com.spotify.client", "", 200);
        assert!(!changed, "пустое имя не повод писать на диск");
        assert_eq!(stored.sources[0].title, "Spotify", "прежнее имя обязано сохраниться");
        assert_eq!(stored.sources[0].seen_at, 200, "seen_at всё равно обновляется");

        let changed_spaces = stored.remember("com.spotify.client", "   ", 300);
        assert!(!changed_spaces, "строка из пробелов также не затирает имя");
        assert_eq!(stored.sources[0].title, "Spotify");
        assert_eq!(stored.sources[0].seen_at, 300);
    }

    /// Запись без поля enabled обязана читаться как включённая (ловушка serde).
    #[test]
    fn a_record_without_an_enabled_field_reads_as_enabled() {
        let json = r#"{"sources":[{"id":"com.spotify.client","title":"Spotify","seenAt":100}]}"#;
        let stored: Stored = serde_json::from_str(json).expect("неполная запись разбирается");
        assert_eq!(stored.sources.len(), 1);
        assert!(
            stored.sources[0].enabled,
            "запись без поля enabled обязана читаться как включённая"
        );
        assert_eq!(stored.sources[0].seen_at, 100);

        let single: Source = serde_json::from_str(r#"{"id":"a","title":"A"}"#).expect("одиночный Source");
        assert!(single.enabled, "Source::default() даёт true для отсутствующего поля");
    }

    #[test]
    fn survives_round_trip() {
        let before = Stored {
            sources: vec![
                Source {
                    id: "com.spotify.client".to_owned(),
                    title: "Spotify".to_owned(),
                    enabled: true,
                    seen_at: 1_700_000_000,
                },
                Source {
                    id: "org.videolan.vlc".to_owned(),
                    title: "VLC media player".to_owned(),
                    enabled: false,
                    seen_at: 1_700_000_500,
                },
            ],
        };
        let text = serde_json::to_string_pretty(&before).expect("сериализуется");
        let after: Stored = serde_json::from_str(&text).expect("читается обратно");
        assert_eq!(before, after);
    }

    /// Переключение несуществующего источника возвращает false и не плодит записей.
    #[test]
    fn set_enabled_returns_false_for_an_unknown_key_and_adds_nothing() {
        let mut stored = Stored::default();
        let changed = stored.set_enabled("unknown.app", false);
        assert!(!changed, "чужой ключ обязан вернуть false");
        assert!(stored.sources.is_empty(), "новых записей заводиться не должно");

        stored.remember("known.app", "Known", 100);
        assert!(stored.allows("known.app"));

        let updated = stored.set_enabled("known.app", false);
        assert!(updated, "существующий ключ обновляется");
        assert!(!stored.allows("known.app"));
    }

    /// Защита от «нули опаснее пропусков»: Default для Source обязан давать enabled: true.
    #[test]
    fn source_default_has_enabled_set_to_true() {
        let default = Source::default();
        assert!(default.enabled, "умолчание руками обязано давать enabled: true");
        assert_eq!(default.id, "");
        assert_eq!(default.title, "");
        assert_eq!(default.seen_at, 0);
    }

    /// Правка под замком отдаёт и свой ответ, и список после неё — тот, что
    /// пишут на диск.
    #[test]
    fn an_update_answers_and_hands_back_the_list() {
        let store = Store::new(Stored::default());
        assert!(store.get().sources.is_empty());

        let (changed, after) = store.update(|stored| stored.remember("app", "App", 100));
        assert!(changed, "новый источник — повод писать");
        assert_eq!(after.sources.len(), 1);
        assert_eq!(store.get(), after, "в памяти то же, что отдано");

        let (found, _) = store.update(|stored| stored.set_enabled("app", false));
        assert!(found);
        assert!(!store.get().allows("app"), "правка осталась в хранилище");
    }
}
