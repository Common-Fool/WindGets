//! Client ID и refresh token на диске.
//!
//! Лежат отдельно от `settings.json` намеренно. Во-первых, у них разная
//! судьба: «отключиться» — это стереть токен, а настройки вида при этом
//! трогать незачем. Во-вторых, токен обновляется из фонового опроса, и класть
//! его в тот же файл, который переписывает переключатель фона, значит однажды
//! получить гонку на ровном месте.
//!
//! ВАЖНО: **refresh token на диске — только под DPAPI** текущего пользователя
//! (`refresh_token_protected`), как пароль почты: он живёт до отзыва и даёт
//! управлять чужим аккаунтом Spotify. Прежний открытый `refresh_token`
//! читается ровно один раз — для переезда — и больше не пишется. Client ID в
//! PKCE не секрет и остаётся открытым.
//!
//! Правка и запись идут под одним замком (`Store::try_update`), а память
//! принимает только успешно записанное: два писателя из разных потоков иначе
//! клали бы на диск устаревший снимок поверх свежего.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use super::api::Player;

const FILE: &str = "spotify.json";

/// Рабочие данные в памяти. Этот тип намеренно не сериализуется напрямую.
#[derive(Clone, Default)]
pub struct Stored {
    /// Выдаётся в Spotify Developer Dashboard. Не секрет: в потоке с PKCE
    /// он и задуман открытым, секрет клиента нам не нужен вовсе.
    pub client_id: Option<String>,
    /// Появляется после успешной авторизации и живёт до отзыва. В памяти —
    /// открытым, на диске — только под DPAPI.
    pub refresh_token: Option<String>,
    /// Последний виденный трек. Пульт показывает его, пока Спотифай молчит,
    /// чтобы виджет не выглядел пустым при каждом запуске.
    pub last_player: Option<Player>,
}

impl fmt::Debug for Stored {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Stored")
            .field("client_id", &self.client_id)
            .field("refresh_token", &"<скрыто>")
            .field("last_player", &self.last_player)
            .finish()
    }
}

/// Формат файла отделён от памяти: старое поле читаем только для миграции.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct OnDisk {
    client_id: Option<String>,
    #[serde(skip_serializing)]
    refresh_token: Option<String>,
    refresh_token_protected: Option<String>,
    last_player: Option<Player>,
}

impl Stored {
    pub fn client_id(&self) -> Option<&str> {
        self.client_id.as_deref().filter(|id| !id.is_empty())
    }

    pub fn refresh_token(&self) -> Option<&str> {
        self.refresh_token.as_deref().filter(|token| !token.is_empty())
    }

    /// Ответ сети относится к этим же реквизитам, а не к новому подключению.
    pub fn same_connection(&self, other: &Stored) -> bool {
        self.client_id() == other.client_id() && self.refresh_token() == other.refresh_token()
    }
}

pub struct Store(Mutex<Stored>);

impl Store {
    pub fn new(credentials: Stored) -> Self {
        Self(Mutex::new(credentials))
    }

    pub fn get(&self) -> Stored {
        self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    /// Меняем свежий снимок и пишем, не отпуская замка. Отказ не меняет память.
    pub fn try_update<T>(
        &self,
        change: impl FnOnce(&mut Stored) -> Result<T, String>,
        persist: impl FnOnce(&Stored) -> Result<(), String>,
    ) -> Result<(Stored, T), String> {
        let mut stored = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut next = stored.clone();
        let answer = change(&mut next)?;
        persist(&next)?;
        *stored = next.clone();
        Ok((next, answer))
    }
}

fn file_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir()
        .map_err(|error| format!("не найден каталог настроек: {error}"))?;
    Ok(dir.join(FILE))
}

fn parse(text: &str) -> Result<(Stored, bool), String> {
    let disk: OnDisk = serde_json::from_str(text)
        .map_err(|_| "не разобрать spotify.json".to_owned())?;
    let (refresh_token, rewrite) = match disk.refresh_token_protected {
        Some(protected) => match crate::dpapi::unprotect(&protected) {
            Ok(token) => (Some(token), disk.refresh_token.is_some()),
            Err(_) => {
                // Не печатаем ни блоб, ни открытый токен, ни содержимое файла.
                eprintln!("Spotify: токен не расшифровался; войдите заново, spotify.json сохранён без изменений");
                (None, false)
            }
        },
        None => {
            let rewrite = disk.refresh_token.is_some();
            (disk.refresh_token, rewrite)
        }
    };
    Ok((Stored { client_id: disk.client_id, refresh_token, last_player: disk.last_player }, rewrite))
}

fn encode(credentials: &Stored) -> Result<String, String> {
    let refresh_token_protected = credentials.refresh_token().map(crate::dpapi::protect).transpose()?;
    let disk = OnDisk {
        client_id: credentials.client_id.clone(),
        refresh_token: None,
        refresh_token_protected,
        last_player: credentials.last_player.clone(),
    };
    serde_json::to_string_pretty(&disk).map_err(|error| format!("не сериализовать учётные данные: {error}"))
}

/// Ошибка DPAPI отключает подключение, но не переписывает исходный файл.
pub fn load(app: &AppHandle) -> Stored {
    match file_path(app) {
        Ok(path) => load_from_path(&path),
        Err(error) => {
            eprintln!("Spotify: {error}");
            Stored::default()
        }
    }
}

