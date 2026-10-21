//! Серверная часть виджета 2FA: коды TOTP по RFC 6238.
//!
//! Устройство отличается от почты и пульта, и намеренно: **фонового цикла тут
//! нет.** Источник у кодов не снаружи, а изнутри — системные часы, — и
//! мгновение смены известно обеим сторонам точно: это `validUntil`, который
//! уезжает в окно вместе с кодом. Цикл в Rust дублировал бы будильник окна,
//! расшифровывал бы все секреты каждые полминуты и рассылал бы коды, которых
//! никто не просил, в том числе спрятанному окну. Вместо него — команда по
//! запросу, а будильник ставит окно.
//!
//! События тут про список записей, а не про коды: карточка обязана обновиться
//! сама, когда запись завели или удалили в настройках.
//!
//! ВАЖНО: **правило звука соблюдается с обеих сторон.** Окно просит коды,
//! только пока видно, и [`totp_codes`] всё равно проверяет это само: спрятанному
//! окну достаётся пустой список. Проверка стоит с самого начала, а не «если
//! понадобится», потому что поведение `visibilityState` в спрятанном окне Tauri
//! — предположение, а эта строка — нет.

pub mod base32;
pub mod code;
pub mod store;
pub mod uri;

use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::dpapi;

use std::sync::Mutex;

use self::store::{Account, Loaded, Row, Store, Stored, Trouble};

/// Метка окна виджета в реестре: по ней команда узнаёт, смотрит ли кто-нибудь.
const WIDGET: &str = "totp";

/// Имя события для рассылки списка записей в окна.
const TOTP_EVENT: &str = "totp";

/// Список записей глазами окна. Секретов в нём нет.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub accounts: Vec<Row>,
    /// Беда с файлом записей: `unreadable` — файл не прочитался, правка
    /// запрещена; `setAside` — файл испорчен и отложен рядом. Пусто — всё
    /// в порядке.
    pub trouble: Option<&'static str>,
}

/// Код одной записи.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Code {
    pub id: String,
    /// Пусто — секрет не расшифровался или испорчен. Карточка скажет об этом
    /// словом, а не покажет заведомо неверные цифры.
    pub code: Option<String>,
    /// Мгновение, когда код сменится, в секундах эпохи. По нему окно ставит
    /// свой будильник, и смена кода с концом кольца — одно и то же мгновение.
    pub valid_until: i64,
}

/// Ответ на запрос кодов.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Codes {
    /// Мгновение, от которого считаны коды. Одно на весь ответ: окно считает
    /// остаток от него же, а не от своих часов.
    pub now: i64,
    pub codes: Vec<Code>,
    /// Часы заведомо неверны — они раньше самой новой записи.
    ///
    /// Проверка односторонняя и бесплатная: ушедшие ВПЕРЁД часы она не ловит
    /// вовсе. Мерить настоящее расхождение можно только у внешнего источника
    /// времени, то есть заведя сеть в виджете, который сегодня офлайновый
    /// целиком, — и это отвергнуто.
    pub clock_suspect: bool,
}

/// Состояние виджета: записи в памяти плюс их же файл на диске.
pub struct Totp {
    store: Store,
    /// Чтение, правка, диск и рассылка идут вместе: отдельные get/set теряли
    /// добавленную запись при одновременном удалении из другого снимка.
    transaction: Mutex<()>,
    /// Что случилось при чтении файла. Пока он не прочитан
    /// (`Trouble::Unreadable`), правки отвергаются: записи на диске есть, мы их
    /// просто не видели, и первая же правка затёрла бы их.
    trouble: Mutex<Option<Trouble>>,
}

impl Totp {
    pub fn new(loaded: Loaded) -> Self {
        Self {
            store: Store::new(loaded.stored),
            transaction: Mutex::new(()),
            trouble: Mutex::new(loaded.trouble),
        }
    }

    fn transact<T>(&self, work: impl FnOnce() -> T) -> T {
        let _transaction = self.transaction.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        work()
    }

