//! Записи 2FA на диске: `totp.json`.
//!
//! Отдельным файлом, а не полем в `settings.json`, по той же причине, что у
//! почты и Спотифая: своя судьба у «удалить запись» и своя запись из раздела
//! настроек — стирание одного не должно задевать настройки вида.
//!
//! ВАЖНО: **секрет наружу не уходит ни одним путём.** На диске лежит base64 от
//! блоба DPAPI, в окно уезжает [`Row`], где поля секрета нет вовсе, а `Debug` у
//! [`Account`] написан руками и печатает заглушку: `eprintln!("{account:?}")`,
//! поставленный когда-нибудь в отладке, не должен положить ключ в лог.

use std::fmt;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use super::code::Shape;

const FILE: &str = "totp.json";

/// Одна заведённая запись.
///
/// Наружу не сериализуется никогда — для окна есть [`Row`]. Serialize тут
/// только ради диска.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Account {
    /// Свой непрозрачный ключ. Издатель с ярлыком в этой роли не годятся: две
    /// записи одного сервиса законны, а ярлык переименовывают.
    pub id: String,
    pub issuer: String,
    pub label: String,
    /// Зашифрованный через DPAPI секрет в base64. Это НЕ открытый секрет!
    pub secret: String,
    pub digits: u8,
    pub period: u32,
    pub algorithm: super::code::Algorithm,
    /// Когда запись завели, в секундах эпохи. Держит порядок списка и служит
    /// единственной проверкой часов — см. `mod.rs`.
    pub added_at: i64,
}

impl Default for Account {
    fn default() -> Self {
        let shape = Shape::default();
        Self {
            id: String::new(),
            issuer: String::new(),
            label: String::new(),
            secret: String::new(),
            digits: shape.digits,
            period: shape.period,
            algorithm: shape.algorithm,
            added_at: 0,
        }
    }
}

/// Печать без секрета. Руками, а не `derive`, и это защита, а не украшение.
impl fmt::Debug for Account {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.debug_struct("Account")
            .field("id", &self.id)
            .field("issuer", &self.issuer)
            .field("label", &self.label)
            .field("secret", &"<скрыт>")
            .field("digits", &self.digits)
            .field("period", &self.period)
            .field("algorithm", &self.algorithm)
            .field("added_at", &self.added_at)
            .finish()
    }
}

impl Account {
    /// Форма кода этой записи, приведённая в диапазон.
    ///
    /// Именно тут, а не при чтении файла: правленный руками `period: 0` иначе
    /// дошёл бы до деления.
    pub fn shape(&self) -> Shape {
        Shape {
            digits: self.digits,
            period: self.period,
            algorithm: self.algorithm,
        }
        .sane()
    }

    /// Строка для окна. Секрета в ней нет и быть не может.
    pub fn row(&self) -> Row {
        let shape = self.shape();
        Row {
            id: self.id.clone(),
            issuer: self.issuer.clone(),
            label: self.label.clone(),
            digits: shape.digits,
            period: shape.period,
        }
    }
}

/// Запись глазами окна: чем подписать строку и какой формы ждать код.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub id: String,
    pub issuer: String,
    pub label: String,
    pub digits: u8,
    pub period: u32,
}

/// Содержимое файла `totp.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stored {
    pub accounts: Vec<Account>,
}

/// Потокобезопасная обёртка вокруг записей в памяти.
pub struct Store(Mutex<Stored>);

impl Store {
    pub fn new(accounts: Stored) -> Self {
        Self(Mutex::new(accounts))
    }

    pub fn get(&self) -> Stored {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn set(&self, accounts: Stored) {
        *self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = accounts;
    }
}

fn file_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("не найден каталог настроек: {error}"))?;
    Ok(dir.join(FILE))
}

