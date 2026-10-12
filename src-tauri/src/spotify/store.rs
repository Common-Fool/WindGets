//! Client ID и refresh token на диске.
//!
//! Лежат отдельно от `settings.json` намеренно. Во-первых, у них разная
//! судьба: «отключиться» — это стереть файл целиком, а настройки вида при
//! этом трогать незачем. Во-вторых, токен обновляется из фонового опроса,
//! и класть его в тот же файл, который переписывает переключатель фона,
//! значит однажды получить гонку на ровном месте.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use super::api::Player;

const FILE: &str = "spotify.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Stored {
    /// Выдаётся в Spotify Developer Dashboard. Не секрет: в потоке с PKCE
    /// он и задуман открытым, секрет клиента нам не нужен вовсе.
    pub client_id: Option<String>,
    /// Появляется после успешной авторизации и живёт до отзыва.
    pub refresh_token: Option<String>,
    /// Последний виденный трек. Пульт показывает его, пока Спотифай молчит,
    /// чтобы виджет не выглядел пустым при каждом запуске.
    pub last_player: Option<Player>,
}

impl Stored {
    pub fn client_id(&self) -> Option<&str> {
        self.client_id.as_deref().filter(|id| !id.is_empty())
    }

    pub fn refresh_token(&self) -> Option<&str> {
        self.refresh_token.as_deref().filter(|token| !token.is_empty())
    }
}

pub struct Store(Mutex<Stored>);

impl Store {
    pub fn new(credentials: Stored) -> Self {
        Self(Mutex::new(credentials))
    }

    pub fn get(&self) -> Stored {
        // Отравленный мьютекс не повод падать: данные внутри осмысленные
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn set(&self, credentials: Stored) {
        *self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = credentials;
    }
}

fn file_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("не найден каталог настроек: {error}"))?;
    Ok(dir.join(FILE))
}

/// Читает учётные данные. Любая беда — считаем, что подключения нет:
/// из-за Spotify приложение стартовать не откажется.
pub fn load(app: &AppHandle) -> Stored {
    let Ok(path) = file_path(app) else {
        return Stored::default();
    };
    let Ok(text) = fs::read_to_string(&path) else {
        return Stored::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save(app: &AppHandle, credentials: &Stored) -> Result<(), String> {
    let path = file_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("не создать каталог настроек: {error}"))?;
    }
    let text = serde_json::to_string_pretty(credentials)
        .map_err(|error| format!("не сериализовать учётные данные: {error}"))?;
    crate::disk::write_atomically(&path, &text)
        .map_err(|error| format!("не записать учётные данные: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Файл могли создать руками или оборвать запись на полуслове —
    /// поля должны просто оказаться пустыми.
    #[test]
    fn missing_fields_read_as_absent() {
        let credentials: Stored = serde_json::from_str("{}").expect("пустой объект читается");
        assert_eq!(credentials.client_id(), None);
        assert_eq!(credentials.refresh_token(), None);
    }

    /// Пустая строка в поле — это «не задано», а не «задано пустым».
    /// Иначе виджет ушёл бы авторизовываться с пустым Client ID.
    #[test]
    fn blank_strings_count_as_absent() {
        let credentials = Stored {
            client_id: Some(String::new()),
            refresh_token: Some(String::new()),
            ..Stored::default()
        };
        assert_eq!(credentials.client_id(), None);
        assert_eq!(credentials.refresh_token(), None);
    }

    #[test]
    fn survives_round_trip() {
        let credentials = Stored {
            client_id: Some("abc".into()),
            refresh_token: Some("def".into()),
            last_player: Some(Player {
                track: "Aerials".to_owned(),
                ..Player::default()
            }),
        };
        let text = serde_json::to_string(&credentials).expect("сериализуется");
        let back: Stored = serde_json::from_str(&text).expect("читается обратно");

        assert_eq!(back.client_id(), Some("abc"));
        assert_eq!(back.refresh_token(), Some("def"));
        assert_eq!(
            back.last_player.map(|player| player.track).as_deref(),
            Some("Aerials"),
            "последний трек не пережил запись на диск"
        );
    }

    /// Файл с прошлой версии знает только про токены — он должен читаться,
    /// а последнего трека в нём просто нет.
    #[test]
    fn file_without_last_track_still_reads() {
        let credentials: Stored =
            serde_json::from_str(r#"{"client_id":"abc","refresh_token":"def"}"#)
                .expect("старый файл читается");

        assert_eq!(credentials.client_id(), Some("abc"));
        assert!(credentials.last_player.is_none());
    }
}