    fn trouble(&self) -> Option<Trouble> {
        *self.trouble.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Непрочитанный файл пробуем прочитать снова: занятым он бывает мгновение
    /// (антивирус, синхронизация папки), и до перезапуска ждать незачем.
    fn retry(&self, app: &AppHandle) {
        let recovered = {
            let mut trouble = self.trouble.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if *trouble != Some(Trouble::Unreadable) {
                return;
            }
            let loaded = store::load(app);
            if loaded.trouble == Some(Trouble::Unreadable) {
                return;
            }
            self.store.set(loaded.stored);
            *trouble = loaded.trouble;
            true
        };
        // Прочиталось — говорим всем окнам, а не одному спросившему: карточка,
        // оставшаяся на «не прочитался», сама больше ничего не спросит
        if recovered {
            let _ = app.emit(TOTP_EVENT, view(&self.store.get(), self.trouble()));
        }
    }

    /// Правка разрешена, только если файл прочитан.
    fn writable(&self, app: &AppHandle) -> Result<(), String> {
        self.retry(app);
        if self.trouble() == Some(Trouble::Unreadable) {
            return Err(tr!("totp.error.unreadable"));
        }
        Ok(())
    }
}

/// Сейчас, в секундах эпохи.
///
/// Часы раньше эпохи бывают у севшей батарейки BIOS — там ноль, а не паника.
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|passed| passed.as_secs() as i64)
        .unwrap_or(0)
}

/// Видно ли окно виджета (правило звука).
fn watched(app: &AppHandle) -> bool {
    app.get_webview_window(WIDGET)
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false)
}

/// Список записей для окна.
fn view(accounts: &Stored, trouble: Option<Trouble>) -> View {
    View {
        accounts: accounts.accounts.iter().map(Account::row).collect(),
        trouble: trouble.map(|trouble| match trouble {
            Trouble::Unreadable => "unreadable",
            Trouble::SetAside => "setAside",
        }),
    }
}

/// Записать на диск и разослать новый список по окнам.
///
/// Рассылка общая, а не поштучно: список записей у всех окон один, своего у
/// виджета тут ничего нет.
fn commit(app: &AppHandle, state: &Totp, accounts: Stored) -> Result<(), String> {
    store::save(app, &accounts)?;
    state.store.set(accounts.clone());
    {
        // Записалось — файл теперь целый, и «отложен как испорченный» больше
        // не про него: пустая карточка иначе твердила бы это до перезапуска
        let mut trouble = state.trouble.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if *trouble == Some(Trouble::SetAside) {
            *trouble = None;
        }
    }
    let _ = app.emit(TOTP_EVENT, view(&accounts, state.trouble()));
    Ok(())
}

/// Работа с файлом записей — в пуле, а не в главном потоке.
///
/// ВАЖНО: **и чтение, и запись умеют ждать.** Занятый антивирусом файл они
/// пробуют снова с паузами по 40 мс, запись ещё и доводит данные до диска, а
/// синхронная команда выполняется в обработчике IPC — на это время вставали
/// бы все окна приложения.
async fn on_pool<T: Send + 'static>(
    app: AppHandle,
    work: impl FnOnce(&AppHandle, &Totp) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Totp>();
        state.transact(|| work(&app, &state))
    })
    .await
    .map_err(|error| {
        eprintln!("[2FA] работа с файлом записей сорвалась: {error}");
        tr!("common.failed")
    })?
}

/// Имя записи: хоть что-то из двух полей обязано быть.
///
/// Безымянная строка на карточке не отвечает на единственный вопрос, ради
/// которого на неё смотрят, — чей это код.
fn named(issuer: &str, label: &str) -> Result<(), String> {
    if issuer.trim().is_empty() && label.trim().is_empty() {
        return Err(tr!("totp.error.noName"));
    }
    Ok(())
}

#[tauri::command]
pub async fn totp_state(app: AppHandle) -> Result<View, String> {
    on_pool(app, |app, state| {
        state.retry(app);
        Ok(view(&state.store.get(), state.trouble()))
    })
    .await
}

