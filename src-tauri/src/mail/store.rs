//! Ящики почты на диске: `mail.json`.
//!
//! Хранится отдельно от `settings.json` по той же причине, что и `spotify.json`:
//! файл может перезаписываться из фона при проверке или смене настроек,
//! а удаление ящика не должно задевать настройки вида.
//!
//! Ящиков бывает несколько (этап 53, слово пользователя): карточка показывает
//! их вкладками, а в дереве настроек у каждого своя страница. Прежний файл с
//! одной учётной записью читается как первый ящик и тут же переписывается —
//! иначе ключ ящика рождался бы заново на каждом запуске.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

const FILE: &str = "mail.json";

/// `mail.json` есть, но при запуске не прочитался.
///
/// ВАЖНО: **непрочитанный файл не переписывается.** Ящики на диске есть, мы их
/// просто не видели, и первая же правка в настройках положила бы поверх пустой
/// список — пароли пришлось бы заводить заново. Хранилище в памяти одно на
/// процесс, поэтому и признак — статика.
static UNREADABLE: AtomicBool = AtomicBool::new(false);

/// Сколько ящиков можно завести.
///
/// Вкладки карточки — точки, как страницы у заметок и дел, и с точкой «Все»
/// пять ящиков дают те же шесть, что предел страниц там: точки обязаны
/// помещаться в узкое окно.
pub const MAX_ACCOUNTS: usize = 5;

/// Один ящик.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Account {
    /// Ключ ящика: по нему карточка и настройки узнают его между запусками.
    pub id: String,
    /// Версия реквизитов: прежние письма не могут действовать в новом ящике.
    pub revision: String,
    /// Адрес IMAP-сервера (например, `imap.gmail.com`)
    pub host: Option<String>,
    /// Порт IMAP-сервера (по умолчанию 993)
    pub port: Option<u16>,
    /// Имя пользователя или адрес почты
    pub login: Option<String>,
    /// Зашифрованный через DPAPI пароль в base64. Это НЕ открытый пароль!
    pub secret: Option<String>,
    /// Своя ссылка на веб-ящик, если человек её задал.
    ///
    /// Пусто — берётся выведенная из адреса IMAP: `imap.gmail.com` даёт
    /// `https://gmail.com`. Но у ящика бывает своя страница входа, и угадать
    /// её по адресу сервера нельзя никак.
    pub mailbox: Option<String>,
}

impl Account {
    pub fn host(&self) -> Option<&str> {
        self.host.as_deref().filter(|s| !s.trim().is_empty())
    }

    pub fn port(&self) -> Option<u16> {
        self.port.filter(|&p| p > 0)
    }

    pub fn login(&self) -> Option<&str> {
        self.login.as_deref().filter(|s| !s.trim().is_empty())
    }

    pub fn secret(&self) -> Option<&str> {
        self.secret.as_deref().filter(|s| !s.trim().is_empty())
    }

    pub fn mailbox(&self) -> Option<&str> {
        self.mailbox.as_deref().filter(|s| !s.trim().is_empty())
    }

    /// Задан ли ящик полностью (хост, логин и пароль/секрет).
    pub fn is_configured(&self) -> bool {
        self.host().is_some() && self.login().is_some() && self.secret().is_some()
    }

    /// Чем ящик подписан во вкладке и в дереве настроек: логином, а пока его
    /// нет — сервером.
    pub fn address(&self) -> String {
        self.login().or(self.host()).unwrap_or_default().to_owned()
    }
}

/// Содержимое файла `mail.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Stored {
    pub accounts: Vec<Account>,
    /// Показывать ли строку содержимого под темой письма.
    ///
    /// Одна на все ящики: это вид карточки, а не учётная запись. По умолчанию
    /// нет, и это не вкус, а расход: строка требует выборки тела на каждое из
    /// десяти писем в каждом опросе, а отправитель с темой приезжают конвертом
    /// даром.
    pub preview: bool,
}

