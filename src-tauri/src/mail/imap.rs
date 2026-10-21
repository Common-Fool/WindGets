//! Клиент IMAP для опроса почтового ящика.
//!
//! Режим работы исключительно для чтения:
//! - `EXAMINE INBOX` вместо `SELECT INBOX`: почтовый ящик открывается только для чтения;
//! - `BODY.PEEK[]` вместо `BODY[]`: тело письма просматривается без выставления флага `\Seen`.
//!
//! Разбор MIME, заголовков и кодировок RFC 2047 выполняет `mail-parser`.

use std::time::Duration;

use async_imap::imap_proto::{types::{AttributeValue, Envelope}, Response, Status as ImapStatus};
use async_imap::types::Flag;
use futures_util::StreamExt;
use serde::Serialize;
use tokio::net::TcpStream;
use tokio_native_tls::native_tls::TlsConnector;
use tokio_native_tls::TlsConnector as TokioTlsConnector;

use super::{Attachment, Inbox, Letter, LetterBody};

/// Таймаут на весь поход в ящик: молчащий сервер должен быстро кончаться отказом.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// Таймаут на поиск по ящику (30 секунд).
///
/// Дольше обычного опроса: SEARCH TEXT сервер вычисляет по всему содержимому
/// писем, и на большом ящике поиск занимает заметно больше времени, чем
/// получение готового конверта последних сообщений.
const SEARCH_TIMEOUT: Duration = Duration::from_secs(30);

/// Размер выборки тела ради строки содержимого.
///
/// Шестьдесят четыре килобайта: письма нынче сплошь HTML без текстовой части, и
/// первые килобайты у них заняты заголовками с подписями DKIM и таблицей
/// стилей — в шестнадцати видимого текста не оказывалось у трети писем, в
/// тридцати двух у части. Столько же и стоит: выборка идёт на каждое из десяти
/// писем в каждом опросе, то есть больше полумегабайта за заход.
///
/// Ровно поэтому строка содержимого выключена по умолчанию и включается
/// настройкой: без неё тело не запрашивается вовсе, и опрос стоит одного
/// конверта.
///
/// Тридцать два килобайта, а не шестнадцать, и число это выстрадано замером.
/// Письма нынче сплошь HTML без текстовой части, и первые килобайты у них
/// заняты заголовками с подписями DKIM и таблицей стилей — видимого текста в
/// шестнадцати килобайтах не оказывалось у трети писем, и строка содержимого у
/// них пустовала. Дальше поднимать нельзя: выборка идёт на каждое из десяти
/// писем в каждом опросе, и это уже треть мегабайта за заход.
const PEEK_BYTES: u32 = 65536;

/// Сколько последних писем показываем.
const LAST_LETTERS: u32 = 10;

/// Предельная длина превью тела письма в символах.
pub const PREVIEW_MAX_CHARS: usize = 180;

/// Предельная длина текста письма для всплывающего окна в символах.
pub const BODY_MAX_CHARS: usize = 10_000;

/// Ошибки при обращении к серверу IMAP.
#[derive(Debug, Clone, PartialEq)]
pub enum MailError {
    /// Учётная запись не задана полностью
    NoAccount,
    /// Список получен до смены поколения папки: UID больше не тот.
    Stale,
    /// Ошибка входа (сервер не пустил: неверный логин или пароль)
    Auth(String),
    /// Ошибка связи (не достучались до сервера: сеть, DNS, таймаут)
    Network(String),
}

impl std::fmt::Display for MailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoAccount => write!(f, "учётная запись не задана"),
            Self::Stale => write!(f, "{}", tr!("mail.error.stale")),
            Self::Auth(msg) => write!(f, "сервер не пустил: {msg}"),
            Self::Network(msg) => write!(f, "не достучались до сервера: {msg}"),
        }
    }
}

impl MailError {
    /// Подробность нужна журналу, а команды возвращают только перевод каталога.
    pub fn user_message(&self) -> String {
        eprintln!("почта: {self}");
        match self {
            Self::NoAccount => tr!("mail.noAccount"),
            Self::Stale => tr!("mail.error.stale"),
            Self::Auth(_) => tr!("mail.signInFailed"),
            Self::Network(_) => tr!("mail.unavailable"),
        }
    }
}

/// UID можно применять только к тому поколению папки, с которым его показали.
/// Если сервер не сообщает поколение, сохраняем совместимость с ним.
fn check_uid_validity(expected: Option<u32>, actual: Option<u32>) -> Result<(), MailError> {
    if matches!((expected, actual), (Some(expected), Some(actual)) if expected != actual) {
        Err(MailError::Stale)
    } else {
        Ok(())
    }
}

/// Выполняет опрос почтового ящика с ограничением по времени в 15 секунд.
pub async fn fetch_inbox(
    host: &str,
    port: u16,
    login: &str,
    password: &str,
    // Своя ссылка на веб-ящик; `None` — вывести из адреса сервера
    own_mailbox: Option<&str>,
    // Нужна ли строка содержимого: без неё тело не запрашиваем вовсе
    preview: bool,
) -> Result<Inbox, MailError> {
    if host.trim().is_empty() || login.trim().is_empty() || password.is_empty() {
        return Err(MailError::NoAccount);
    }

    tokio::time::timeout(
        REQUEST_TIMEOUT,
        fetch_inbox_session(host, port, login, password, own_mailbox, preview),
    )
    .await
    .map_err(|_| {
        MailError::Network(format!(
            "время ожидания ответа сервера истекло ({} с)",
            REQUEST_TIMEOUT.as_secs()
        ))
    })?
}

/// Устанавливает TLS-соединение и выполняет вход на IMAP-сервер.
async fn connect_and_login(
    host: &str,
    port: u16,
    login: &str,
    password: &str,
) -> Result<async_imap::Session<tokio_native_tls::TlsStream<TcpStream>>, MailError> {
    let tls_connector = TlsConnector::builder()
        .build()
        .map_err(|e| MailError::Network(format!("ошибка инициализации TLS: {e}")))?;
    let tokio_connector = TokioTlsConnector::from(tls_connector);

    let tcp_stream = TcpStream::connect((host, port))
        .await
        .map_err(|e| MailError::Network(format!("не удалось подключиться к {host}:{port}: {e}")))?;

    let tls_stream = tokio_connector
        .connect(host, tcp_stream)
        .await
        .map_err(|e| MailError::Network(format!("ошибка TLS-рукопожатия с {host}: {e}")))?;

    let mut client = async_imap::Client::new(tls_stream);

    // Считываем начальное приветствие сервера (* OK ...)
    client
        .read_response()
        .await
        .map_err(|e| MailError::Network(format!("ошибка чтения ответа сервера: {e}")))?;

    // Выполняем вход
    let session = match client.login(login, password).await {
        Ok(session) => session,
        Err((err, _)) => {
            return Err(match err {
                // Отказ входа — это неверный пароль, только если сервер не
                // назвал другую причину. `[UNAVAILABLE]`, `[INUSE]`, лимит
                // соединений (RFC 5530) — беда на минуту, а «неверный пароль»
                // останавливает опрос до нажатия и зовёт менять верный
                async_imap::error::Error::No(msg) if !transient_refusal(&msg) => MailError::Auth(msg),
                async_imap::error::Error::Bad(msg) | async_imap::error::Error::No(msg) => {
                    MailError::Network(msg)
                }
                other => MailError::Network(other.to_string()),
            });
        }
    };

    Ok(session)
}

/// Отказ входа с кодом временной беды сервера, а не неверных реквизитов.
fn transient_refusal(message: &str) -> bool {
    let upper = message.to_ascii_uppercase();
    ["[UNAVAILABLE]", "[INUSE]", "[LIMIT]", "[SERVERBUG]", "[OVERQUOTA]", "TOO MANY"]
        .iter()
        .any(|code| upper.contains(code))
}

/// Шаг проверки подключения — на нём она и остановилась, если не прошла.
///
/// Шаги разведены затем, чтобы отказ говорил, что чинить: не нашёлся сервер —
/// опечатка в адресе, не соединилось — порт или сеть, не вышло TLS — порт без
/// шифрования, не пустил вход — пароль.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    Resolve,
    Connect,
    Tls,
    Greeting,
    Login,
    Inbox,
}

/// Чем кончилась проверка подключения из окна настроек.
///
/// `detail` — ответ системы или сервера как есть, без перевода: в нём коды,
/// по которым чинят, — `os error 11001`, `[AUTHENTICATIONFAILED]`. Пароля в
/// нём не бывает: сервер его не повторяет, а сами мы его туда не пишем.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub enum Check {
    #[serde(rename_all = "camelCase")]
    Passed { messages: u32, unseen: u32, millis: u64 },
    #[serde(rename_all = "camelCase")]
    Failed {
        stage: Stage,
        /// Сервер промолчал до срока, а не отказал.
        timed_out: bool,
        detail: String,
        millis: u64,
    },
}

/// Отказ шага: где, по сроку ли, и что сказали.
type Refusal = (Stage, bool, String);

/// Один шаг проверки под общим сроком: срок вышел — это отказ этого шага.
async fn step<T, E: std::fmt::Display>(
    stage: Stage,
    deadline: tokio::time::Instant,
    work: impl std::future::Future<Output = Result<T, E>>,
) -> Result<T, Refusal> {
    match tokio::time::timeout_at(deadline, work).await {
        Err(_) => Err((stage, true, format!("timeout {} s", REQUEST_TIMEOUT.as_secs()))),
        Ok(Err(error)) => Err((stage, false, error.to_string())),
        Ok(Ok(value)) => Ok(value),
    }
}

/// Проверяет ящик по шагам: сервер, соединение, TLS, приветствие, вход,
/// «Входящие». Ходит тем же путём, что и опрос, и так же слепо: `EXAMINE`, не
/// `SELECT`, — проверка не трогает ни одного флага.
pub async fn check_account(host: &str, port: u16, login: &str, password: &str) -> Check {
    let started = std::time::Instant::now();
    let deadline = tokio::time::Instant::now() + REQUEST_TIMEOUT;
    let result = check_steps(host, port, login, password, deadline).await;
    let millis = started.elapsed().as_millis() as u64;

    match result {
        Ok((messages, unseen)) => Check::Passed {
            messages,
            unseen,
            millis,
        },
        Err((stage, timed_out, detail)) => Check::Failed {
            stage,
            timed_out,
            detail,
            millis,
        },
    }
}

async fn check_steps(
    host: &str,
    port: u16,
    login: &str,
    password: &str,
    deadline: tokio::time::Instant,
) -> Result<(u32, u32), Refusal> {
    // Имя отдельно от соединения: «нет такого сервера» и «сервер не отвечает»
    // лечатся по-разному, а `connect` по имени слил бы их в один отказ
    let addresses: Vec<std::net::SocketAddr> =
        step(Stage::Resolve, deadline, tokio::net::lookup_host((host, port)))
            .await?
            .collect();
    if addresses.is_empty() {
        return Err((Stage::Resolve, false, format!("{host}: no addresses")));
    }

    let tcp = step(Stage::Connect, deadline, TcpStream::connect(&addresses[..])).await?;

    let connector = TlsConnector::builder()
        .build()
        .map_err(|error| (Stage::Tls, false, error.to_string()))?;
    let tls = step(
        Stage::Tls,
        deadline,
        TokioTlsConnector::from(connector).connect(host, tcp),
    )
    .await?;

    let mut client = async_imap::Client::new(tls);
    let greeting = step(Stage::Greeting, deadline, client.read_response()).await?;
    if greeting.is_none() {
        return Err((Stage::Greeting, false, "connection closed".to_owned()));
    }

    // Отказ входа — текстом сервера без обёртки: в нём код вроде
    // [AUTHENTICATIONFAILED], и он понятнее любого пересказа
    let login_result = tokio::time::timeout_at(deadline, client.login(login, password)).await;
    let mut session = match login_result {
        Err(_) => return Err((Stage::Login, true, format!("timeout {} s", REQUEST_TIMEOUT.as_secs()))),
        Ok(Err((async_imap::error::Error::Bad(text) | async_imap::error::Error::No(text), _))) => {
            return Err((Stage::Login, false, text));
        }
        Ok(Err((other, _))) => return Err((Stage::Login, false, other.to_string())),
        Ok(Ok(session)) => session,
    };

    let mailbox = step(Stage::Inbox, deadline, session.examine("INBOX")).await?;
    let unseen = step(Stage::Inbox, deadline, session.uid_search("UNSEEN UNDELETED")).await?;
    let _ = tokio::time::timeout_at(deadline, session.logout()).await;

    Ok((mailbox.exists, unseen.len() as u32))
}