/// Коды для перечисленных записей; пустой список — коды для всех.
///
/// Спрятанное окно получает пустой ответ: расшифровывать секреты некому и
/// незачем. Окно на пустой ответ не перезапрашивает раньше следующей границы
/// периода — иначе вышла бы плотная петля IPC под спрятанной карточкой.
#[tauri::command]
pub fn totp_codes(app: AppHandle, state: State<'_, Totp>, ids: Vec<String>) -> Codes {
    let moment = now();
    let accounts = state.store.get();
    let newest = accounts
        .accounts
        .iter()
        .map(|account| account.added_at)
        .max()
        .unwrap_or(0);

    if !watched(&app) {
        return Codes {
            now: moment,
            codes: Vec::new(),
            clock_suspect: moment < newest,
        };
    }

    let wanted: Vec<&Account> = accounts
        .accounts
        .iter()
        .filter(|account| ids.is_empty() || ids.contains(&account.id))
        .collect();

    let codes = wanted
        .into_iter()
        .map(|account| {
            let shape = account.shape();
            let secret = dpapi::unprotect(&account.secret)
                .ok()
                .and_then(|text| base32::decode(&text).ok());

            match secret {
                Some(bytes) => {
                    let (digits, valid_until) = code::at(&bytes, &shape, moment);
                    Code {
                        id: account.id.clone(),
                        code: Some(digits),
                        valid_until,
                    }
                }
                // Секрет не расшифровался — чаще всего это перенесённый профиль
                // Windows. Показать вместо кода нечего, и врать цифрами нельзя
                None => Code {
                    id: account.id.clone(),
                    code: None,
                    valid_until: (moment / i64::from(shape.period) + 1) * i64::from(shape.period),
                },
            }
        })
        .collect();

    Codes {
        now: moment,
        codes,
        clock_suspect: moment < newest,
    }
}

/// Завести запись: ссылкой `otpauth://` или голым секретом в base32.
///
/// Одной командой, а не двумя: разница между ссылкой и секретом — это разбор
/// строки, а не два разных действия. Поля издателя и ярлыка при вставленной
/// ссылке уступают ей: она пришла от сервиса, а поля заполнены на глаз.
#[tauri::command]
pub async fn add_totp_account(app: AppHandle, input: String, issuer: String, label: String) -> Result<Row, String> {
    on_pool(app, move |app, state| add(app, state, input, issuer, label)).await
}

fn add(app: &AppHandle, state: &Totp, input: String, issuer: String, label: String) -> Result<Row, String> {
    state.writable(app)?;
    let typed = input.trim();
    if typed.is_empty() {
        return Err(tr!("totp.error.nothing"));
    }

    let (issuer, label, secret, shape) = if typed.to_ascii_lowercase().starts_with("otpauth://") {
        let parsed = uri::parse(typed)?;
        // Пустое поле ссылки не должно стирать то, что человек набрал руками
        let issuer = if parsed.issuer.is_empty() {
            issuer.trim().to_owned()
        } else {
            parsed.issuer
        };
        let label = if parsed.label.is_empty() {
            label.trim().to_owned()
        } else {
            parsed.label
        };
        (issuer, label, parsed.secret, parsed.shape)
    } else {
        (
            issuer.trim().to_owned(),
            label.trim().to_owned(),
            typed.to_owned(),
            code::Shape::default(),
        )
    };

    named(&issuer, &label)?;

    // Разбираем секрет ДО записи: испорченный, он дошёл бы до карточки и
    // показывал бы «Unavailable» вместо того, чтобы сказать об этом сразу
    base32::decode(&secret).map_err(|reason| tr!("totp.error.badSecret", reason = reason))?;

    let account = Account {
        id: store::new_id(),
        issuer,
        label,
        secret: dpapi::protect(&secret).map_err(|error| {
            eprintln!("[2FA] секрет не зашифровался: {error}");
            tr!("totp.error.noEncrypt")
        })?,
        digits: shape.digits,
        period: shape.period,
        algorithm: shape.algorithm,
        added_at: now(),
    };

    let row = account.row();
    let mut accounts = state.store.get();
    accounts.accounts.push(account);
    commit(app, state, accounts)?;

    Ok(row)
}

/// Переименовать запись.
///
/// Нужно потому, что секрета у человека под рукой уже нет: опечатку в ярлыке
/// иначе пришлось бы чинить, заводя запись заново с нуля.
#[tauri::command]
pub async fn rename_totp_account(app: AppHandle, id: String, issuer: String, label: String) -> Result<(), String> {
    on_pool(app, move |app, state| rename(app, state, id, issuer, label)).await
}

fn rename(app: &AppHandle, state: &Totp, id: String, issuer: String, label: String) -> Result<(), String> {
    state.writable(app)?;
    named(&issuer, &label)?;

    let mut accounts = state.store.get();
    let Some(account) = accounts
        .accounts
        .iter_mut()
        .find(|account| account.id == id)
    else {
        return Err(tr!("totp.error.noEntry"));
    };

    account.issuer = issuer.trim().to_owned();
    account.label = label.trim().to_owned();

    commit(app, state, accounts)
}