impl Stored {
    pub fn account(&self, id: &str) -> Option<&Account> {
        self.accounts.iter().find(|account| account.id == id)
    }
}

/// Файл до этапа 53: одна учётная запись плоским объектом.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Single {
    host: Option<String>,
    port: Option<u16>,
    login: Option<String>,
    secret: Option<String>,
    preview: bool,
    mailbox: Option<String>,
}

/// Файл любой из двух редакций.
///
/// ВАЖНО: **редакция выбирается по полю `accounts`, а не перебором.** С
/// `untagged` нынешний файл, у которого одно поле не разобралось (порт
/// строкой «993»), молча принимался за прежний плоский объект без единого
/// ящика — тот лишние поля пропускает, — и первая же запись клала на диск
/// `accounts: []`: все ящики и пароли уходили безвозвратно. Теперь такой
/// файл — неразобравшийся и откладывается в `mail.bad.json`.
enum OnDisk {
    Many {
        accounts: Vec<Account>,
        preview: bool,
    },
    One(Single),
}

#[derive(Deserialize)]
struct Many {
    accounts: Vec<Account>,
    #[serde(default)]
    preview: bool,
}

impl OnDisk {
    fn decode(text: &str) -> Result<Self, serde_json::Error> {
        let value: serde_json::Value = serde_json::from_str(text)?;
        if value.get("accounts").is_some() {
            let Many { accounts, preview } = serde_json::from_value(value)?;
            Ok(Self::Many { accounts, preview })
        } else {
            Ok(Self::One(serde_json::from_value(value)?))
        }
    }

    /// Приводит файл к нынешнему виду. Второе значение — был ли это переезд:
    /// тогда файл надо переписать сразу.
    fn into_stored(self) -> (Stored, bool) {
        match self {
            Self::Many { accounts, preview } => (Stored { accounts, preview }, false),
            Self::One(single) => {
                let account = Account {
                    id: String::new(),
                    revision: String::new(),
                    host: single.host,
                    port: single.port,
                    login: single.login,
                    secret: single.secret,
                    mailbox: single.mailbox,
                };
                // Пустой прежний файл — это «ящиков нет», а не пустой ящик
                let kept = account.host().is_some() || account.login().is_some() || account.secret().is_some();
                let accounts = if kept { vec![account] } else { Vec::new() };
                (
                    Stored {
                        accounts,
                        preview: single.preview,
                    },
                    kept,
                )
            }
        }
    }
}

/// Ключи у всех ящиков есть и не повторяются, ящиков не больше предела.
///
/// Второе значение — пришлось ли что-то чинить: тогда файл переписывается,
/// иначе выданный ключ рождался бы заново на каждом запуске.
fn sane(mut stored: Stored) -> (Stored, bool) {
    let mut changed = false;
    let mut seen: Vec<String> = Vec::new();

    for account in &mut stored.accounts {
        if account.id.trim().is_empty() || seen.contains(&account.id) {
            account.id = crate::totp::store::new_id();
            changed = true;
        }
        seen.push(account.id.clone());
    }

    if stored.accounts.len() > MAX_ACCOUNTS {
        stored.accounts.truncate(MAX_ACCOUNTS);
        changed = true;
    }

    (stored, changed)
}

/// Разбор текста файла — отдельно от чтения ради тестов.
fn parse(text: &str) -> (Stored, bool) {
    let Ok(on_disk) = OnDisk::decode(text) else {
        return (Stored::default(), false);
    };
    let (stored, migrated) = on_disk.into_stored();
    let (stored, repaired) = sane(stored);
    (stored, migrated || repaired)
}

/// Представление ящика для окна настроек.
///
/// Пароль наружу не отдаётся никогда — только флаг `hasPassword`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MailAccount {
    pub host: String,
    pub port: u16,
    pub login: String,
    pub has_password: bool,
    /// Своя ссылка на ящик. Пусто — значит «как выведется само».
    pub mailbox: String,
    /// Что выведется само: подсказка серым в пустом поле настроек.
    pub mailbox_hint: String,
}