/// Внутренняя логика подключения и выполнения IMAP-команд для опроса ящика.
async fn fetch_inbox_session(
    host: &str,
    port: u16,
    login: &str,
    password: &str,
    own_mailbox: Option<&str>,
    preview: bool,
) -> Result<Inbox, MailError> {
    let mut session = connect_and_login(host, port, login, password).await?;

    // Открываем ящик строго только для чтения через EXAMINE (не SELECT!)
    let mailbox = session
        .examine("INBOX")
        .await
        .map_err(|e| MailError::Network(format!("ошибка команды EXAMINE INBOX: {e}")))?;

    // Число непрочитанных через UID SEARCH. UNDELETED — потому что письмо,
    // помеченное к удалению, из карточки уже убрано, и в счётчике ему не место
    let unseen_set = session
        .uid_search("UNSEEN UNDELETED")
        .await
        .map_err(|e| MailError::Network(format!("ошибка UID SEARCH UNSEEN: {e}")))?;
    let unseen = unseen_set.len() as u32;

    // Своя ссылка старше выведенной: её задали руками именно потому, что
    // выведенная вела не туда
    let mailbox_url = own_mailbox
        .map(|link| link.to_owned())
        .or_else(|| super::web::mailbox_url(host));

    if mailbox.exists == 0 {
        let _ = session.logout().await;
        return Ok(Inbox {
            uid_validity: mailbox.uid_validity,
            unseen,
            letters: Vec::new(),
            mailbox: mailbox_url,
        });
    }

    // Определяем UIDs последних до 10 писем
    let last_uids = get_last_uids(&mut session, mailbox.exists).await?;
    if last_uids.is_empty() {
        let _ = session.logout().await;
        return Ok(Inbox {
            uid_validity: mailbox.uid_validity,
            unseen,
            letters: Vec::new(),
            mailbox: mailbox_url,
        });
    }

    // Запрашиваем ровно 10 последних писем:
    // PEEK гарантирует, что флаг \Seen не будет проставлен сервером
    let uid_query = last_uids
        .iter()
        .map(|u| u.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let fetch_query = fetch_query(preview);

    let mut letters = checked_fetch(&mut session, &format!("UID FETCH {uid_query} {fetch_query}"), |fetch| {
        // Помеченное к удалению письмо не показываем, но итог команды всё
        // равно проверяем, даже если отфильтрованы все полученные письма
        if fetch.flags().any(|flag| flag == Flag::Deleted) {
            None
        } else {
            Some(parse_letter(fetch, host, own_mailbox))
        }
    }).await?;

    // Закрываем сессию чисто
    let _ = session.logout().await;

    // Сортируем письма от новых к старым (по времени, затем по UID)
    letters.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| b.uid.cmp(&a.uid)));

    Ok(Inbox {
        uid_validity: mailbox.uid_validity,
        unseen,
        letters,
        mailbox: mailbox_url,
    })
}

/// Сверяет поколение сразу при открытии папки, до любой команды с UID.
async fn checked_inbox<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send>(
    session: &mut async_imap::Session<T>,
    uid_validity: Option<u32>,
    writable: bool,
) -> Result<(), MailError> {
    let mailbox = if writable {
        session.select("INBOX").await
    } else {
        session.examine("INBOX").await
    }.map_err(|e| MailError::Network(format!("ошибка открытия INBOX: {e}")))?;
    check_uid_validity(uid_validity, mailbox.uid_validity)
}

/// Данные FETCH остаются у парсера библиотеки; здесь нужны только поля,
/// которые читает карточка. Конструктор библиотечного Fetch закрыт.
struct Fetched<'a>(&'a [AttributeValue<'a>]);

impl Fetched<'_> {
    fn uid(&self) -> Option<u32> {
        self.0.iter().find_map(|attr| match attr {
            AttributeValue::Uid(uid) => Some(*uid),
            _ => None,
        })
    }

    fn flags(&self) -> impl Iterator<Item = Flag<'_>> {
        self.0.iter().filter_map(|attr| match attr {
            AttributeValue::Flags(flags) => Some(flags.iter().map(|flag| Flag::from(flag.as_ref()))),
            _ => None,
        }).flatten()
    }

    fn body(&self) -> Option<&[u8]> {
        self.0.iter().find_map(|attr| match attr {
            AttributeValue::BodySection { section: None, data: Some(body), .. }
            | AttributeValue::Rfc822(Some(body)) => Some(body.as_ref()),
            _ => None,
        })
    }

    fn size(&self) -> Option<u32> {
        self.0.iter().find_map(|attr| match attr {
            AttributeValue::Rfc822Size(size) => Some(*size),
            _ => None,
        })
    }

    fn envelope(&self) -> Option<&Envelope<'_>> {
        self.0.iter().find_map(|attr| match attr {
            AttributeValue::Envelope(envelope) => Some(envelope.as_ref()),
            _ => None,
        })
    }

    fn internal_date(&self) -> Option<i64> {
        let raw = self.0.iter().find_map(|attr| match attr {
            AttributeValue::InternalDate(date) => Some(date.as_ref()),
            _ => None,
        })?;
        // У IMAP дата «17-Jul-2026», у RFC 822 — «17 Jul 2026».
        // Меняем только две черты даты: минус часового пояса остаётся своим
        let date = mail_parser::DateTime::parse_rfc822(&raw.replacen('-', " ", 2))?;
        date.is_valid().then(|| date.to_timestamp())
    }
}

/// async-imap 0.11.3 завершает поток FETCH на теге, не проверяя его статус.
/// Читаем ответы тем же парсером, но накопленное отдаём только после своего
/// tagged OK: даже одно полученное письмо не превращает NO/BAD в успех.
async fn checked_fetch<T, R>(
    session: &mut async_imap::Session<T>,
    command: &str,
    mut parse: impl FnMut(&Fetched<'_>) -> Option<R>,
) -> Result<Vec<R>, MailError>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    let tag = session.run_command(command).await
        .map_err(|e| MailError::Network(format!("ошибка {command}: {e}")))?;
    let mut values = Vec::new();
    let mut answered = false;
    loop {
        let response = session.read_response().await
            .map_err(|e| MailError::Network(format!("ошибка чтения FETCH: {e}")))?
            .ok_or_else(|| MailError::Network("соединение оборвалось до завершения FETCH".to_owned()))?;
        match response.parsed() {
            Response::Done { tag: received, status, code, information } => {
                if received != &tag || *status != ImapStatus::Ok {
                    return Err(MailError::Network(format!("ошибка завершения FETCH: {received:?} {status:?} {code:?} {information:?}")));
                }
                if !answered {
                    return Err(MailError::Network("сервер не отдал ни одного письма из запрошенных".to_owned()));
                }
                return Ok(values);
            }
            Response::Fetch(_, attributes) => {
                answered = true;
                if let Some(value) = parse(&Fetched(attributes)) {
                    values.push(value);
                }
            }
            Response::Data { status: ImapStatus::Bye, .. } => {
                return Err(MailError::Network("сервер закрыл соединение во время FETCH".to_owned()));
            }
            _ => {}
        }
    }
}

/// Дочитывает ответ STORE и проверяет tagged-статус, включая NO/BAD и обрыв.
/// Поток uid_store в async-imap 0.11 отбрасывает финальный статус без проверки.
async fn store_flags<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send>(
    session: &mut async_imap::Session<T>,
    uids: &str,
    flag: &str,
) -> Result<(), MailError> {
    session
        .run_command_and_check_ok(format!("UID STORE {uids} +FLAGS.SILENT ({flag})"))
        .await
        .map_err(|e| MailError::Network(format!("ошибка UID STORE: {e}")))
}

/// Помечает письмо прочитанным по UID с ограничением по времени в 15 секунд.
pub async fn mark_read(
    host: &str,
    port: u16,
    login: &str,
    password: &str,
    uids: &[u32],
    uid_validity: Option<u32>,
) -> Result<(), MailError> {
    if host.trim().is_empty() || login.trim().is_empty() || password.is_empty() {
        return Err(MailError::NoAccount);
    }
    if uids.is_empty() {
        return Ok(());
    }

    tokio::time::timeout(
        REQUEST_TIMEOUT,
        mark_read_session(host, port, login, password, uids, uid_validity),
    )
    .await
    .map_err(|_| {
        MailError::Network(format!(
            "время ожидания ответа сервера истекло ({} с)",
            REQUEST_TIMEOUT.as_secs()
        ))
    })?
}

async fn mark_read_session(
    host: &str,
    port: u16,
    login: &str,
    password: &str,
    uids: &[u32],
    uid_validity: Option<u32>,
) -> Result<(), MailError> {
    let mut session = connect_and_login(host, port, login, password).await?;

    checked_inbox(&mut session, uid_validity, true).await?;

    store_flags(&mut session, &uid_set(uids), "\\Seen").await?;

    let _ = session.logout().await;
    Ok(())
}

/// Помечает все письма в ящике прочитанными с ограничением по времени в 15 секунд.
pub async fn mark_all_read(
    host: &str,
    port: u16,
    login: &str,
    password: &str,
) -> Result<(), MailError> {
    if host.trim().is_empty() || login.trim().is_empty() || password.is_empty() {
        return Err(MailError::NoAccount);
    }

    tokio::time::timeout(
        REQUEST_TIMEOUT,
        mark_all_read_session(host, port, login, password),
    )
    .await
    .map_err(|_| {
        MailError::Network(format!(
            "время ожидания ответа сервера истекло ({} с)",
            REQUEST_TIMEOUT.as_secs()
        ))
    })?
}

async fn mark_all_read_session(
    host: &str,
    port: u16,
    login: &str,
    password: &str,
) -> Result<(), MailError> {
    let mut session = connect_and_login(host, port, login, password).await?;

    session
        .select("INBOX")
        .await
        .map_err(|e| MailError::Network(format!("ошибка SELECT INBOX: {e}")))?;

    // Сперва пробуем одной командой на весь диапазон; не принял сервер
    // диапазон `1:*` — ищем непрочитанные сами, и уже их отказ — настоящий
    if store_flags(&mut session, "1:*", "\\Seen").await.is_err() {
        let unseen = session
            .uid_search("UNSEEN")
            .await
            .map_err(|e| MailError::Network(format!("ошибка UID SEARCH UNSEEN: {e}")))?;

        if !unseen.is_empty() {
            let uids = unseen.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(",");
            store_flags(&mut session, &uids, "\\Seen").await?;
        }
    }

    let _ = session.logout().await;
    Ok(())
}

/// Удаляет письмо (переносит в корзину) с ограничением по времени в 15 секунд.
pub async fn delete_letter(
    host: &str,
    port: u16,
    login: &str,
    password: &str,
    uids: &[u32],
    uid_validity: Option<u32>,
) -> Result<(), MailError> {
    if host.trim().is_empty() || login.trim().is_empty() || password.is_empty() {
        return Err(MailError::NoAccount);
    }
    if uids.is_empty() {
        return Ok(());
    }

    tokio::time::timeout(
        REQUEST_TIMEOUT,
        delete_letter_session(host, port, login, password, uids, uid_validity),
    )
    .await
    .map_err(|_| {
        MailError::Network(format!(
            "время ожидания ответа сервера истекло ({} с)",
            REQUEST_TIMEOUT.as_secs()
        ))
    })?
}

async fn delete_letter_session(
    host: &str,
    port: u16,
    login: &str,
    password: &str,
    uids: &[u32],
    uid_validity: Option<u32>,
) -> Result<(), MailError> {
    let mut session = connect_and_login(host, port, login, password).await?;

    // Открываем ящик INBOX на запись
    checked_inbox(&mut session, uid_validity, true).await?;

    // Опрашиваем возможности сервера
    let capabilities = session
        .capabilities()
        .await
        .map_err(|e| MailError::Network(format!("ошибка CAPABILITY: {e}")))?;
    let can_move = capabilities.has_str("MOVE");
    let has_uidplus = capabilities.has_str("UIDPLUS");

    // Ищем папку корзины по списку папок
    let mut list_stream = session
        .list(Some(""), Some("*"))
        .await
        .map_err(|e| MailError::Network(format!("ошибка LIST: {e}")))?;

    struct FolderInfo {
        name: String,
        has_trash_attr: bool,
    }

    let mut folders = Vec::new();
    while let Some(res) = list_stream.next().await {
        if let Ok(name) = res {
            let has_trash_attr = name.attributes().iter().any(|attr| {
                matches!(attr, async_imap::types::NameAttribute::Trash)
                    || matches!(attr, async_imap::types::NameAttribute::Extension(ext) if ext.eq_ignore_ascii_case("\\Trash") || ext.eq_ignore_ascii_case("Trash"))
            });
            folders.push(FolderInfo {
                name: name.name().to_string(),
                has_trash_attr,
            });
        }
    }
    drop(list_stream);

    // Порядок поиска: флаг `\Trash`, потом имя `Trash`, потом `[Gmail]/Trash`
    let trash_folder = folders
        .iter()
        .find(|f| f.has_trash_attr)
        .or_else(|| folders.iter().find(|f| f.name.eq_ignore_ascii_case("Trash")))
        .or_else(|| {
            folders
                .iter()
                .find(|f| f.name == "[Gmail]/Trash" || f.name.eq_ignore_ascii_case("[Gmail]/Trash"))
        })
        .map(|f| f.name.clone());

    let Some(trash_mailbox) = trash_folder else {
        let _ = session.logout().await;
        return Err(MailError::Network(
            "папка корзины не найдена на сервере".to_string(),
        ));
    };

    // Перенос в корзину
    let moved = if can_move {
        session.uid_mv(uid_set(uids), &trash_mailbox).await.is_ok()
    } else {
        false
    };

    if !moved {
        // Запасной путь: копия в корзину, затем пометка к удалению
        session
            .uid_copy(uid_set(uids), &trash_mailbox)
            .await
            .map_err(|e| MailError::Network(format!("ошибка UID COPY в корзину: {e}")))?;

        store_flags(&mut session, &uid_set(uids), "\\Deleted").await?;

        /*
         * ВАЖНО: **сплошного `EXPUNGE` тут быть не должно.** Он выносит из
         * ящика ВСЕ письма с флагом `\Deleted`, а не наше одно, — и чужие
         * пометки, оставленные другим почтовиком, ушли бы вместе с ним
         * безвозвратно и никуда: копию в корзину мы сделали только своему
         * письму. Одно нажатие мышью стёрло бы чужую почту.
         *
         * Поэтому точечный `UID EXPUNGE` — только там, где сервер объявил
         * `UIDPLUS`. Где не объявил, письмо остаётся помеченным: копия в
         * корзине уже лежит, а из карточки помеченные к удалению не
         * показываются вовсе. Настоящую уборку сделает почтовик.
         */
        if has_uidplus {
            let exp_stream = session
                .uid_expunge(uid_set(uids))
                .await
                .map_err(|e| MailError::Network(format!("ошибка UID EXPUNGE: {e}")))?;
            tokio::pin!(exp_stream);
            while let Some(res) = exp_stream.next().await {
                res.map_err(|e| MailError::Network(format!("ошибка UID EXPUNGE: {e}")))?;
            }
        }
    }

    let _ = session.logout().await;
    Ok(())
}