/// Что пошло не так при чтении файла записей.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trouble {
    /// Файл есть, но не прочитался: занят, нет доступа. Записей мы не видели,
    /// и писать поверх нельзя — первая же правка затёрла бы их пустым списком.
    Unreadable,
    /// Файл прочитался, но не разобрался. Он отложен рядом (`totp.bad.json`):
    /// секреты в нём под DPAPI, и вернуть их можно руками.
    SetAside,
}

/// Прочитанное с диска и то, что при этом случилось.
#[derive(Debug, Default)]
pub struct Loaded {
    pub stored: Stored,
    pub trouble: Option<Trouble>,
}

/// Читает записи с диска.
///
/// ВАЖНО: **«файла нет» и «файл не прочитался» — разные исходы.** Прежде оба
/// давали пустой список, и следующая же новая запись ложилась на диск поверх
/// прежнего файла — то есть временный отказ чтения стирал все секреты
/// навсегда. Теперь пусто только там, где файла правда нет.
pub fn load(app: &AppHandle) -> Loaded {
    let path = match file_path(app) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("2FA: {error}");
            return Loaded { stored: Stored::default(), trouble: Some(Trouble::Unreadable) };
        }
    };

    let text = match crate::disk::read_existing(&path) {
        Ok(Some(text)) => text,
        Ok(None) => return Loaded::default(),
        Err(error) => {
            eprintln!("2FA: totp.json не прочитался, запись запрещена до повторного чтения: {error}");
            return Loaded { stored: Stored::default(), trouble: Some(Trouble::Unreadable) };
        }
    };

    match parse(&text) {
        Some(stored) => Loaded { stored, trouble: None },
        None => {
            match crate::disk::set_aside(&path) {
                Ok(aside) => eprintln!("2FA: totp.json не разобрался, файл отложен в {}", aside.display()),
                Err(error) => {
                    // Не отложился — значит, запись поверх его бы затёрла:
                    // держим хранилище запертым, как непрочитанное
                    eprintln!("2FA: totp.json не разобрался и не отложен: {error}");
                    return Loaded { stored: Stored::default(), trouble: Some(Trouble::Unreadable) };
                }
            }
            Loaded { stored: Stored::default(), trouble: Some(Trouble::SetAside) }
        }
    }
}

/// Разбор текста файла; `None` — файл испорчен.
fn parse(text: &str) -> Option<Stored> {
    serde_json::from_str(text).ok()
}

/// Атомарно сохраняет записи на диск. Подробность отказа — в журнал (она
/// русская и с текстом Windows), в окно — строка каталога.
pub fn save(app: &AppHandle, accounts: &Stored) -> Result<(), String> {
    write(app, accounts).map_err(|error| {
        eprintln!("[2FA] записи не сохранились: {error}");
        tr!("totp.error.save")
    })
}

fn write(app: &AppHandle, accounts: &Stored) -> Result<(), String> {
    let path = file_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("не создать каталог настроек: {error}"))?;
    }
    let text = serde_json::to_string_pretty(accounts)
        .map_err(|error| format!("не сериализовать записи 2FA: {error}"))?;
    crate::disk::write_atomically(&path, &text)
        .map_err(|error| format!("не записать записи 2FA: {error}"))
}