impl Default for MailAccount {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 993,
            login: String::new(),
            has_password: false,
            mailbox: String::new(),
            mailbox_hint: String::new(),
        }
    }
}

impl From<&Account> for MailAccount {
    fn from(account: &Account) -> Self {
        Self {
            host: account.host().unwrap_or_default().to_owned(),
            port: account.port().unwrap_or(993),
            login: account.login().unwrap_or_default().to_owned(),
            has_password: account.secret().is_some(),
            mailbox: account.mailbox().unwrap_or_default().to_owned(),
            mailbox_hint: account
                .host()
                .and_then(super::web::mailbox_url)
                .unwrap_or_default(),
        }
    }
}

/// Потокобезопасная обёртка вокруг ящиков в памяти.
pub struct Store(Mutex<Stored>);

impl Store {
    pub fn new(stored: Stored) -> Self {
        Self(Mutex::new(stored))
    }

    pub fn get(&self) -> Stored {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Меняет ящики, не отпуская замка, и отдаёт получившееся.
    ///
    /// Раздельные «взять» и «положить» тут опасны: ящики правят и форма
    /// настроек, и галочка строки содержимого, и правка, уложившаяся между
    /// чтением и записью соседней, пропала бы целиком.
    /// Публикует правку в памяти только после успешной записи, под тем же замком.
    pub fn try_update<T>(
        &self,
        change: impl FnOnce(&mut Stored) -> Result<T, String>,
        persist: impl FnOnce(&Stored) -> Result<(), String>,
    ) -> Result<(Stored, T), String> {
        let mut stored = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if UNREADABLE.load(Ordering::SeqCst) {
            return Err(tr!("mail.error.unreadable"));
        }
        let mut next = stored.clone();
        let answer = change(&mut next)?;
        persist(&next)?;
        *stored = next.clone();
        Ok((next, answer))
    }
}

fn file_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|error| {
            eprintln!("почта: не найден каталог настроек: {error}");
            tr!("common.notSaved")
        })?;
    Ok(dir.join(FILE))
}

/// Читает ящики с диска.
///
/// Файла нет — ящиков нет. Файл не прочитался — ящиков в памяти нет, и правка
/// запрещена до перезапуска (`UNREADABLE`). Файл не разобрался — он отложен
/// рядом (`mail.bad.json`), как у настроек, и ящики заводятся заново.
///
/// Прежний файл с одной записью переписывается нынешним сразу: ключ ящика,
/// выданный при переезде, обязан пережить перезапуск.
pub fn load(app: &AppHandle) -> Stored {
    let Ok(path) = file_path(app) else {
        return Stored::default();
    };
    let text = match crate::disk::read_existing(&path) {
        Ok(Some(text)) => text,
        Ok(None) => return Stored::default(),
        Err(error) => {
            eprintln!("почта: mail.json не прочитался, правка ящиков запрещена до перезапуска: {error}");
            UNREADABLE.store(true, Ordering::SeqCst);
            return Stored::default();
        }
    };

    if OnDisk::decode(&text).is_err() {
        match crate::disk::set_aside(&path) {
            Ok(aside) => eprintln!("почта: mail.json не разобрался, файл отложен в {}", aside.display()),
            Err(error) => {
                eprintln!("почта: mail.json не разобрался и не отложен, правка запрещена: {error}");
                UNREADABLE.store(true, Ordering::SeqCst);
            }
        }
        return Stored::default();
    }

    let (stored, rewrite) = parse(&text);
    if rewrite {
        if let Err(error) = save(app, &stored) {
            eprintln!("почта: переезд mail.json не записался: {error}");
        }
    }
    stored
}