/// Находит UID последних десяти писем.
///
/// ВАЖНО: **спрашиваем номерами сообщений, а не `UID SEARCH ALL`.** Поиск по
/// всему ящику возвращает UID каждого письма — в почте, копившейся годами, это
/// десятки тысяч чисел, и приходят они каждые две минуты внутри общего срока в
/// пятнадцать секунд. Последние письма — это всегда хвост нумерации, и хвост
/// берётся одним коротким запросом независимо от размера ящика.
async fn get_last_uids<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send>(
    session: &mut async_imap::Session<T>,
    exists: u32,
) -> Result<Vec<u32>, MailError> {
    let seq_start = exists.saturating_sub(LAST_LETTERS - 1).max(1);
    let mut uids = checked_fetch(session, &format!("FETCH {seq_start}:* UID"), |fetch| fetch.uid()).await?;
    // Письма в ящике есть, а номеров не пришло — это отказ, а не пустой ящик
    if uids.is_empty() {
        return Err(MailError::Network("сервер не отдал номера последних писем".to_owned()));
    }

    uids.sort_unstable();
    Ok(uids)
}

/// Извлекает поля письма из ответа FETCH и тела сообщения.
fn parse_letter(
    fetch: &Fetched<'_>,
    host: &str,
    own_mailbox: Option<&str>,
) -> Letter {
    let uid = fetch.uid().unwrap_or(0);
    let seen = fetch.flags().any(|f| f == Flag::Seen);
    let at = fetch
        .internal_date()
        .unwrap_or_default();

    // Тело письма из BODY[] или BODY.PEEK[]
    let body_bytes = fetch.body();

    // Разбираем MIME и заголовки через mail-parser
    let parsed_msg = body_bytes.and_then(|bytes| mail_parser::MessageParser::default().parse(bytes));

    let subject = extract_subject(fetch, parsed_msg.as_ref());
    let from = extract_from(fetch, parsed_msg.as_ref());
    let from_address = extract_from_address(Some(fetch), parsed_msg.as_ref());
    let to = extract_to(Some(fetch), parsed_msg.as_ref());
    let raw_text = extract_raw_text(parsed_msg.as_ref());

    // Превью рисуется простым текстом: записи ссылок `[слово](адрес)` в нём
    // остаются одним словом, адрес показывать нельзя
    let preview = raw_text.as_deref().map(link_words).as_deref().and_then(clean_preview);

    let message_id = extract_message_id(fetch, parsed_msg.as_ref());
    let link = super::web::letter_url(host, message_id.as_deref(), own_mailbox);

    Letter {
        // Чей ящик — знает только опрос: сессия IMAP о нашем списке ящиков не
        // слышала. Заполняет `mail/mod.rs`
        account: String::new(),
        revision: String::new(),
        uid_validity: None,
        uid,
        from,
        from_address,
        to,
        subject,
        at,
        seen,
        preview,
        link,
    }
}

/// Предел на тело одного письма: мегабайт.
///
/// Не экономия, а защита от письма с вложением на два десятка мегабайт:
/// показать его карточке всё равно нечем, а ждать пришлось бы всерьёз.
const BODY_LIMIT: u32 = 1024 * 1024;

/// Тянет тело одного письма целиком — для всплывающего окна.
///
/// ВАЖНО: **тело не едет вместе со списком, и это не экономия, а
/// работоспособность.** Списку хватает шестнадцати килобайт на письмо ради
/// строчки превью, но письма нынче сплошь HTML без текстовой части, и видимый
/// текст в них лежит за гранью этой выборки — до неё доезжает одна шапка со
/// стилями. Замерено щупом на живом ящике: из десяти писем шесть давали пустоту,
/// а остальные — восемнадцать знаков подписи внизу. Целиком тянем ровно то
/// письмо, которое открыли, и ровно тогда, когда его открыли.
pub async fn fetch_body(
    host: &str,
    port: u16,
    login: &str,
    password: &str,
    uid: u32,
    uid_validity: Option<u32>,
) -> Result<LetterBody, MailError> {
    tokio::time::timeout(
        REQUEST_TIMEOUT,
        fetch_body_session(host, port, login, password, uid, uid_validity),
    )
    .await
    .map_err(|_| {
        MailError::Network(format!(
            "время ожидания ответа сервера истекло ({} с)",
            REQUEST_TIMEOUT.as_secs()
        ))
    })?
}

async fn fetch_body_session(
    host: &str,
    port: u16,
    login: &str,
    password: &str,
    uid: u32,
    uid_validity: Option<u32>,
) -> Result<LetterBody, MailError> {
    let mut session = connect_and_login(host, port, login, password).await?;

    // Только чтение и только PEEK: открытое письмо прочитанным не становится,
    // для этого есть своя кнопка
    checked_inbox(&mut session, uid_validity, false).await?;

    let bodies = checked_fetch(&mut session, &format!("UID FETCH {uid} (RFC822.SIZE BODY.PEEK[]<0.{BODY_LIMIT}>)"), fetched_letter_body).await?;
    let letter_body = bodies.into_iter().next().unwrap_or(LetterBody {
        truncated: false,
        text: None,
        attachments: Vec::new(),
    });
    let _ = session.logout().await;
    Ok(letter_body)
}

/// Признак обрезки сохраняется, даже если оборванный MIME не разобрался.
fn fetched_letter_body(fetch: &Fetched<'_>) -> Option<LetterBody> {
    let bytes = fetch.body()?;
    let mut body = mail_parser::MessageParser::default().parse(bytes)
        .as_ref().map(parse_letter_body)
        .unwrap_or(LetterBody { truncated: false, text: None, attachments: Vec::new() });
    body.truncated |= fetch.size().map_or(bytes.len() >= BODY_LIMIT as usize, |size| size as usize > bytes.len());
    Some(body)
}

/// Извлекает тело письма и список вложений из разобранного сообщения.
pub fn parse_letter_body(msg: &mail_parser::Message<'_>) -> LetterBody {
    let text = extract_raw_text(Some(msg)).as_deref().and_then(clean_body_full);
    let truncated = text.as_ref().is_some_and(|text| text.chars().count() > BODY_MAX_CHARS);
    let text = text.map(|text| text.chars().take(BODY_MAX_CHARS).collect());
    let attachments = extract_attachments(msg);
    LetterBody { truncated, text, attachments }
}

/// Извлекает вложения из сообщения.
/// Встроенные картинки (inline) вложениями не считаются.
pub fn extract_attachments(msg: &mail_parser::Message<'_>) -> Vec<Attachment> {
    use mail_parser::MimeHeaders;

    let mut attachments = Vec::new();
    for part in msg.attachments() {
        // Встроенные картинки (inline) вложениями не считаем
        let is_inline = matches!(part.body, mail_parser::PartType::InlineBinary(_))
            || part.content_disposition().is_some_and(|cd| cd.is_inline())
            || (part.content_id().is_some() && !part.content_disposition().is_some_and(|cd| cd.is_attachment()));

        if is_inline {
            continue;
        }

        let name = part
            .attachment_name()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.rsplit(['/', '\\']).next().unwrap_or(s))
            .filter(|s| !s.is_empty())
            .unwrap_or("attachment")
            .to_string();

        let size = part.len() as u64;
        let kind = detect_attachment_kind(&name, part.content_type());

        attachments.push(Attachment { name, size, kind });
    }

    attachments
}

/// Определяет короткий тип вложения («pdf», «png», «zip», «file»)
/// по расширению файла или MIME-типу. Незнакомый тип — «file».
pub fn detect_attachment_kind(filename: &str, content_type: Option<&mail_parser::ContentType<'_>>) -> String {
    // 1. Проверяем расширение имени файла
    if let Some(ext) = filename.rsplit('.').next() {
        if ext != filename {
            let ext_lower = ext.to_ascii_lowercase();
            match ext_lower.as_str() {
                "pdf" => return "pdf".to_string(),
                "png" => return "png".to_string(),
                "jpg" | "jpeg" => return "jpg".to_string(),
                "gif" => return "gif".to_string(),
                "webp" => return "webp".to_string(),
                "svg" => return "svg".to_string(),
                "zip" => return "zip".to_string(),
                "rar" => return "rar".to_string(),
                "7z" => return "7z".to_string(),
                "tar" => return "tar".to_string(),
                "gz" => return "gz".to_string(),
                "doc" | "docx" => return ext_lower,
                "xls" | "xlsx" => return ext_lower,
                "ppt" | "pptx" => return ext_lower,
                "txt" => return "txt".to_string(),
                "csv" => return "csv".to_string(),
                "mp3" => return "mp3".to_string(),
                "mp4" => return "mp4".to_string(),
                _ => {}
            }
        }
    }

    // 2. Проверяем MIME-тип
    if let Some(ct) = content_type {
        let ctype = ct.ctype().to_ascii_lowercase();
        let subtype = ct.subtype().unwrap_or("").to_ascii_lowercase();
        let mime = format!("{ctype}/{subtype}");
        match mime.as_str() {
            "application/pdf" => return "pdf".to_string(),
            "image/png" => return "png".to_string(),
            "image/jpeg" => return "jpg".to_string(),
            "image/gif" => return "gif".to_string(),
            "image/webp" => return "webp".to_string(),
            "image/svg+xml" => return "svg".to_string(),
            "application/zip" | "application/x-zip-compressed" => return "zip".to_string(),
            "application/x-rar-compressed" | "application/vnd.rar" => return "rar".to_string(),
            "application/x-7z-compressed" => return "7z".to_string(),
            "application/gzip" | "application/x-gzip" => return "gz".to_string(),
            "application/x-tar" => return "tar".to_string(),
            "text/plain" => return "txt".to_string(),
            "text/csv" => return "csv".to_string(),
            "application/msword" => return "doc".to_string(),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => return "docx".to_string(),
            "application/vnd.ms-excel" => return "xls".to_string(),
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => return "xlsx".to_string(),
            "application/vnd.ms-powerpoint" => return "ppt".to_string(),
            "application/vnd.openxmlformats-officedocument.presentationml.presentation" => return "pptx".to_string(),
            "audio/mpeg" => return "mp3".to_string(),
            "video/mp4" => return "mp4".to_string(),
            _ => {}
        }
    }

    "file".to_string()
}

/// Сколько найденных писем максимум возвращаем при поиске.
const SEARCH_LETTERS_LIMIT: usize = 30;

/// Мерки поиска: что ищем и только ли среди непрочитанных.
///
/// Стоят одним типом, а не двумя доводами подряд, потому что в `SEARCH` они и
/// складываются в одну строку условий — `UNDELETED UNSEEN TEXT "слово"`. Это
/// фильтр и поиск разом, а не две похожих команды: пустой запрос при поднятом
/// `unseen_only` — по-прежнему поиск, просто одной меркой меньше.
#[derive(Debug, Clone, Copy)]
pub struct SearchCriteria<'a> {
    /// Строка запроса от человека; пустая — ищем одним лишь фильтром
    pub query: &'a str,
    /// Показывать только непрочитанные
    pub unseen_only: bool,
}