fn load_from_path(path: &Path) -> Stored {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!("Spotify: не прочитать spotify.json: {error}");
            }
            return Stored::default();
        }
    };
    let (stored, rewrite) = match parse(&text) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("Spotify: {error}; файл сохранён без изменений");
            return Stored::default();
        }
    };
    if rewrite {
        if let Err(error) = save_to_path(path, &stored) {
            eprintln!("Spotify: не записать миграцию токена под DPAPI: {error}");
        }
    }
    stored
}

/// Вызывается под замком Store; при начальной миграции Store ещё не создан.
pub fn save(app: &AppHandle, credentials: &Stored) -> Result<(), String> {
    save_to_path(&file_path(app)?, credentials)
}

fn save_to_path(path: &Path, credentials: &Stored) -> Result<(), String> {
    let text = encode(credentials)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("не создать каталог настроек: {error}"))?;
    }
    crate::disk::write_atomically(path, &text).map_err(|error| format!("не записать учётные данные: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credentials() -> Stored {
        Stored {
            client_id: Some("sample-client".into()),
            refresh_token: Some("sample-refresh-token".into()),
            last_player: Some(Player { track: "Sample track".into(), ..Player::default() }),
        }
    }

    fn test_path() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir().join(format!("windgets-spotify-test-{}-{id}.json", std::process::id()))
    }

    #[test]
    fn missing_fields_read_as_absent() {
        let (stored, rewrite) = parse("{}").expect("пустой объект читается");
        assert_eq!(stored.client_id(), None);
        assert_eq!(stored.refresh_token(), None);
        assert!(!rewrite);
        assert!(parse("не json").is_err());
    }

    #[test]
    fn blank_strings_count_as_absent() {
        let stored = Stored { client_id: Some(String::new()), refresh_token: Some(String::new()), ..Stored::default() };
        assert_eq!(stored.client_id(), None);
        assert_eq!(stored.refresh_token(), None);
    }

    #[test]
    fn protected_token_survives_round_trip_without_plaintext_on_disk() {
        let stored = credentials();
        let text = encode(&stored).expect("шифруется и сериализуется");
        assert!(!text.contains("sample-refresh-token"));
        let disk: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(disk.get("refresh_token").is_none());
        assert!(disk["refresh_token_protected"].is_string());
        assert_eq!(disk["client_id"], "sample-client");
        let (back, rewrite) = parse(&text).expect("читается обратно");
        assert!(!rewrite);
        assert!(back.same_connection(&stored));
        assert_eq!(back.last_player.unwrap().track, "Sample track");
    }

    #[test]
    fn legacy_file_is_immediately_migrated_and_survives_a_reload() {
        let path = test_path();
        fs::write(&path, r#"{"client_id":"sample-client","refresh_token":"sample-refresh-token"}"#).unwrap();
        let stored = load_from_path(&path);
        assert!(stored.same_connection(&credentials()));
        assert!(stored.last_player.is_none());
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("sample-refresh-token"));
        assert!(serde_json::from_str::<serde_json::Value>(&text).unwrap().get("refresh_token").is_none());
        assert!(load_from_path(&path).same_connection(&stored));
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn unreadable_protected_token_does_not_fall_back_or_overwrite_the_file() {
        let path = test_path();
        let text = r#"{"client_id":"sample-client","refresh_token":"legacy-must-not-be-used","refresh_token_protected":"broken!"}"#;
        fs::write(&path, text).unwrap();
        let stored = load_from_path(&path);
        assert_eq!(stored.client_id(), Some("sample-client"));
        assert_eq!(stored.refresh_token(), None);
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn debug_hides_the_token() {
        let text = format!("{:?}", credentials());
        assert!(text.contains("<скрыто>"));
        assert!(!text.contains("sample-refresh-token"));
    }

    #[test]
    fn failed_persistence_keeps_the_previous_state() {
        let store = Store::new(credentials());
        assert!(store.try_update(|next| { next.refresh_token = None; Ok(()) }, |_| Err("диск заполнен".into())).is_err());
        assert!(store.get().same_connection(&credentials()));
        let (written, ()) = store.try_update(|next| { next.refresh_token = None; Ok(()) }, |_| Ok(())).unwrap();
        assert_eq!(written.refresh_token(), None);
        assert_eq!(store.get().refresh_token(), None);
    }

    #[test]
    fn rejected_change_is_not_persisted() {
        let store = Store::new(credentials());
        let result = store.try_update(|next| { next.client_id = None; Err::<(), _>("отказ".into()) }, |_| panic!("отказавшую правку писать нельзя"));
        assert!(result.is_err());
        assert!(store.get().same_connection(&credentials()));
    }

    #[test]
    fn parallel_writers_keep_both_changes_and_publish_only_what_was_written() {
        let store = Store::new(credentials());
        let disk = Mutex::new(Stored::default());
        let start = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                start.wait();
                store.try_update(|next| { next.refresh_token = Some("rotated-token".into()); Ok(()) }, |next| { *disk.lock().unwrap() = next.clone(); Ok(()) }).unwrap();
            });
            scope.spawn(|| {
                start.wait();
                store.try_update(|next| { next.last_player = Some(Player { track: "Next track".into(), ..Player::default() }); Ok(()) }, |next| { *disk.lock().unwrap() = next.clone(); Ok(()) }).unwrap();
            });
        });
        let stored = store.get();
        let written = disk.lock().unwrap();
        assert_eq!(stored.refresh_token(), Some("rotated-token"));
        assert_eq!(stored.last_player.as_ref().unwrap().track, "Next track");
        assert!(stored.same_connection(&written));
        assert_eq!(stored.last_player.unwrap().track, written.last_player.as_ref().unwrap().track);
    }
}