/// Атомарно сохраняет ящики на диск.
pub fn save(app: &AppHandle, stored: &Stored) -> Result<(), String> {
    let path = file_path(app)?;
    save_at(&path, stored)
}

fn save_at(path: &std::path::Path, stored: &Stored) -> Result<(), String> {
    let result = (|| {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("не создать каталог настроек: {error}"))?;
        }
        let text = serde_json::to_string_pretty(stored)
            .map_err(|error| format!("не сериализовать учётные данные: {error}"))?;
        crate::disk::write_atomically(path, &text)
            .map_err(|error| format!("не записать учётные данные: {error}"))
    })();
    result.map_err(|error: String| {
        eprintln!("почта: mail.json не записался: {error}");
        tr!("common.notSaved")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_save_returns_catalog_message_and_preserves_existing_file() {
        let path = std::env::temp_dir().join(format!("windgets-mail-save-{}", crate::totp::store::new_id()));
        fs::write(&path, b"kept").unwrap();
        let result = save_at(&path.join("mail.json"), &Stored::default());
        assert_eq!(result.unwrap_err(), tr!("common.notSaved"));
        assert_eq!(fs::read(&path).unwrap(), b"kept");
        fs::remove_file(path).unwrap();
    }

    fn account(id: &str, login: &str) -> Account {
        Account {
            id: id.to_owned(),
            revision: String::new(),
            host: Some("imap.example.org".into()),
            port: Some(993),
            login: Some(login.into()),
            secret: Some("c2VjcmV0".into()),
            mailbox: None,
        }
    }

    #[test]
    fn failed_persistence_keeps_old_credentials() {
        let before = Stored { accounts: vec![account("one", "old@example.org")], preview: false };
        let store = Store::new(before.clone());
        let changed = |next: &mut Stored| {
            next.accounts[0].login = Some("new@example.org".into());
            Ok(())
        };
        assert!(store.try_update(changed, |_| Err("диск заполнен".into())).is_err());
        assert_eq!(store.get(), before);
        let (saved, ()) = store.try_update(changed, |_| Ok(())).expect("запись прошла");
        assert_eq!(store.get(), saved);
        assert_eq!(saved.accounts[0].login(), Some("new@example.org"));
    }

    #[test]
    fn rejected_change_is_not_persisted() {
        let store = Store::new(Stored::default());
        let result = store.try_update(|next| {
            next.preview = true;
            Err::<(), _>("отказ".into())
        }, |_| panic!("отказавшую правку писать нельзя"));
        assert!(result.is_err());
        assert!(!store.get().preview);
    }

    /// Пустой или оборванный файл — ящиков нет, и это не повод переписывать.
    #[test]
    fn an_empty_file_has_no_accounts() {
        let (stored, rewrite) = parse("{}");
        assert!(stored.accounts.is_empty());
        assert!(!stored.preview);
        assert!(!rewrite);

        let (broken, _) = parse("не json");
        assert!(broken.accounts.is_empty());
    }

    /// Прежний файл с одной записью становится первым ящиком со своим ключом,
    /// строка содержимого остаётся при нём, и файл просится на перезапись.
    #[test]
    fn an_old_single_account_file_becomes_the_first_account() {
        let old = r#"{"host":"imap.gmail.com","port":993,"login":"me@gmail.com","secret":"YmxvYg==","preview":true,"mailbox":null}"#;
        let (stored, rewrite) = parse(old);

        assert!(rewrite, "ключ, выданный при переезде, обязан попасть на диск");
        assert!(stored.preview);
        assert_eq!(stored.accounts.len(), 1);
        let first = &stored.accounts[0];
        assert_eq!(first.id.len(), 16);
        assert_eq!(first.login(), Some("me@gmail.com"));
        assert_eq!(first.secret(), Some("YmxvYg=="));
        assert!(first.is_configured());
    }

    /// Круг записи с двумя ящиками ничего не теряет и ключей не меняет.
    #[test]
    fn two_accounts_survive_the_disk() {
        let stored = Stored {
            accounts: vec![account("aaaa", "one@example.org"), account("bbbb", "two@example.org")],
            preview: true,
        };
        let text = serde_json::to_string(&stored).expect("сериализуется");
        let (back, rewrite) = parse(&text);

        assert_eq!(back, stored);
        assert!(!rewrite);
    }

    /// Правленный руками файл: пустые и повторные ключи получают новые, лишние
    /// ящики сверх предела отбрасываются.
    #[test]
    fn keys_are_unique_and_the_limit_holds() {
        let mut accounts = vec![account("", "a@x"), account("same", "b@x"), account("same", "c@x")];
        accounts.extend((0..5).map(|n| account(&format!("k{n}"), "d@x")));
        let text = serde_json::to_string(&Stored { accounts, preview: false }).expect("сериализуется");

        let (stored, rewrite) = parse(&text);
        assert!(rewrite);
        assert_eq!(stored.accounts.len(), MAX_ACCOUNTS);
        let mut ids: Vec<&str> = stored.accounts.iter().map(|a| a.id.as_str()).collect();
        assert!(ids.iter().all(|id| !id.is_empty()));
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), MAX_ACCOUNTS, "ключи не повторяются");
    }

    /// Пустая строка в поле — это «не задано», а не «задано пустым».
    #[test]
    fn blank_strings_count_as_absent() {
        let blank = Account {
            id: "x".into(),
            revision: String::new(),
            host: Some(String::new()),
            port: Some(0),
            login: Some("   ".to_string()),
            secret: Some(String::new()),
            mailbox: None,
        };
        assert_eq!(blank.host(), None);
        assert_eq!(blank.port(), None);
        assert_eq!(blank.login(), None);
        assert_eq!(blank.secret(), None);
        assert!(!blank.is_configured());
        assert_eq!(blank.address(), "");
    }

    /// `hasPassword` истинен только при непустом секрете.
    #[test]
    fn has_password_only_true_with_secret() {
        let without = Account {
            secret: None,
            ..account("x", "user")
        };
        assert!(!MailAccount::from(&without).has_password);

        let empty = Account {
            secret: Some(String::new()),
            ..without.clone()
        };
        assert!(!MailAccount::from(&empty).has_password);

        assert!(MailAccount::from(&account("x", "user")).has_password);
    }

    /// Новый ящик в форме — пустые строки и порт 993.
    #[test]
    fn empty_account_defaults_to_port_993() {
        let form = MailAccount::from(&Account::default());
        assert_eq!(form.host, "");
        assert_eq!(form.port, 993);
        assert!(!form.has_password);
    }

    /// Форма шлёт ящик без поля hasPassword — он всё равно читается.
    #[test]
    fn config_without_has_password_deserializes() {
        let json = r#"{"host":"imap.test.com","port":993,"login":"user"}"#;
        let form: MailAccount = serde_json::from_str(json).expect("читается без hasPassword");
        assert_eq!(form.login, "user");
        assert!(!form.has_password);
    }

    /// Подпись ящика — логин, а пока его нет — сервер.
    #[test]
    fn an_account_is_named_by_its_login() {
        assert_eq!(account("x", "me@example.org").address(), "me@example.org");
        let no_login = Account {
            login: None,
            ..account("x", "")
        };
        assert_eq!(no_login.address(), "imap.example.org");
    }

    #[test]
    fn a_broken_current_file_is_not_taken_for_an_empty_old_one() {
        let broken = r#"{"accounts":[{"id":"0123456789abcdef","host":"imap.example.com","port":"993"}],"preview":false}"#;
        assert!(OnDisk::decode(broken).is_err(), "порт строкой обязан ронять разбор, а не давать пустой прежний файл");
        let old = r#"{"host":"imap.example.com","port":993}"#;
        assert!(matches!(OnDisk::decode(old), Ok(OnDisk::One(_))));
    }
}