/// Выполняет поиск по ящику по `TEXT "<запрос>"` с ограничением по времени в 30 секунд.
/// Возвращает до 30 самых новых писем из найденных.
pub async fn search_inbox(
    host: &str,
    port: u16,
    login: &str,
    password: &str,
    criteria: SearchCriteria<'_>,
    own_mailbox: Option<&str>,
    preview: bool,
) -> Result<Inbox, MailError> {
    if host.trim().is_empty() || login.trim().is_empty() || password.is_empty() {
        return Err(MailError::NoAccount);
    }

    let criteria = SearchCriteria {
        query: criteria.query.trim(),
        ..criteria
    };
    if criteria.query.is_empty() && !criteria.unseen_only {
        let mailbox_url = own_mailbox
            .map(|link| link.to_owned())
            .or_else(|| super::web::mailbox_url(host));
        return Ok(Inbox {
            uid_validity: None,
            unseen: 0,
            letters: Vec::new(),
            mailbox: mailbox_url,
        });
    }

    tokio::time::timeout(
        SEARCH_TIMEOUT,
        search_inbox_session(host, port, login, password, criteria, own_mailbox, preview),
    )
    .await
    .map_err(|_| {
        MailError::Network(format!(
            "время ожидания ответа сервера при поиске истекло ({} с)",
            SEARCH_TIMEOUT.as_secs()
        ))
    })?
}

async fn search_inbox_session(
    host: &str,
    port: u16,
    login: &str,
    password: &str,
    criteria: SearchCriteria<'_>,
    own_mailbox: Option<&str>,
    preview: bool,
) -> Result<Inbox, MailError> {
    let mut session = connect_and_login(host, port, login, password).await?;

    // Открываем ящик строго только для чтения через EXAMINE (не SELECT!)
    let mailbox = session
        .examine("INBOX")
        .await
        .map_err(|e| MailError::Network(format!("ошибка команды EXAMINE INBOX: {e}")))?;

    let mailbox_url = own_mailbox
        .map(|link| link.to_owned())
        .or_else(|| super::web::mailbox_url(host));

    /*
     * Условия складываются: «непрочитанные» и «содержит слово» — это один и тот
     * же `SEARCH`, просто с двумя мерками подряд. Поэтому фильтр и поиск здесь
     * одна команда, а не две похожих.
     */
    let mut conditions = vec!["UNDELETED".to_owned()];
    if criteria.unseen_only {
        conditions.push("UNSEEN".to_owned());
    }
    if !criteria.query.is_empty() {
        conditions.push(format!("TEXT {}", escape_imap_string(criteria.query)));
    }
    let conditions = conditions.join(" ");

    /*
     * Сперва с `CHARSET UTF-8`, и только потом без него.
     *
     * Запрос пишет человек, и половина писем тут по-русски. Строка команды в
     * IMAP по букве стандарта семибитная, и кириллица в ней без объявленной
     * кодировки — на усмотрение сервера: один поймёт, другой ответит `BAD`, а
     * третий молча не найдёт ничего. Объявленная кодировка снимает вопрос там,
     * где сервер её умеет; где не умеет — он отвечает отказом на саму команду,
     * и тогда идём прежним путём, которым ищут латиницу.
     */
    let found_uids = match session
        .uid_search(&format!("CHARSET UTF-8 {conditions}"))
        .await
    {
        Ok(found) => found,
        Err(_) => session
            .uid_search(&conditions)
            .await
            .map_err(|e| MailError::Network(format!("ошибка UID SEARCH: {e}")))?,
    };

    if found_uids.is_empty() {
        let _ = session.logout().await;
        return Ok(Inbox {
            uid_validity: mailbox.uid_validity,
            unseen: 0,
            letters: Vec::new(),
            mailbox: mailbox_url,
        });
    }

    let mut uids: Vec<u32> = found_uids.into_iter().collect();
    uids.sort_unstable();

    // Берём не более 30 самых новых писем (хвост списка UID)
    let selected_uids = if uids.len() > SEARCH_LETTERS_LIMIT {
        &uids[uids.len() - SEARCH_LETTERS_LIMIT..]
    } else {
        &uids[..]
    };

    let uid_query = selected_uids
        .iter()
        .map(|u| u.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let fetch_query = fetch_query(preview);

    let mut letters = checked_fetch(&mut session, &format!("UID FETCH {uid_query} {fetch_query}"), |fetch| {
        // Помеченное к удалению письмо не показываем, но итог команды всё
        // равно проверяем, даже если отфильтрованы все полученные письма
        if fetch.flags().any(|flag| flag == Flag::Deleted) {
            None
        } else {
            Some(parse_letter(fetch, host, own_mailbox))
        }
    }).await?;

    let _ = session.logout().await;

    // Сортируем письма от новых к старым (по времени, затем по UID)
    letters.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| b.uid.cmp(&a.uid)));

    // Число непрочитанных СРЕДИ НАЙДЕННЫХ
    let unseen = letters.iter().filter(|l| !l.seen).count() as u32;

    Ok(Inbox {
        uid_validity: mailbox.uid_validity,
        unseen,
        letters,
        mailbox: mailbox_url,
    })
}

/// Что просим у сервера на каждое письмо.
///
/// Без строки содержимого тело не запрашивается вовсе: отправитель, тема и дата
/// лежат в конверте и стоят копейки, а выборка тела — это десятки килобайт на
/// письмо в каждом опросе.
fn fetch_query(preview: bool) -> String {
    if preview {
        format!("(UID ENVELOPE FLAGS INTERNALDATE BODY.PEEK[]<0.{PEEK_BYTES}>)")
    } else {
        "(UID ENVELOPE FLAGS INTERNALDATE)".to_owned()
    }
}