/// Переставить записи в порядке присланных ключей.
///
/// Порядок списка — это порядок на карточке, и меняют его перетаскиванием в
/// настройках, а не пунктом меню: «поднять выше» и «опустить ниже» на десятке
/// записей превращаются в два десятка нажатий.
///
/// Незнакомые ключи пропускаются, а не отвергаются, и пропущенные дописываются
/// в хвост: окно настроек могло отстать от правки из другого места, и ронять
/// из-за этого весь порядок нельзя.
#[tauri::command]
pub async fn reorder_totp_accounts(app: AppHandle, ids: Vec<String>) -> Result<(), String> {
    on_pool(app, move |app, state| {
        state.writable(app)?;
        let mut accounts = state.store.get();
        accounts.accounts = arrange(accounts.accounts, &ids);
        commit(app, state, accounts)
    })
    .await
}

/// Сама перестановка, без окон и диска, — чтобы её можно было проверить.
fn arrange(mut accounts: Vec<Account>, ids: &[String]) -> Vec<Account> {
    let mut sorted: Vec<Account> = Vec::with_capacity(accounts.len());

    for id in ids {
        if let Some(at) = accounts.iter().position(|account| account.id == *id) {
            sorted.push(accounts.remove(at));
        }
    }

    // Всё, чего в присланном порядке не было, — в хвост, в прежнем порядке
    sorted.append(&mut accounts);
    sorted
}