/// Новый непрозрачный ключ записи: шестнадцать шестнадцатеричных знаков.
///
/// Случайность криптостойкая — тот же `getrandom`, что у PKCE Спотифая. Ключ
/// секретом не является, но предсказуемый счётчик однажды столкнул бы две
/// записи, заведённые в одну секунду.
pub fn new_id() -> String {
    let mut bytes = [0u8; 8];
    // Отказ источника случайности на Windows означает беду куда крупнее нашей
    getrandom::fill(&mut bytes).expect("источник случайности недоступен");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::super::code::Algorithm;
    use super::*;

    fn sample() -> Account {
        Account {
            id: "a1b2c3d4e5f60718".to_owned(),
            issuer: "GitHub".to_owned(),
            label: "demo-user".to_owned(),
            secret: "0J/RgNC40LLQtdGCLdCx0LvQvtCx".to_owned(),
            digits: 6,
            period: 30,
            algorithm: Algorithm::Sha1,
            added_at: 1_700_000_000,
        }
    }

    /// Инвариант безопасности в виде теста: в том, что уезжает в окно, секрета
    /// нет ни полем, ни строкой.
    #[test]
    fn the_secret_never_reaches_the_window() {
        let account = sample();
        let json = serde_json::to_string(&account.row()).expect("строка сериализуется");

        assert!(!json.contains("secret"), "в строке окна осталось поле секрета");
        assert!(
            !json.contains(&account.secret),
            "в строке окна оказалось значение секрета"
        );
        assert!(json.contains("GitHub"), "издатель обязан доезжать");
    }

    /// `eprintln!("{account:?}")` в отладке не должен положить ключ в лог.
    #[test]
    fn the_debug_print_hides_the_secret() {
        let account = sample();
        let printed = format!("{account:?}");

        assert!(
            !printed.contains(&account.secret),
            "Debug напечатал секрет: {printed}"
        );
        assert!(printed.contains("<скрыт>"), "заглушка на месте");
        assert!(printed.contains("GitHub"), "остальное печатается как было");
    }

    /// Файл могли создать пустым или оборвать запись.
    #[test]
    fn missing_fields_read_as_absent() {
        let empty: Stored = serde_json::from_str("{}").expect("пустой объект");
        assert!(empty.accounts.is_empty());

        let partial: Stored =
            serde_json::from_str(r#"{"accounts":[{"issuer":"A"}]}"#).expect("неполная запись");
        assert_eq!(partial.accounts.len(), 1);
        assert_eq!(partial.accounts[0].issuer, "A");
        assert_eq!(partial.accounts[0].digits, 6, "умолчание формы на месте");
    }

    #[test]
    fn survives_round_trip() {
        let before = Stored {
            accounts: vec![sample()],
        };
        let text = serde_json::to_string(&before).expect("сериализуется");
        let after: Stored = serde_json::from_str(&text).expect("читается обратно");
        assert_eq!(before, after);
    }

    /// Правленный руками файл не должен доходить до деления на ноль.
    #[test]
    fn a_hand_edited_file_is_brought_back_into_range() {
        let wild = Account {
            digits: 99,
            period: 0,
            ..sample()
        };

        let shape = wild.shape();
        assert_eq!(shape.digits, 8);
        assert_eq!(shape.period, 30);
        assert_eq!(wild.row().digits, 8, "в окно тоже уезжает приведённое");
    }

    /// Две записи одного сервиса законны: рабочая почта и личная.
    #[test]
    fn two_accounts_of_one_service_live_side_by_side() {
        let one = sample();
        let two = Account {
            id: "ffffffffffffffff".to_owned(),
            label: "demo-user-work".to_owned(),
            ..sample()
        };

        let rows = [one.row(), two.row()];
        assert_ne!(rows[0].id, rows[1].id, "ключи обязаны различаться");
        assert_eq!(rows[0].issuer, rows[1].issuer, "издатель при этом один");
    }

    /// Ключ случаен и не повторяется.
    #[test]
    fn a_new_id_is_sixteen_hex_signs() {
        let id = new_id();
        assert_eq!(id.len(), 16);
        assert!(id.chars().all(|sign| sign.is_ascii_hexdigit()));
        assert_ne!(id, new_id(), "два ключа подряд не совпадают");
    }

    /// Испорченный файл отличается от целого: иначе он молча стал бы пустым
    /// списком, и первая новая запись затёрла бы все секреты.
    #[test]
    fn a_broken_file_is_told_apart_from_a_whole_one() {
        assert!(parse("не json").is_none());
        assert!(parse(r#"{"accounts":"много"}"#).is_none());
        assert!(parse("{}").is_some_and(|stored| stored.accounts.is_empty()));
        assert!(parse(r#"{"accounts":[]}"#).is_some());
    }
}