/// Собирает набор UID для команды: «12,15,19».
///
/// Пачкой, а не по одному: каждое письмо своей сессией — это соединение, TLS и
/// вход на каждую галочку, то есть секунды на письмо. Отметить десяток разом
/// стоит ровно столько же, сколько отметить одно.
fn uid_set(uids: &[u32]) -> String {
    uids.iter()
        .map(|uid| uid.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// Экранирует строку запроса для команды IMAP: обрамляет кавычками,
/// экранируя обратные косые черты и кавычки внутри.
pub fn escape_imap_string(query: &str) -> String {
    let mut escaped = String::with_capacity(query.len() + 2);
    escaped.push('"');
    for c in query.chars() {
        match c {
            '\\' => escaped.push_str(r"\\"),
            '"' => escaped.push_str(r#"\""#),
            '\r' | '\n' => escaped.push(' '),
            other => escaped.push(other),
        }
    }
    escaped.push('"');
    escaped
}

/// Извлекает Message-ID из заголовков письма (mail-parser) или из IMAP ENVELOPE.
fn extract_message_id(
    fetch: &Fetched<'_>,
    parsed: Option<&mail_parser::Message<'_>>,
) -> Option<String> {
    if let Some(msg) = parsed {
        if let Some(mid) = msg.message_id() {
            let s = mid.trim();
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }

    if let Some(env) = fetch.envelope() {
        if let Some(raw) = &env.message_id {
            let s = String::from_utf8_lossy(raw);
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }

    None
}

/// Извлекает тему письма с декодированием RFC 2047 (`=?UTF-8?B?...?=`).
fn extract_subject(
    fetch: &Fetched<'_>,
    parsed: Option<&mail_parser::Message<'_>>,
) -> String {
    // Если mail-parser успешно разобрал сообщение, берём тему оттуда
    if let Some(msg) = parsed {
        if let Some(subj) = msg.subject() {
            return subj.to_string();
        }
    }

    // Иначе берём из конверта IMAP
    if let Some(env) = fetch.envelope() {
        if let Some(raw) = &env.subject {
            let raw_str = String::from_utf8_lossy(raw);
            return decode_rfc2047_header(&raw_str);
        }
    }

    String::new()
}

/// Извлекает отправителя: имя из конверта, а если имени нет — адрес.
fn extract_from(fetch: &Fetched<'_>, parsed: Option<&mail_parser::Message<'_>>) -> String {
    // Сначала пробуем разобранное mail-parser сообщение
    if let Some(msg) = parsed {
        if let Some(from_header) = msg.from() {
            let (name, addr) = match from_header {
                mail_parser::Address::List(list) => {
                    let first = list.first();
                    (
                        first.and_then(|a| a.name.as_deref()),
                        first.and_then(|a| a.address.as_deref()),
                    )
                }
                mail_parser::Address::Group(groups) => {
                    let first = groups.first().and_then(|g| g.addresses.first());
                    (
                        first.and_then(|a| a.name.as_deref()),
                        first.and_then(|a| a.address.as_deref()),
                    )
                }
            };

            if let Some(n) = name.filter(|s| !s.trim().is_empty()) {
                return n.trim().to_string();
            }
            if let Some(a) = addr.filter(|s| !s.trim().is_empty()) {
                return a.trim().to_string();
            }
        }
    }

    // Запасной путь из IMAP ENVELOPE
    if let Some(env) = fetch.envelope() {
        if let Some(addrs) = &env.from {
            if let Some(addr) = addrs.first() {
                let name = addr.name.as_ref().map(|n| {
                    let s = String::from_utf8_lossy(n);
                    decode_rfc2047_header(&s)
                });
                let mailbox = addr
                    .mailbox
                    .as_ref()
                    .map(|m| String::from_utf8_lossy(m).to_string());
                let host = addr
                    .host
                    .as_ref()
                    .map(|h| String::from_utf8_lossy(h).to_string());

                let address_str = match (mailbox, host) {
                    (Some(m), Some(h)) => format!("{m}@{h}"),
                    (Some(m), None) => m,
                    _ => String::new(),
                };

                if let Some(n) = name.filter(|s| !s.trim().is_empty()) {
                    return n.trim().to_string();
                }
                if !address_str.trim().is_empty() {
                    return address_str.trim().to_string();
                }
            }
        }
    }

    String::new()
}

/// Форматирует адрес из полей IMAP ENVELOPE (mailbox и host).
fn format_imap_address(mailbox: Option<&[u8]>, host: Option<&[u8]>) -> Option<String> {
    let m = mailbox.map(String::from_utf8_lossy);
    let h = host.map(String::from_utf8_lossy);
    let s = match (m, h) {
        (Some(mb), Some(ht)) if !mb.is_empty() && !ht.is_empty() => format!("{mb}@{ht}"),
        (Some(mb), _) if !mb.is_empty() => mb.into_owned(),
        _ => String::new(),
    };
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Извлекает адрес отправителя из IMAP ENVELOPE или разобранного сообщения.
fn extract_from_address(
    fetch: Option<&Fetched<'_>>,
    parsed: Option<&mail_parser::Message<'_>>,
) -> Option<String> {
    if let Some(f) = fetch {
        if let Some(env) = f.envelope() {
            if let Some(addrs) = &env.from {
                if let Some(addr) = addrs.first() {
                    if let Some(a) = format_imap_address(addr.mailbox.as_deref(), addr.host.as_deref()) {
                        return Some(a);
                    }
                }
            }
        }
    }

    if let Some(msg) = parsed {
        if let Some(from_header) = msg.from() {
            let addr = match from_header {
                mail_parser::Address::List(list) => {
                    list.first().and_then(|a| a.address.as_deref())
                }
                mail_parser::Address::Group(groups) => {
                    groups.first().and_then(|g| g.addresses.first()).and_then(|a| a.address.as_deref())
                }
            };
            if let Some(a) = addr.map(str::trim).filter(|s| !s.is_empty()) {
                return Some(a.to_string());
            }
        }
    }

    None
}

/// Извлекает адреса получателей из IMAP ENVELOPE или разобранного сообщения.
/// Возвращает не более пяти адресов. Пустой список — пустой Vec, а не None.
fn extract_to(
    fetch: Option<&Fetched<'_>>,
    parsed: Option<&mail_parser::Message<'_>>,
) -> Vec<String> {
    let mut recipients = Vec::new();

    if let Some(f) = fetch {
        if let Some(env) = f.envelope() {
            if let Some(addrs) = &env.to {
                for addr in addrs {
                    if let Some(a) = format_imap_address(addr.mailbox.as_deref(), addr.host.as_deref()) {
                        recipients.push(a);
                        if recipients.len() >= 5 {
                            return recipients;
                        }
                    }
                }
            }
        }
    }

    if recipients.is_empty() {
        if let Some(msg) = parsed {
            if let Some(to_header) = msg.to() {
                match to_header {
                    mail_parser::Address::List(list) => {
                        for a in list {
                            if let Some(addr) = a.address.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                                recipients.push(addr.to_string());
                                if recipients.len() >= 5 {
                                    return recipients;
                                }
                            }
                        }
                    }
                    mail_parser::Address::Group(groups) => {
                        for g in groups {
                            for a in &g.addresses {
                                if let Some(addr) = a.address.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                                    recipients.push(addr.to_string());
                                    if recipients.len() >= 5 {
                                        return recipients;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    recipients
}

/// Вырезает содержимое тегов `<style>`, `<script>`, `<head>` из HTML до преобразования в текст.
pub fn strip_html_sections(html: &str) -> String {
    let mut current = html.to_string();
    for tag in &["head", "style", "script"] {
        current = strip_tag_and_content(&current, tag);
    }
    current
}

fn strip_tag_and_content(input: &str, tag: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(open_rel_idx) = find_tag_start(rest, tag) {
        out.push_str(&rest[..open_rel_idx]);
        let after_tag_start = &rest[open_rel_idx..];

        // Ищем конец открывающего тега '>'
        let Some(tag_end_idx) = after_tag_start.find('>') else {
            rest = "";
            break;
        };

        let tag_header = &after_tag_start[..tag_end_idx];
        // Если тег самозакрывающийся (<tag ... />), вырезаем только сам тег
        if tag_header.trim_end().ends_with('/') {
            rest = &after_tag_start[tag_end_idx + 1..];
            continue;
        }

        let inner_and_rest = &after_tag_start[tag_end_idx + 1..];
        // Ищем закрывающий тег </tag...>
        if let Some(close_rel_idx) = find_closing_tag_start(inner_and_rest, tag) {
            let after_close_start = &inner_and_rest[close_rel_idx..];
            if let Some(close_end_idx) = after_close_start.find('>') {
                rest = &after_close_start[close_end_idx + 1..];
            } else {
                rest = "";
                break;
            }
        } else {
            // Закрывающий тег не найден (например, обрезанный HTML) — отбрасываем до конца
            rest = "";
            break;
        }
    }
    out.push_str(rest);
    out
}

fn find_tag_start(haystack: &str, tag: &str) -> Option<usize> {
    let bytes = haystack.as_bytes();
    let tag_bytes = tag.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let candidate = &bytes[i + 1..];
            if candidate.len() >= tag_bytes.len()
                && candidate[..tag_bytes.len()].eq_ignore_ascii_case(tag_bytes)
            {
                let next_char_idx = tag_bytes.len();
                if next_char_idx == candidate.len() {
                    return Some(i);
                }
                let next_byte = candidate[next_char_idx];
                if next_byte == b'>' || next_byte == b'/' || next_byte.is_ascii_whitespace() {
                    return Some(i);
                }
            }
        }
        i += 1;
    }
    None
}

fn find_closing_tag_start(haystack: &str, tag: &str) -> Option<usize> {
    let bytes = haystack.as_bytes();
    let tag_bytes = tag.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'<' && bytes[i + 1] == b'/' {
            let candidate = &bytes[i + 2..];
            if candidate.len() >= tag_bytes.len()
                && candidate[..tag_bytes.len()].eq_ignore_ascii_case(tag_bytes)
            {
                let next_char_idx = tag_bytes.len();
                if next_char_idx == candidate.len() {
                    return Some(i);
                }
                let next_byte = candidate[next_char_idx];
                if next_byte == b'>' || next_byte.is_ascii_whitespace() {
                    return Some(i);
                }
            }
        }
        i += 1;
    }
    None
}

/// Декодирует строку заголовка через mail-parser (обрабатывает `=?UTF-8?...?=`).
pub fn decode_rfc2047_header(raw: &str) -> String {
    if !raw.contains("=?") {
        return raw.to_string();
    }
    // Используем mail-parser для разбора синтетического заголовка
    let header_line = format!("Subject: {raw}\r\n\r\n");
    if let Some(msg) = mail_parser::MessageParser::default().parse(header_line.as_bytes()) {
        if let Some(subj) = msg.subject() {
            return subj.to_string();
        }
    }
    raw.to_string()
}

/// Подготавливает HTML для конвертации в текст:
/// - вырезает теги style, script, head вместе с содержимым;
/// - заменяет закрывающие блочные теги и ячейки таблиц (</td>, </tr>, </div>, </li>, </h1>..<h6>, <hr>)
///   на <br>, чтобы html_to_text расставлял переносы строк и вёрстка не слипалась в одну строку.
pub fn prepare_html_for_text(html: &str) -> String {
    let stripped = strip_html_sections(html);
    let with_links = expose_links(&stripped);
    replace_html_block_breaks(&with_links)
}

/// Записи ссылок `[слово](адрес)` — одним словом: для превью.
fn link_words(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let tail = &rest[open..];
        let record = tail.find("](").and_then(|middle| {
            let close = tail[middle + 2..].find(')')?;
            Some((middle, middle + 2 + close))
        });
        match record {
            Some((middle, close)) if !tail[1..middle].contains('[') => {
                out.push_str(&rest[..open]);
                out.push_str(&tail[1..middle]);
                rest = &tail[close + 1..];
            }
            _ => {
                out.push_str(&rest[..=open]);
                rest = &rest[open + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Выносит адрес ссылки в сам текст: `<a href="X">слово</a>` даёт «слово (X)».
///
/// ВАЖНО: **без этого ссылка в письме пропадает бесследно.** Превращение HTML в
/// текст выбрасывает теги вместе с их свойствами, и «unsubscribe» остаётся
/// словом, за которым ничего нет. А вынесенный в скобки адрес карточка узнаёт
/// сама — тем же `Linked`, что делает ссылки нажимаемыми в заметках.
///
/// Заодно это единственная защита от подмены: видно, куда ведёт слово
/// «Войти в аккаунт», а не только само слово.
fn expose_links(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;

    while let Some(idx) = find_tag_start(rest, "a") {
        out.push_str(&rest[..idx]);
        let tag = &rest[idx..];
        let Some(tag_end) = tag.find('>') else {
            out.push_str(tag);
            return out;
        };

        let href = href_of(&tag[..tag_end]);
        // Закрывающего нет — дальше его нет и у остальных `<a>`: остаток
        // целиком как есть. Искать заново от каждого следующего `<a>` значило
        // бы квадратичный разбор, и враждебное письмо в мегабайт висело бы
        // минутами
        let Some(close) = find_tag_start(&tag[tag_end..], "/a") else {
            out.push_str(tag);
            return out;
        };

        let inner = &tag[tag_end + 1..tag_end + close];
        /*
         * Слово в квадратных скобках, адрес в круглых — запись Markdown.
         * Карточка показывает слово, а открывает адрес; видеть его в тексте
         * незачем, ссылки в письмах бывают длиной в три строки.
         *
         * ВАЖНО: **слово расплющивается здесь, а не потом.** Внутри ссылки
         * бывает своя разметка, вплоть до таблицы, и следующий проход вставил
         * бы переносы прямо между скобками — на экране это выглядело как
         * рассыпанные по строкам «[», текст, «]» и адрес. Поэтому теги из
         * слова снимаем сразу, пробелы схлопываем, а сами скобки внутри
         * убираем: они бы оборвали запись раньше времени.
         */
        let word = flatten_label(inner);
        match href {
            // Ссылка без слова — обычно картинка. Показывать нечего, а голый
            // адрес и есть та простыня, ради избавления от которой всё это
            Some(_) if word.is_empty() => {}
            Some(link) if !word.contains(link.as_str()) => {
                // Скобки в адресе кодируются: запись `[слово](адрес)` фронт
                // режет на первой `)`, и `…/a(b)?x` открывался бы обрубком.
                // Для сервера `%28`/`%29` — те же знаки
                let link = link.replace('(', "%28").replace(')', "%29").replace(' ', "%20");
                out.push_str(&format!("[{word}]({link})"));
            }
            _ => out.push_str(&word),
        }

        rest = &tag[tag_end + close..];
    }

    out.push_str(rest);
    out
}

/// Готовит слово ссылки: снимает теги, схлопывает пробелы, убирает скобки.
fn flatten_label(inner: &str) -> String {
    let mut text = String::with_capacity(inner.len());
    let mut rest = inner;

    while let Some(idx) = rest.find('<') {
        text.push_str(&rest[..idx]);
        let Some(end) = rest[idx..].find('>') else {
            rest = "";
            break;
        };
        rest = &rest[idx + end + 1..];
    }
    text.push_str(rest);

    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(['[', ']'], "")
        .trim()
        .to_owned()
}

/// Достаёт `href` из открывающего тега ссылки. Годится только http и https:
/// по этой строке потом запускают браузер.
fn href_of(tag: &str) -> Option<String> {
    let bytes = tag.as_bytes();
    let mut at = 0;
    // Сначала имя тега, затем атрибуты целиком: href внутри data-href,
    // hreflang или чужого значения не должен становиться ссылкой.
    while at < bytes.len() && !bytes[at].is_ascii_whitespace() {
        at += 1;
    }
    while at < bytes.len() {
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        if at == bytes.len() || bytes[at] == b'>' || bytes[at] == b'/' {
            break;
        }
        let name = at;
        while at < bytes.len() && !bytes[at].is_ascii_whitespace() && !matches!(bytes[at], b'=' | b'>') {
            at += 1;
        }
        let is_href = tag[name..at].eq_ignore_ascii_case("href");
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        if at == bytes.len() || bytes[at] != b'=' {
            continue;
        }
        at += 1;
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        let quote = bytes.get(at).copied().filter(|ch| matches!(ch, b'"' | b'\''));
        if quote.is_some() {
            at += 1;
        }
        let start = at;
        while at < bytes.len() && match quote {
            Some(quote) => bytes[at] != quote,
            None => !bytes[at].is_ascii_whitespace() && bytes[at] != b'>',
        } {
            at += 1;
        }
        let link = tag[start..at].trim();
        if is_href {
            return (link.starts_with("http://") || link.starts_with("https://"))
                .then(|| link.to_owned());
        }
        if quote.is_some() && at < bytes.len() {
            at += 1;
        }
    }
    None
}

fn replace_html_block_breaks(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;

    while let Some(idx) = rest.find('<') {
        out.push_str(&rest[..idx]);
        let tag_candidate = &rest[idx..];
        let Some(end_idx) = tag_candidate.find('>') else {
            out.push_str(tag_candidate);
            return out;
        };
        let tag_content = tag_candidate[1..end_idx].trim();
        let is_block_break = if let Some(stripped_slash) = tag_content.strip_prefix('/') {
            let closing = stripped_slash.trim();
            closing.eq_ignore_ascii_case("td")
                || closing.eq_ignore_ascii_case("th")
                || closing.eq_ignore_ascii_case("tr")
                || closing.eq_ignore_ascii_case("div")
                || closing.eq_ignore_ascii_case("li")
                || closing.eq_ignore_ascii_case("h1")
                || closing.eq_ignore_ascii_case("h2")
                || closing.eq_ignore_ascii_case("h3")
                || closing.eq_ignore_ascii_case("h4")
                || closing.eq_ignore_ascii_case("h5")
                || closing.eq_ignore_ascii_case("h6")
        } else {
            tag_content.eq_ignore_ascii_case("hr")
                || tag_content.eq_ignore_ascii_case("hr/")
                || tag_content.starts_with("hr ")
                || tag_content.starts_with("hr/")
        };

        if is_block_break {
            if !out.ends_with("<br>") {
                out.push_str("<br>");
            }
        } else {
            out.push_str(&tag_candidate[..=end_idx]);
        }
        rest = &tag_candidate[end_idx + 1..];
    }
    out.push_str(rest);
    out
}

/// Извлекает исходный текст первой текстовой части письма (`text/plain` или `text/html`).
pub fn extract_raw_text(parsed: Option<&mail_parser::Message<'_>>) -> Option<String> {
    let msg = parsed?;

    // 1. Первая текстовая часть text/plain
    for part in msg.text_bodies() {
        if let mail_parser::PartType::Text(text) = &part.body {
            let s = text.trim();
            if !s.is_empty() {
                return Some(text.to_string());
            }
        }
    }

    // 2. Если text/plain нет — текст из text/html без тегов (предварительно подготовив HTML)
    for part in msg.html_bodies() {
        if let mail_parser::PartType::Html(html) = &part.body {
            let prepared = prepare_html_for_text(html);
            let plain = mail_parser::decoders::html::html_to_text(&prepared);
            let s = plain.trim();
            if !s.is_empty() {
                return Some(plain);
            }
        }
    }

    None
}

/// Раскодирует HTML-сущности, оставленные отправителем прямо в тексте.
///
/// Рассылки (Okta, например) кладут `&nbsp;` и `&amp;` и в часть `text/plain`,
/// где их никто не раскодирует, и в превью они стояли как есть. Берём
/// именованные, что встречаются в письмах, и числовые; незнакомое оставляем.
fn decode_entities(raw: &str) -> String {
    if !raw.contains('&') {
        return raw.to_owned();
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let decoded = tail.find(';').filter(|&end| end <= 10).and_then(|end| {
            let name = &tail[1..end];
            let ch = match name {
                "nbsp" => Some(Some(' ')),
                "amp" => Some(Some('&')),
                "lt" => Some(Some('<')),
                "gt" => Some(Some('>')),
                "quot" => Some(Some('"')),
                "apos" => Some(Some('\'')),
                // Невидимые разделители рассылок превью не нужны вовсе
                "zwnj" | "zwj" | "shy" => Some(None),
                _ => name
                    .strip_prefix("#x")
                    .or_else(|| name.strip_prefix("#X"))
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| name.strip_prefix('#').and_then(|dec| dec.parse::<u32>().ok()))
                    .and_then(char::from_u32)
                    .map(Some),
            };
            ch.map(|ch| (ch, end))
        });
        match decoded {
            Some((ch, end)) => {
                if let Some(ch) = ch {
                    out.push(ch);
                }
                rest = &tail[end + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Схлопывает пробелы и переносы строк в один пробел и обрезает до 180 знаков по границе символа.
pub fn clean_preview(raw: &str) -> Option<String> {
    let raw = decode_entities(raw);
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    // Обрезаем до PREVIEW_MAX_CHARS по границе символа (Unicode scalar value), а не байта
    let truncated: String = collapsed.chars().take(PREVIEW_MAX_CHARS).collect();
    Some(truncated)
}

/// Очищает полный текст письма:
/// - вырезает теги `<style>`, `<script>`, `<head>`, если они ещё есть в тексте;
/// - выбрасывает строки, состоящие из одних разделителей (`-`, `=`, `_`, `*`, `|`, пробелы);
/// - схлопывает две и больше подряд идущих пустых строк в одну;
/// - схлопывает подряд идущие одинаковые строки в одну;
/// - склеивает короткие обрывки в абзац (если строка не кончается на `.`, `!`, `?`, `:`,
///   а следующая начинается со строчной буквы);
/// - убирает пустые строки в начале и в конце;
/// - ограничивает длину 10 000 знаками по границе символа.
///
/// Пустой текст даёт `None`.
pub fn clean_body(raw: &str) -> Option<String> {
    clean_body_full(raw).map(|text| text.chars().take(BODY_MAX_CHARS).collect())
}

fn clean_body_full(raw: &str) -> Option<String> {
    let raw = &decode_entities(raw);
    // 1. Вырезаем style, script, head, если они присутствуют
    let stripped = if raw.contains('<') {
        strip_html_sections(raw)
    } else {
        raw.to_string()
    };

    // 2. Обрабатываем строки: фильтруем строки-разделители
    let mut initial_lines = Vec::new();
    for line in stripped.lines() {
        let trimmed = line.trim();
        if is_delimiter_line(trimmed) {
            continue;
        }
        initial_lines.push(trimmed.to_string());
    }

    // 3. Схлопываем подряд идущие одинаковые строки (включая пустые строки)
    let mut deduped_lines = Vec::new();
    for line in initial_lines {
        if deduped_lines.last() != Some(&line) {
            deduped_lines.push(line);
        }
    }

    // 4. Склеиваем обрывки в абзацы:
    // если предыдущая строка не кончается на '.', '!', '?', ':',
    // а текущая начинается со строчной буквы — склеиваем их через пробел
    let mut glued_lines: Vec<String> = Vec::new();
    for line in deduped_lines {
        if line.is_empty() {
            glued_lines.push(line);
            continue;
        }

        if let Some(prev) = glued_lines.last_mut() {
            let should_glue = !prev.is_empty()
                && !prev.ends_with(['.', '!', '?', ':'])
                && line.chars().next().is_some_and(|c| c.is_lowercase());

            if should_glue {
                prev.push(' ');
                prev.push_str(&line);
                continue;
            }
        }

        glued_lines.push(line);
    }

    // 5. Убираем пустые строки в начале и в конце
    while glued_lines.first().is_some_and(|s| s.is_empty()) {
        glued_lines.remove(0);
    }
    while glued_lines.last().is_some_and(|s| s.is_empty()) {
        glued_lines.pop();
    }

    if glued_lines.is_empty() {
        return None;
    }

    let joined = glued_lines.join("\n");
    if joined.trim().is_empty() {
        return None;
    }

    Some(joined)
}

/// Проверяет, состоит ли строка исключительно из разделителей (`-`, `=`, `_`, `*`, `|`, пробелы).
fn is_delimiter_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    trimmed.chars().all(|c| matches!(c, '-' | '=' | '_' | '*' | '|' | ' ' | '\t'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn href_is_a_whole_attribute_outside_other_values() {
        for tag in [
            r#"<a hreflang="en" href="https://example.org/right""#,
            r#"<a data-href="https://example.org/wrong" HREF = 'https://example.org/right'"#,
            r#"<a title=" href='https://example.org/wrong'" href=https://example.org/right"#,
            r#"<a disabled href = "https://example.org/right""#,
        ] {
            assert_eq!(href_of(tag).as_deref(), Some("https://example.org/right"), "{tag}");
        }
        for tag in [
            r#"<a hreflang="https://example.org/wrong""#,
            r#"<a data-href="https://example.org/wrong""#,
            r#"<a href-other="https://example.org/wrong""#,
            r#"<a title="href=https://example.org/wrong""#,
            r#"<a href="javascript:alert(1)""#,
        ] {
            assert_eq!(href_of(tag), None, "{tag}");
        }
    }

    #[test]
    fn command_errors_use_catalog_without_server_details() {
        assert_eq!(MailError::NoAccount.user_message(), tr!("mail.noAccount"));
        assert_eq!(MailError::Auth("AUTH detail".into()).user_message(), tr!("mail.signInFailed"));
        assert_eq!(MailError::Network("NETWORK detail".into()).user_message(), tr!("mail.unavailable"));
        assert_eq!(MailError::Stale.user_message(), tr!("mail.error.stale"));
        assert!(MailError::Network("NETWORK detail".into()).to_string().contains("NETWORK detail"));
    }

    /// Сервер в памяти говорит с настоящим парсером async-imap. Литерал тела
    /// тоже проходит протокол, поэтому строка, похожая на тег, не завершает FETCH.
    async fn fetch_session(
        command: &str,
        data: &str,
        status: Option<&str>,
    ) -> (async_imap::Session<tokio::io::DuplexStream>, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        let (client_io, server_io) = tokio::io::duplex(4096);
        let command = command.to_owned();
        let data = data.to_owned();
        let status = status.map(str::to_owned);
        let server = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(server_io);
            let mut reader = BufReader::new(reader);
            writer.write_all(b"* OK ready\r\n").await.unwrap();
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let tag = line.split_whitespace().next().unwrap();
            writer.write_all(format!("{tag} OK logged in\r\n").as_bytes()).await.unwrap();
            line.clear();
            reader.read_line(&mut line).await.unwrap();
            let (tag, sent) = line.trim_end().split_once(' ').unwrap();
            assert_eq!(sent, command);
            writer.write_all(data.as_bytes()).await.unwrap();
            if let Some(status) = status {
                writer.write_all(format!("{tag} {status} [UNAVAILABLE] finished\r\n").as_bytes()).await.unwrap();
            }
        });
        let mut client = async_imap::Client::new(client_io);
        client.read_response().await.unwrap().unwrap();
        let session = client.login("user", "password").await.unwrap();
        (session, server)
    }

    /// Размер приходит тем же FETCH; без него граница выборки считается обрывом.
    #[tokio::test]
    async fn fetched_body_reports_limits_and_preserves_the_flag_on_broken_mime() {
        let small = "Subject: Sample\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nshort".to_owned();
        let mut boundary = format!("X-Padding: {}\r\n\r\nshort", "p".repeat(BODY_LIMIT as usize - 20));
        boundary.push_str(&" ".repeat(BODY_LIMIT as usize - boundary.len()));
        for (bytes, size, expected) in [
            (small.as_str(), Some(small.len() as u32), false),
            (small.as_str(), Some(BODY_LIMIT + 1), true),
            (small.as_str(), None, false),
            (boundary.as_str(), None, true),
            (boundary.as_str(), Some(BODY_LIMIT), false),
            ("", Some(BODY_LIMIT + 1), true),
        ] {
            let command = format!("UID FETCH 42 (RFC822.SIZE BODY.PEEK[]<0.{BODY_LIMIT}>)");
            let size_field = size.map(|size| format!("RFC822.SIZE {size} ")).unwrap_or_default();
            let data = format!("* 1 FETCH (UID 42 {size_field}BODY[]<0> {{{}}}\r\n{bytes})\r\n", bytes.len());
            let (mut session, server) = fetch_session(&command, &data, Some("OK")).await;
            let bodies = checked_fetch(&mut session, &command, fetched_letter_body).await.unwrap();
            assert_eq!(bodies.len(), 1);
            assert_eq!(bodies[0].truncated, expected, "размер: {size:?}");
            assert_eq!(serde_json::to_value(&bodies[0]).unwrap()["truncated"], expected);
            server.await.unwrap();
        }
    }

    #[test]
    fn parsed_body_reports_the_text_limit_only_when_text_was_cut() {
        for length in [BODY_MAX_CHARS, BODY_MAX_CHARS + 1] {
            let raw = format!("Content-Type: text/plain; charset=utf-8\r\n\r\n{}", "я".repeat(length));
            let msg = mail_parser::MessageParser::default().parse(raw.as_bytes()).unwrap();
            let body = parse_letter_body(&msg);
            assert_eq!(body.truncated, length > BODY_MAX_CHARS);
            assert_eq!(body.text.unwrap().chars().count(), BODY_MAX_CHARS);
        }
    }

    /// Частичный список, номера, поиск и тело обязаны дождаться своего OK.
    /// Даже уже полученные данные не дают успеха при NO/BAD, BYE или обрыве.
    #[tokio::test]
    async fn every_fetch_requires_a_successful_tagged_completion() {
        let body = "Subject: Sample\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nA0002 OK inside the body";
        let data = format!("* 1 FETCH (UID 42 FLAGS (\\Seen) INTERNALDATE \"17-Jul-2026 12:30:00 -0430\" BODY[]<0> {{{}}}\r\n{body})\r\n", body.len());
        for command in [
            "UID FETCH 42,43 (UID FLAGS ENVELOPE INTERNALDATE)",
            "FETCH 1:* UID",
            "UID FETCH 42,43 (UID FLAGS ENVELOPE INTERNALDATE BODY.PEEK[]<0.65536>)",
            "UID FETCH 42 (BODY.PEEK[]<0.1048576>)",
        ] {
            for status in [Some("OK"), Some("NO"), Some("BAD"), None] {
                let (mut session, server) = fetch_session(command, &data, status).await;
                let result = tokio::time::timeout(Duration::from_secs(2), checked_fetch(&mut session, command, |fetch| {
                    let letter = parse_letter(fetch, "imap.example.org", None);
                    Some((letter, fetch.body().unwrap().to_vec()))
                })).await.expect("FETCH должен завершиться");
                if status == Some("OK") {
                    let values = result.unwrap();
                    assert_eq!(values.len(), 1);
                    assert_eq!(values[0].0.uid, 42);
                    assert!(values[0].0.seen);
                    assert_eq!(values[0].0.subject, "Sample");
                    assert_eq!(values[0].0.at, 1_784_307_600);
                    assert_eq!(values[0].1, body.as_bytes());
                } else {
                    assert!(matches!(result, Err(MailError::Network(_))), "{command}: {status:?}");
                }
                server.await.unwrap();
            }
        }

        for (data, status) in [
            ("", Some("OK")),
            ("* 1 FETCH (UID 42)\r\n* BYE disconnected\r\n", None),
            ("* 1 FETCH (UID 42)\r\nwrong OK wrong tag\r\n", None),
            ("* 1 FETCH (UID 42)\r\n* 2 FETCH (UID", None),
        ] {
            let command = "FETCH 1:* UID";
            let (mut session, server) = fetch_session(command, data, status).await;
            let result = tokio::time::timeout(Duration::from_secs(2), checked_fetch(&mut session, command, |fetch| fetch.uid()))
                .await.expect("FETCH должен завершиться");
            assert!(matches!(result, Err(MailError::Network(_))), "{data}: {status:?}");
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn checked_fetch_preserves_envelope_fields_and_timezone() {
        let command = "UID FETCH 42 (UID FLAGS ENVELOPE INTERNALDATE)";
        let data = concat!(
            "* 1 FETCH (UID 42 FLAGS () INTERNALDATE \"17-Jul-2026 12:30:00 +0200\" ",
            "ENVELOPE (NIL \"Subject\" ((\"Sender\" NIL \"sender\" \"example.org\")) NIL NIL ",
            "((NIL NIL \"reader\" \"example.org\")) NIL NIL NIL \"<sample@example.org>\"))\r\n",
        );
        let (mut session, server) = fetch_session(command, data, Some("OK")).await;
        let letters = checked_fetch(&mut session, command, |fetch| Some(parse_letter(fetch, "imap.example.org", None))).await.unwrap();
        assert_eq!(letters.len(), 1);
        assert_eq!(letters[0].subject, "Subject");
        assert_eq!(letters[0].from, "Sender");
        assert_eq!(letters[0].from_address.as_deref(), Some("sender@example.org"));
        assert_eq!(letters[0].at, 1_784_284_200);
        assert!(!letters[0].seen);
        server.await.unwrap();

        let (mut session, server) = fetch_session(command, data, Some("OK")).await;
        let skipped = checked_fetch(&mut session, command, |_| None::<()>).await.unwrap();
        assert!(skipped.is_empty());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn last_uids_reject_a_partial_failed_fetch() {
        for status in [Some("OK"), Some("NO"), Some("BAD"), None] {
            let (mut session, server) = fetch_session("FETCH 1:* UID", "* 1 FETCH (UID 42)\r\n", status).await;
            let result = tokio::time::timeout(Duration::from_secs(2), get_last_uids(&mut session, 10))
                .await.expect("FETCH UID должен завершиться");
            assert_eq!(result.is_ok(), status == Some("OK"));
            if let Ok(uids) = result {
                assert_eq!(uids, vec![42]);
            }
            server.await.unwrap();
        }
    }

    #[test]
    fn uid_validity_requires_the_same_known_generation() {
        assert_eq!(check_uid_validity(Some(77), Some(77)), Ok(()));
        assert_eq!(check_uid_validity(Some(77), Some(78)), Err(MailError::Stale));
        assert_eq!(check_uid_validity(Some(77), None), Ok(()));
        assert_eq!(check_uid_validity(None, Some(78)), Ok(()));
        assert_eq!(check_uid_validity(None, None), Ok(()));
    }

    /// Ответ разбирает настоящий клиент, транспорт — память, без сети и TLS.
    /// При смене поколения после открытия папки больше не уходит ни одной команды.
    #[tokio::test]
    async fn mailbox_responses_guard_uids_before_any_action() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        for writable in [false, true] {
            for actual in [Some(77), Some(u32::MAX), None] {
                let (client_io, server_io) = tokio::io::duplex(4096);
                let server = tokio::spawn(async move {
                    let (reader, mut writer) = tokio::io::split(server_io);
                    let mut reader = BufReader::new(reader);
                    writer.write_all(b"* OK ready\r\n").await.unwrap();
                    let mut line = String::new();
                    reader.read_line(&mut line).await.unwrap();
                    let tag = line.split_whitespace().next().unwrap();
                    writer.write_all(format!("{tag} OK logged in\r\n").as_bytes()).await.unwrap();
                    line.clear();
                    reader.read_line(&mut line).await.unwrap();
                    let command = if writable { "SELECT \"INBOX\"" } else { "EXAMINE \"INBOX\"" };
                    assert!(line.contains(command), "{line}");
                    let tag = line.split_whitespace().next().unwrap();
                    let generation = actual.map(|value| format!("* OK [UIDVALIDITY {value}] generation\r\n")).unwrap_or_default();
                    writer.write_all(format!("* 1 EXISTS\r\n* OK [UIDNEXT 43] next\r\n{generation}{tag} OK opened\r\n").as_bytes()).await.unwrap();
                    line.clear();
                    assert_eq!(reader.read_line(&mut line).await.unwrap(), 0, "лишняя команда: {line}");
                });
                let mut client = async_imap::Client::new(client_io);
                client.read_response().await.unwrap().unwrap();
                let mut session = client.login("user", "password").await.unwrap();
                let result = checked_inbox(&mut session, Some(77), writable).await;
                assert_eq!(result, if actual == Some(u32::MAX) { Err(MailError::Stale) } else { Ok(()) });
                drop(session);
                tokio::time::timeout(Duration::from_secs(2), server).await.unwrap().unwrap();
            }
        }
    }


    /// Проверяем протокол на настоящем клиенте библиотеки: промежуточный FETCH
    /// не заменяет финальный OK, а NO/BAD и оборванный ответ не дают успеха.
    #[tokio::test]
    async fn store_flags_checks_the_complete_response() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        for status in [Some("OK"), Some("NO"), Some("BAD"), None] {
            let (client_io, server_io) = tokio::io::duplex(4096);
            let server = tokio::spawn(async move {
                let (reader, mut writer) = tokio::io::split(server_io);
                let mut reader = BufReader::new(reader);
                writer.write_all(b"* OK ready\r\n").await.unwrap();
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                let tag = line.split_whitespace().next().unwrap();
                writer.write_all(format!("{tag} OK logged in\r\n").as_bytes()).await.unwrap();
                line.clear();
                reader.read_line(&mut line).await.unwrap();
                assert!(line.contains("UID STORE 42 +FLAGS.SILENT (\\Seen)"));
                writer.write_all(b"* 1 FETCH (UID 42 FLAGS (\\Seen))\r\n").await.unwrap();
                if let Some(status) = status {
                    let tag = line.split_whitespace().next().unwrap();
                    writer.write_all(format!("{tag} {status} finished\r\n").as_bytes()).await.unwrap();
                }
            });
            let mut client = async_imap::Client::new(client_io);
            client.read_response().await.unwrap().unwrap();
            let mut session = client.login("user", "password").await.unwrap();
            let result = tokio::time::timeout(Duration::from_secs(2), store_flags(&mut session, "42", "\\Seen"))
                .await.expect("ответ должен завершиться");
            assert_eq!(result.is_ok(), status == Some("OK"), "{status:?}: {result:?}");
            server.await.unwrap();
        }
    }


    /// Разметка внутри ссылки не должна рвать запись на строки: именно это
    /// однажды и рассыпало по экрану «[», текст, «]» и адрес порознь.
    #[test]
    fn a_link_with_markup_inside_stays_on_one_line() {
        let html = concat!(
            "<a href=\"https://example.com/x\">",
            "<table><tr><td>
  How to
  </td><td>read this</td></tr></table>",
            "</a>"
        );
        let out = prepare_html_for_text(html);
        assert!(
            out.contains("[How to read this](https://example.com/x)"),
            "{out}"
        );
        // Второй проверки не нужно: перенос внутри записи оборвал бы саму
        // подстроку выше, и она бы не нашлась
    }

    /// Ссылка без слова — обычно картинка. Голый адрес на экране и есть та
    /// простыня, ради избавления от которой всё затевалось.
    #[test]
    fn a_link_without_a_word_leaves_nothing() {
        let html = "<a href=\"https://example.com/pixel\"><img src=\"https://x/y.png\"></a>";
        let out = prepare_html_for_text(html);
        assert!(!out.contains("https://example.com/pixel"), "{out}");
    }

    /// Скобки внутри слова оборвали бы запись раньше времени.
    #[test]
    fn brackets_inside_a_word_are_dropped() {
        let html = "<a href=\"https://example.com/a\">[sale] today</a>";
        let out = prepare_html_for_text(html);
        assert!(out.contains("[sale today](https://example.com/a)"), "{out}");
    }

    #[test]
    fn collapses_spaces_and_newlines() {
        let input = "  Первая строка\n\n\tВторая строка   с пробелами   \r\nТретья строка  ";
        let cleaned = clean_preview(input).expect("превью получено");
        assert_eq!(cleaned, "Первая строка Вторая строка с пробелами Третья строка");
    }

    #[test]
    fn trims_to_180_chars_on_char_boundary() {
        // 200 русских букв (каждая по 2 байта в UTF-8)
        let input = "А".repeat(200);
        let cleaned = clean_preview(&input).expect("превью получено");
        assert_eq!(cleaned.chars().count(), 180);
        assert_eq!(cleaned.len(), 360); // 180 * 2 байта
        assert_eq!(cleaned, "А".repeat(180));
    }

    #[test]
    fn handles_multibyte_emoji_without_breaking() {
        // Эмодзи по 4 байта
        let input = "🦀".repeat(200);
        let cleaned = clean_preview(&input).expect("превью получено");
        assert_eq!(cleaned.chars().count(), 180);
        assert_eq!(cleaned, "🦀".repeat(180));
    }

    #[test]
    fn entities_left_in_plain_text_are_decoded() {
        assert_eq!(
            clean_preview("&nbsp; Activate&nbsp;account &amp; more &#8212; &#x41;&zwnj;B").as_deref(),
            Some("Activate account & more \u{2014} AB")
        );
        // Незнакомое и голый амперсанд остаются как были
        assert_eq!(decode_entities("Tom & Jerry &bogus; &"), "Tom & Jerry &bogus; &");
    }

    #[test]
    fn empty_preview_returns_none() {
        assert_eq!(clean_preview(""), None);
        assert_eq!(clean_preview("   \n\t  \r\n  "), None);
    }

    #[test]
    fn decodes_rfc2047_subject() {
        let encoded = "=?UTF-8?B?0J/RgNC40LLQtdGCINC80LjRgA==?=";
        let decoded = decode_rfc2047_header(encoded);
        assert_eq!(decoded, "Привет мир");
    }

    #[test]
    fn extracts_preview_from_raw_mime_message() {
        let raw = concat!(
            "From: =?UTF-8?B?0JjQstCw0L0=?= <ivan@example.org>\r\n",
            "Subject: =?UTF-8?B?0KLQtdGB0YI=?=\r\n",
            "Content-Type: text/plain; charset=utf-8\r\n",
            "\r\n",
            "Привет,\n\nэто тестовое письмо   с   пробелами.\n"
        );
        let msg = mail_parser::MessageParser::default().parse(raw.as_bytes()).expect("разобрано");
        // Тот же путь, что у опроса: сырой текст, потом чистка превью
        let preview = extract_raw_text(Some(&msg)).as_deref().and_then(clean_preview).expect("превью извлечено");
        assert_eq!(preview, "Привет, это тестовое письмо с пробелами.");
    }

    #[test]
    fn extracts_preview_from_html_when_no_text_plain() {
        let raw = concat!(
            "From: Sender <sender@example.org>\r\n",
            "Subject: HTML mail\r\n",
            "Content-Type: text/html; charset=utf-8\r\n",
            "\r\n",
            "<html><body><h1>Заголовок</h1><p>Текст в абзаце.</p></body></html>"
        );
        let msg = mail_parser::MessageParser::default().parse(raw.as_bytes()).expect("разобрано");
        let preview = extract_raw_text(Some(&msg)).as_deref().and_then(clean_preview).expect("превью извлечено");
        assert!(preview.contains("Заголовок"));
        assert!(preview.contains("Текст в абзаце."));
        assert!(!preview.contains("<h1>"));
    }

    /// Настоящее письмо — многочастное, и шестнадцать килобайт режут его посреди
    /// части. Проверяем, что из такого письма вообще что-то достаётся.
    #[test]
    fn truncated_multipart_still_gives_text() {
        let boundary = "----=_Part_42";
        let mut raw = String::new();
        raw.push_str("From: Sender <s@example.org>
");
        raw.push_str("Subject: Real world
");
        raw.push_str(&format!(
            "Content-Type: multipart/alternative; boundary=\"{boundary}\"

"
        ));
        raw.push_str(&format!("--{boundary}
"));
        raw.push_str("Content-Type: text/plain; charset=utf-8

");
        raw.push_str("Вот текст письма, он короткий.

");
        raw.push_str(&format!("--{boundary}
"));
        raw.push_str("Content-Type: text/html; charset=utf-8

");
        raw.push_str("<html><body><p>");
        raw.push_str(&"long html ".repeat(4000));

        // Обрываем, как обрывает выборка в 16 КБ: закрывающей границы нет
        let cut = &raw.as_bytes()[..16384.min(raw.len())];
        let msg = mail_parser::MessageParser::default()
            .parse(cut)
            .expect("разобрано хоть как-то");
        let text = extract_raw_text(Some(&msg));
        assert!(
            text.is_some(),
            "из оборванного многочастного письма не достали ни строчки"
        );
        assert!(text.unwrap().contains("Вот текст письма"));
    }

    /// То же письмо целиком — чтобы отличить беду обрыва от беды разбора.
    #[test]
    fn whole_multipart_gives_text() {
        let boundary = "----=_Part_42";
        let raw = format!(
            "From: Sender <s@example.org>
Subject: Real world
Content-Type: multipart/alternative; boundary=\"{boundary}\"

--{boundary}
Content-Type: text/plain; charset=utf-8

Вот текст письма.
--{boundary}
Content-Type: text/html; charset=utf-8

<p>html</p>
--{boundary}--
"
        );
        let msg = mail_parser::MessageParser::default()
            .parse(raw.as_bytes())
            .expect("разобрано");
        assert!(extract_raw_text(Some(&msg)).is_some(), "целое письмо тоже не разобралось");
    }

    #[tokio::test]
    async fn fetch_inbox_with_empty_credentials_returns_no_account() {
        let err = fetch_inbox("", 993, "", "", None, true).await.unwrap_err();
        assert_eq!(err, MailError::NoAccount);
    }

    /// Порт, который никто не слушает, — отказ на шаге соединения, а не входа:
    /// чинить тут порт или сеть, а не пароль.
    #[tokio::test]
    async fn a_closed_port_fails_at_connect() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("порт выдан");
        let port = listener.local_addr().expect("адрес есть").port();
        drop(listener);

        match check_account("127.0.0.1", port, "me", "secret").await {
            Check::Failed { stage, timed_out, .. } => {
                assert_eq!(stage, Stage::Connect);
                assert!(!timed_out);
            }
            other => panic!("ожидался отказ соединения, пришло {other:?}"),
        }
    }

    /// Сервер без TLS — отказ на шаге TLS. Так выглядит порт 143 вместо 993.
    #[tokio::test]
    async fn a_plain_server_fails_at_tls() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("порт выдан");
        let port = listener.local_addr().expect("адрес есть").port();
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                use tokio::io::AsyncWriteExt;
                let _ = socket.write_all(b"* OK IMAP4rev1 ready\r\n").await;
            }
        });

        match check_account("127.0.0.1", port, "me", "secret").await {
            Check::Failed { stage, .. } => assert_eq!(stage, Stage::Tls),
            other => panic!("ожидался отказ TLS, пришло {other:?}"),
        }
    }

    #[test]
    fn clean_body_preserves_newlines_and_collapses_empty_lines() {
        let input = "Первый абзац   \r\n\r\n\r\nВторой абзац  \n   \n\nТретий абзац\n";
        let cleaned = clean_body(input).expect("текст получен");
        assert_eq!(cleaned, "Первый абзац\n\nВторой абзац\n\nТретий абзац");
    }

    #[test]
    fn clean_body_removes_trailing_spaces_per_line() {
        let input = "Строка 1    \nСтрока 2\t\t  \nСтрока 3";
        let cleaned = clean_body(input).expect("текст получен");
        assert_eq!(cleaned, "Строка 1\nСтрока 2\nСтрока 3");
    }

    #[test]
    fn clean_body_trims_to_10000_chars_on_char_boundary() {
        let input = "Б".repeat(12_000);
        let cleaned = clean_body(&input).expect("текст получен");
        assert_eq!(cleaned.chars().count(), 10_000);
        assert_eq!(cleaned.len(), 20_000); // 10000 * 2 байта
        assert_eq!(cleaned, "Б".repeat(10_000));
    }

    #[test]
    fn clean_body_empty_returns_none() {
        assert_eq!(clean_body(""), None);
        assert_eq!(clean_body("   \r\n\t  \n  "), None);
    }

    #[test]
    fn clean_body_strips_style_script_and_head_tags() {
        let input = "<head><title>Тест</title></head><style>body { color: red; }</style><script>alert(1);</script>Основной текст письма";
        let cleaned = clean_body(input).expect("текст получен");
        assert_eq!(cleaned, "Основной текст письма");
        assert!(!cleaned.contains("color: red"));
        assert!(!cleaned.contains("alert"));
        assert!(!cleaned.contains("Тест"));
    }

    #[test]
    fn clean_body_discards_delimiter_lines() {
        let input = "Текст до разделителя\n---\n=====\n_ _ _\n***\n|---|---|\nТекст после разделителя";
        let cleaned = clean_body(input).expect("текст получен");
        assert_eq!(cleaned, "Текст до разделителя\nТекст после разделителя");
    }

    #[test]
    fn clean_body_collapses_multiple_empty_lines_into_one() {
        let input = "Первый абзац\n\n\n\n\nВторой абзац";
        let cleaned = clean_body(input).expect("текст получен");
        assert_eq!(cleaned, "Первый абзац\n\nВторой абзац");
    }

    #[test]
    fn clean_body_collapses_consecutive_identical_lines() {
        let input = "Заголовок рассылки\nЗаголовок рассылки\nСвежие новости\nСвежие новости\nСвежие новости";
        let cleaned = clean_body(input).expect("текст получен");
        assert_eq!(cleaned, "Заголовок рассылки\nСвежие новости");
    }

    #[test]
    fn clean_body_glues_broken_fragments_into_paragraph() {
        let input = "Это длинное предложение,\nкоторое было разорвано\nвёрсткой таблицы.\nНовое предложение:\nследующая строка со строчной не склеивается с двоеточием.";
        let cleaned = clean_body(input).expect("текст получен");
        assert_eq!(
            cleaned,
            "Это длинное предложение, которое было разорвано вёрсткой таблицы.\nНовое предложение:\nследующая строка со строчной не склеивается с двоеточием."
        );

        // Знаки конца предложения (?, !, .) не дают склеивать следующую строку со строчной
        let punct_input = "Вопрос?\nответ\nВосклицание!\nпродолжение\nТочка.\nпродолжение";
        let punct_cleaned = clean_body(punct_input).expect("текст получен");
        assert_eq!(punct_cleaned, "Вопрос?\nответ\nВосклицание!\nпродолжение\nТочка.\nпродолжение");
    }

    #[test]
    fn clean_body_trims_leading_and_trailing_empty_lines() {
        let input = "\n\n   \n\nПолезный текст\n\n   \n\n";
        let cleaned = clean_body(input).expect("текст получен");
        assert_eq!(cleaned, "Полезный текст");
    }

    #[test]
    fn clean_body_real_world_table_email_turns_into_readable_paragraphs() {
        let html = concat!(
            "<html><head><style>td { padding: 4px; } .cls { display: block; }</style></head>",
            "<body><table>",
            "<tr><td>Уведомление о заказе</td></tr>",
            "<tr><td>Уведомление о заказе</td></tr>",
            "<tr><td>----------------------------------</td></tr>",
            "<tr><td>Здравствуйте, Иван! Ваш заказ был успешно</td></tr>",
            "<tr><td>оформлен и уже передан</td></tr>",
            "<tr><td>в службу курьерской доставки.</td></tr>",
            "<tr><td>==================================</td></tr>",
            "<tr><td>Курьер свяжется с вами за час.</td></tr>",
            "<tr><td>С уважением,</td></tr>",
            "<tr><td>служба заботы о клиентах.</td></tr>",
            "<tr><td>|---|---|</td></tr>",
            "<tr><td>Поддержка: 8-800-000-00-00</td></tr>",
            "<tr><td>Поддержка: 8-800-000-00-00</td></tr>",
            "</table></body></html>"
        );

        let formatted = format!("Content-Type: text/html; charset=utf-8\r\n\r\n{html}");
        let msg = mail_parser::MessageParser::default()
            .parse(formatted.as_bytes())
            .expect("разобрано");

        let raw = extract_raw_text(Some(&msg)).expect("извлечено");
        let body = clean_body(&raw).expect("очищено");

        assert!(!body.contains("<style>"));
        assert!(!body.contains("padding: 4px"));
        assert!(!body.contains("---"));
        assert!(!body.contains("==="));
        assert!(body.contains("Здравствуйте, Иван! Ваш заказ был успешно оформлен и уже передан в службу курьерской доставки."));
        assert!(body.contains("С уважением, служба заботы о клиентах."));
        assert!(body.contains("Курьер свяжется с вами за час."));
        assert_eq!(body.matches("Уведомление о заказе").count(), 1);
        assert_eq!(body.matches("Поддержка: 8-800-000-00-00").count(), 1);
    }

    #[test]
    fn escape_imap_string_escapes_quotes_and_backslashes() {
        assert_eq!(escape_imap_string("hello"), r#""hello""#);
        assert_eq!(escape_imap_string(r#"Ivan "The Great""#), r#""Ivan \"The Great\"""#);
        assert_eq!(escape_imap_string(r"C:\docs\file.txt"), r#""C:\\docs\\file.txt""#);
        assert_eq!(escape_imap_string("foo\r\nbar"), r#""foo  bar""#);
    }

    #[test]
    fn detect_attachment_kind_identifies_formats() {
        assert_eq!(detect_attachment_kind("doc.pdf", None), "pdf");
        assert_eq!(detect_attachment_kind("IMG_01.PNG", None), "png");
        assert_eq!(detect_attachment_kind("archive.zip", None), "zip");
        assert_eq!(detect_attachment_kind("photo.jpeg", None), "jpg");
        assert_eq!(detect_attachment_kind("unknown.xyz123", None), "file");
        assert_eq!(detect_attachment_kind("noextension", None), "file");
    }

    #[test]
    fn extract_attachments_filters_inline_images() {
        let boundary = "----=_Part_Att_Test";
        let raw = format!(
            concat!(
                "From: Sender <s@example.org>\r\n",
                "Subject: Attachments test\r\n",
                "Content-Type: multipart/mixed; boundary=\"{boundary}\"\r\n",
                "\r\n",
                "--{boundary}\r\n",
                "Content-Type: text/plain\r\n",
                "\r\n",
                "Привет!\r\n",
                "--{boundary}\r\n",
                "Content-Type: application/pdf; name=\"report.pdf\"\r\n",
                "Content-Disposition: attachment; filename=\"report.pdf\"\r\n",
                "Content-Transfer-Encoding: base64\r\n",
                "\r\n",
                "cGRmZGF0YQ==\r\n",
                "--{boundary}\r\n",
                "Content-Type: image/png\r\n",
                "Content-Disposition: inline\r\n",
                "Content-ID: <logo@domain.com>\r\n",
                "Content-Transfer-Encoding: base64\r\n",
                "\r\n",
                "aW1hZ2VkYXRh\r\n",
                "--{boundary}--\r\n"
            ),
            boundary = boundary
        );

        let msg = mail_parser::MessageParser::default().parse(raw.as_bytes()).expect("разобрано");
        let parsed = parse_letter_body(&msg);
        assert_eq!(parsed.text, Some("Привет!".to_string()));
        assert_eq!(parsed.attachments.len(), 1);
        assert_eq!(parsed.attachments[0].name, "report.pdf");
        assert_eq!(parsed.attachments[0].kind, "pdf");
        assert!(parsed.attachments[0].size > 0);
    }

    #[test]
    fn extract_addresses_from_parsed_message() {
        let raw = concat!(
            "From: =?UTF-8?B?0JjQstCw0L0=?= <ivan@example.org>\r\n",
            "To: a@example.org, b@example.org, c@example.org, d@example.org, e@example.org, f@example.org\r\n",
            "Subject: Test\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "Тест\r\n"
        );
        let msg = mail_parser::MessageParser::default().parse(raw.as_bytes()).expect("разобрано");
        let from_addr = extract_from_address(None, Some(&msg));
        assert_eq!(from_addr, Some("ivan@example.org".to_string()));
        let to = extract_to(None, Some(&msg));
        // Не больше 5 получателей
        assert_eq!(to.len(), 5);
        assert_eq!(to, vec!["a@example.org", "b@example.org", "c@example.org", "d@example.org", "e@example.org"]);
    }

    #[tokio::test]
    async fn search_inbox_with_empty_query_returns_empty_inbox_without_network() {
        let res = search_inbox(
            "example.com",
            993,
            "login",
            "password",
            SearchCriteria {
                query: "   ",
                unseen_only: false,
            },
            Some("https://mail.example.com"),
            true,
        )
        .await
        .expect("пустой результат без ошибки");
        assert_eq!(res.unseen, 0);
        assert!(res.letters.is_empty());
        assert_eq!(res.mailbox, Some("https://mail.example.com".to_string()));
    }

    /// Пустой запрос при включённом фильтре — это не «искать нечего»: сходить
    /// за непрочитанными надо, и учётные данные для этого нужны.
    #[tokio::test]
    async fn a_bare_unseen_filter_still_needs_an_account() {
        let criteria = SearchCriteria {
            query: "",
            unseen_only: true,
        };
        let err = search_inbox("", 993, "", "", criteria, None, true).await.unwrap_err();
        assert_eq!(err, MailError::NoAccount);
    }

    #[tokio::test]
    async fn search_inbox_with_empty_credentials_returns_no_account() {
        let criteria = SearchCriteria {
            query: "запрос",
            unseen_only: false,
        };
        let err = search_inbox("", 993, "", "", criteria, None, true).await.unwrap_err();
        assert_eq!(err, MailError::NoAccount);
    }

    #[test]
    fn brackets_in_a_link_do_not_cut_the_record_short() {
        let out = prepare_html_for_text(r#"<a href="https://example.org/a(b)?x=1">Open</a>"#);
        assert!(out.contains("[Open](https://example.org/a%28b%29?x=1)"), "{out}");
    }

    #[test]
    fn the_preview_shows_link_words_without_addresses() {
        assert_eq!(link_words("Hi [Open](https://example.com/x) now [a] b"), "Hi Open now [a] b");
    }

    #[test]
    fn a_busy_server_is_not_a_wrong_password() {
        assert!(transient_refusal("[UNAVAILABLE] Try again later"));
        assert!(!transient_refusal("[AUTHENTICATIONFAILED] Invalid credentials"));
    }

    /// День из одной цифры IMAP пишет с пробелом впереди (`date-day-fixed`):
    /// « 7-Jul-2026». Дата письма обязана читаться и так, и с минусом пояса.
    #[test]
    fn imap_dates_read_with_a_padded_day_and_a_negative_zone() {
        use async_imap::imap_proto::types::AttributeValue;
        use std::borrow::Cow;
        let date = |raw: &'static str| {
            let attrs = [AttributeValue::InternalDate(Cow::Borrowed(raw))];
            Fetched(&attrs).internal_date()
        };
        let padded = date(" 7-Jul-2026 12:30:00 +0000").expect("день с пробелом впереди");
        let plain = date("07-Jul-2026 12:30:00 +0000").expect("день с нулём впереди");
        assert_eq!(padded, plain);
        let west = date("17-Jul-2026 12:30:00 -0430").expect("отрицательный пояс");
        let utc = date("17-Jul-2026 17:00:00 +0000").unwrap();
        assert_eq!(west, utc);
    }
}