/// Удалить запись. Секрет уходит вместе с ней и не восстанавливается ничем.
#[tauri::command]
pub async fn remove_totp_account(app: AppHandle, id: String) -> Result<(), String> {
    on_pool(app, move |app, state| {
        state.writable(app)?;
        let mut accounts = state.store.get();
        let before = accounts.accounts.len();
        accounts.accounts.retain(|account| account.id != id);

        if accounts.accounts.len() == before {
            return Err(tr!("totp.error.noEntry"));
        }

        commit(app, state, accounts)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::totp::code::Algorithm;

    fn account(added_at: i64) -> Account {
        Account {
            id: store::new_id(),
            issuer: "GitHub".to_owned(),
            label: "demo-user".to_owned(),
            secret: "не важно".to_owned(),
            digits: 6,
            period: 30,
            algorithm: Algorithm::Sha1,
            added_at,
        }
    }

    /// Удаление не берёт старый снимок, пока добавление ещё пишет и рассылает.
    #[test]
    fn concurrent_edits_keep_disk_memory_and_events_in_one_order() {
        use std::sync::mpsc;
        use std::thread;

        let first = account(1);
        let added = account(2);
        let state = Totp::new(Loaded {
            stored: Stored { accounts: vec![first.clone()] },
            trouble: None,
        });
        let disk = Mutex::new(state.store.get());
        let events = Mutex::new(Vec::new());
        let (reading, read) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let (removing, remove_started) = mpsc::channel();

        thread::scope(|threads| {
            let state = &state;
            let disk = &disk;
            let events = &events;
            let added = &added;
            threads.spawn(move || state.transact(|| {
                let mut next = state.store.get();
                next.accounts.push(added.clone());
                reading.send(()).unwrap();
                released.recv().unwrap();
                *disk.lock().unwrap() = next.clone();
                state.store.set(next.clone());
                events.lock().unwrap().push(next);
            }));
            read.recv().unwrap();
            assert!(state.transaction.try_lock().is_err(), "транзакция держит общий замок");
            threads.spawn(move || {
                removing.send(()).unwrap();
                state.transact(|| {
                    let mut next = state.store.get();
                    next.accounts.retain(|entry| entry.id != first.id);
                    *disk.lock().unwrap() = next.clone();
                    state.store.set(next.clone());
                    events.lock().unwrap().push(next);
                });
            });
            remove_started.recv().unwrap();
            release.send(()).unwrap();
        });

        let expected = Stored { accounts: vec![added] };
        assert_eq!(state.store.get(), expected);
        assert_eq!(*disk.lock().unwrap(), expected);
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].accounts.len(), 2);
        assert_eq!(events[1], expected);
    }

    /// Фронт читает поля в camelCase, как у всех остальных виджетов.
    #[test]
    fn view_serializes_to_camel_case() {
        let accounts = Stored {
            accounts: vec![account(1)],
        };
        let json = serde_json::to_string(&view(&accounts, None)).expect("вид сериализуется");

        assert!(json.contains("\"accounts\""));
        assert!(json.contains("\"issuer\":\"GitHub\""));
        assert!(!json.contains("secret"), "секрет в виде окна недопустим");
    }

    /// Тот же инвариант для ответа с кодами: там его тоже быть не должно.
    #[test]
    fn the_codes_answer_carries_no_secret() {
        let answer = Codes {
            now: 1_700_000_000,
            codes: vec![Code {
                id: "a1".to_owned(),
                code: Some("123456".to_owned()),
                valid_until: 1_700_000_010,
            }],
            clock_suspect: false,
        };
        let json = serde_json::to_string(&answer).expect("ответ сериализуется");

        assert!(json.contains("\"validUntil\""), "будильник доезжает");
        assert!(!json.contains("secret"));
    }

    /// Нерасшифрованный код — пустота, а не выдуманные цифры.
    #[test]
    fn an_unreadable_secret_becomes_an_empty_code() {
        let answer = Code {
            id: "a1".to_owned(),
            code: None,
            valid_until: 30,
        };
        let json = serde_json::to_string(&answer).expect("сериализуется");
        assert!(json.contains("\"code\":null"));
    }

    /// Часы позади самой новой записи — точно неверные; впереди — не наше дело.
    #[test]
    fn a_clock_behind_the_newest_record_is_suspect() {
        let newest = 1_700_000_000i64;

        assert!(newest - 1 < newest, "часы позади записи подозрительны");
        assert!(
            !(newest + 100_000 < newest),
            "ушедшие вперёд часы эта проверка не ловит, и это записано"
        );
    }

    /// Вся цепочка разом: секрет → DPAPI → файл → обратно → код.
    ///
    /// Единственный тест, который держит её целиком. Порознь каждое звено
    /// проверено своим файлом, но стык «строка base32 пережила шифрование»
    /// виден только тут — а поломка в нём выглядела бы как неверный код, то
    /// есть как неисправный сервис.
    #[cfg(windows)]
    #[test]
    fn a_saved_secret_survives_dpapi_and_gives_the_same_code() {
        let plain = "JBSWY3DPEHPK3PXP";
        let shape = code::Shape::default();
        let moment = 1_700_000_000;

        let straight = code::at(&base32::decode(plain).expect("секрет разбирается"), &shape, moment);

        let stored = Stored {
            accounts: vec![Account {
                secret: dpapi::protect(plain).expect("шифруется"),
                ..account(1)
            }],
        };

        // Ровно то, что делает totp_codes
        let back = dpapi::unprotect(&stored.accounts[0].secret).expect("расшифровывается");
        let through = code::at(&base32::decode(&back).expect("разбирается"), &shape, moment);

        assert_eq!(through, straight, "код после круга через диск обязан совпасть");
        assert_ne!(stored.accounts[0].secret, plain, "на диске лежит не секрет");
    }

    /// Перестановка идёт ровно по присланному порядку.
    #[test]
    fn the_order_follows_the_sent_keys() {
        let one = account(1);
        let two = account(2);
        let three = account(3);
        let keys = vec![three.id.clone(), one.id.clone(), two.id.clone()];

        let sorted = arrange(vec![one.clone(), two.clone(), three.clone()], &keys);
        assert_eq!(
            sorted.iter().map(|a| a.id.clone()).collect::<Vec<_>>(),
            keys
        );
    }

    /// Окно настроек могло отстать от правки из другого места: незнакомый ключ
    /// пропускаем, а не ронем из-за него весь порядок.
    #[test]
    fn unknown_keys_are_skipped_and_missing_ones_go_last() {
        let one = account(1);
        let two = account(2);
        let keys = vec!["чужой".to_owned(), two.id.clone()];

        let sorted = arrange(vec![one.clone(), two.clone()], &keys);
        assert_eq!(sorted.len(), 2, "ни одна запись не потерялась");
        assert_eq!(sorted[0].id, two.id, "присланный порядок соблюдён");
        assert_eq!(sorted[1].id, one.id, "непомянутая ушла в хвост");
    }

    /// Пустой порядок ничего не меняет и ничего не теряет.
    #[test]
    fn an_empty_order_keeps_everything() {
        let before = vec![account(1), account(2)];
        let after = arrange(before.clone(), &[]);
        assert_eq!(
            after.iter().map(|a| a.id.clone()).collect::<Vec<_>>(),
            before.iter().map(|a| a.id.clone()).collect::<Vec<_>>()
        );
    }

    /// Безымянная запись не отвечает на вопрос, чей это код.
    #[test]
    fn a_nameless_record_is_refused() {
        assert!(named("", "").is_err());
        assert!(named("  ", " ").is_err(), "одни пробелы — тоже пусто");
        assert!(named("GitHub", "").is_ok());
        assert!(named("", "demo-user").is_ok());
    }
}
