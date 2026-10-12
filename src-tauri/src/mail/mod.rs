//! Серверная часть виджета почты.
//!
//! Показывает последние письма из ящиков по IMAPS: отправитель, тема, дата,
//! число непрочитанных и короткое превью тела письма.
//!
//! Поведение повторяет виджет погоды (`weather`):
//! - опрос по «правилу звука»: ходим в сеть, только пока окно виджета видно;
//! - ручное обновление по кнопке через `mail_refresh`;
//! - ящики хранятся в отдельном файле `mail.json`, пароли зашифрованы через DPAPI.
//!
//! Ящиков бывает несколько (этап 53, слово пользователя): у каждого своё
//! состояние, свой шаг опроса и свой счёт отказов, а наружу они едут одним
//! снимком. Письмо опознаётся парой «ящик + UID»: UID уникален только внутри
//! своего ящика.

pub mod imap;
pub mod store;
pub mod web;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Notify;

use crate::dpapi;

use self::store::{Account, MailAccount, Store, Stored, MAX_ACCOUNTS};

/// Метка окна виджета в реестре: по ней опрос узнаёт, смотрит ли кто-нибудь на виджет.
const WIDGET: &str = "mail";

/// Имя события для рассылки состояния в окна.
const MAIL_EVENT: &str = "mail";

/// Шаг опроса почты при открытом окне (две минуты).
const STEP: Duration = Duration::from_secs(2 * 60);

/// Пауза после сбоя сети (30 секунд).
const RETRY: Duration = Duration::from_secs(30);

/// Частота пробуждения фонового цикла (5 секунд).
const TICK: Duration = Duration::from_secs(5);

/// Сколько сетевых отказов подряд терпим, прежде чем объявить на карточке ошибку.
const FAILURES_BEFORE_ALARM: u32 = 2;

/// Состояние одного ящика для фронтенда.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum View {
    /// Ящик заведён не до конца — нет сервера, логина или пароля. В сеть не ходим.
    NoAccount,
    /// Запись есть, письма ещё загружаются.
    Loading,
    /// Письма получены; stale — долго нет ответа от сервера, на экране прошлый снимок.
    Ready { inbox: Inbox, stale: bool },
    /// Ошибка при обращении к серверу.
    Failed { reason: String, kind: Trouble },
}

/// Чем кончился поход в ящик, если он не удался.
///
/// Карточка по-английски скупа, и различать ей надо ровно одно: пройдёт ли это
/// само. Не пустил сервер — менять пароль, а ждать бесполезно; не достучались —
/// наоборот, ждать и есть всё лечение. Русская строка `reason` рядом остаётся
/// для лога, в карточку она не попадает никогда.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Trouble {
    /// Сервер не пустил: логин или пароль не те.
    Auth,
    /// Не достучались: сеть, DNS, истёкший срок ожидания.
    Network,
}

/// Ящик в снимке: ключ, подпись и состояние.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoxView {
    pub id: String,
    /// Логин ящика: подсказка у точки вкладки и строка в дереве настроек.
    pub address: String,
    pub view: View,
}

/// Всё, что знают окна о почте, одним снимком.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MailView {
    /// Ящики в порядке заведения.
    pub accounts: Vec<BoxView>,
    /// Строка содержимого под темой письма — вид карточки, одна на все ящики.
    pub preview: bool,
}

/// Содержимое папки «Входящие».
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Inbox {
    /// Общее число непрочитанных писем в папке.
    pub unseen: u32,
    /// Последние полученные письма (до 10 штук).
    pub letters: Vec<Letter>,
    /// Адрес веб-ящика.
    pub mailbox: Option<String>,
}

/// Информация об отдельном письме.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Letter {
    /// Ключ ящика, из которого письмо. На вкладке «Все» письма разных ящиков
    /// стоят одним списком, и действие над письмом обязано уйти в его ящик.
    #[serde(default)]
    pub account: String,
    /// Идентификатор письма (UID) — уникален только внутри своего ящика.
    pub uid: u32,
    /// Отправитель: имя из конверта, а если имени нет — адрес.
    pub from: String,
    /// Адрес отправителя: ivan@example.org.
    pub from_address: Option<String>,
    /// Адреса получателей, не больше пяти.
    pub to: Vec<String>,
    /// Тема письма как есть.
    pub subject: String,
    /// Время письма в секундах эпохи Unix, UTC.
    pub at: i64,
    /// Прочитано ли письмо (флаг \Seen).
    pub seen: bool,
    /// Превью первой текстовой части тела (до 180 знаков).
    ///
    /// Тела письма тут нет намеренно: со списком оно приезжало обрубленным и
    /// пустым — см. `imap::fetch_body`. Всплывающее окно просит его отдельно.
    pub preview: Option<String>,
    /// Ссылка на письмо в веб-интерфейсе.
    pub link: Option<String>,
}

/// Тело письма и вложения для всплывающего окна.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LetterBody {
    /// Очищенный текст письма.
    pub text: Option<String>,
    /// Вложения письма.
    pub attachments: Vec<Attachment>,
}

/// Информация о вложении письма.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    /// Имя файла (при отсутствии — «attachment»).
    pub name: String,
    /// Размер в байтах, как лежит в письме.
    pub size: u64,
    /// Тип: «pdf», «png», «zip», «file».
    pub kind: String,
}

/// Помечает письма ключом их ящика.
fn owned(mut inbox: Inbox, account: &str) -> Inbox {
    for letter in &mut inbox.letters {
        letter.account = account.to_owned();
    }
    inbox
}

/// С чего начинается ящик, которого опрос ещё не видел.
fn initial(account: &Account) -> View {
    if account.is_configured() {
        View::Loading
    } else {
        View::NoAccount
    }
}

/// Снимок по ящикам из файла: прежние состояния сохраняются, новым ящикам —
/// начальные, удалённые уходят.
fn rebuild(stored: &Stored, previous: &[BoxView]) -> Vec<BoxView> {
    stored
        .accounts
        .iter()
        .map(|account| {
            let view = previous
                .iter()
                .find(|kept| kept.id == account.id)
                .map(|kept| kept.view.clone())
                .unwrap_or_else(|| initial(account));
            BoxView {
                id: account.id.clone(),
                address: account.address(),
                view,
            }
        })
        .collect()
}

/// Управление состоянием виджета почты и фоновым опросом.
pub struct Mail {
    boxes: Mutex<Vec<BoxView>>,
    pub store: Arc<Store>,
    nudge: Arc<Notify>,
    forced: AtomicBool,
}

impl Default for Mail {
    fn default() -> Self {
        Self::new(Stored::default())
    }
}

impl Mail {
    pub fn new(stored: Stored) -> Self {
        Self {
            boxes: Mutex::new(rebuild(&stored, &[])),
            store: Arc::new(Store::new(stored)),
            nudge: Arc::new(Notify::new()),
            forced: AtomicBool::new(false),
        }
    }

    pub fn trigger_refresh(&self) {
        self.forced.store(true, Ordering::Relaxed);
        self.nudge.notify_one();
    }

    fn locked(&self) -> MutexGuard<'_, Vec<BoxView>> {
        self.boxes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn snapshot(&self) -> MailView {
        MailView {
            accounts: self.locked().clone(),
            preview: self.store.get().preview,
        }
    }
}

/// Команда: возвращает текущее состояние виджета.
#[tauri::command]
pub fn mail_state(state: State<'_, Mail>) -> MailView {
    state.snapshot()
}

/// Команда: ручное обновление писем (кнопка в карточке) — всех ящиков разом.
#[tauri::command]
pub fn mail_refresh(state: State<'_, Mail>) {
    state.trigger_refresh();
}

/// Учётные данные для одного похода в ящик.
///
/// Пароль в них живёт ровно столько, сколько идёт поход: в состоянии
/// приложения он не оседает никогда.
struct Credentials {
    host: String,
    port: u16,
    login: String,
    password: String,
    mailbox: Option<String>,
    preview: bool,
}

/// Готовит учётные данные ящика или отказывается, если его нет.
///
/// Один на все действия над письмами. Копии этой дюжины строк начали бы
/// расходиться при первой же правке, а расходиться им нельзя: они про пароль.
fn credentials(store: &Store, id: &str) -> Result<Credentials, String> {
    let stored = store.get();
    let account = stored
        .account(id)
        .ok_or_else(|| "ящика с таким ключом нет".to_owned())?;

    let host = account
        .host()
        .ok_or_else(|| "учётная запись не задана".to_owned())?
        .to_owned();
    let login = account
        .login()
        .ok_or_else(|| "учётная запись не задана".to_owned())?
        .to_owned();
    let secret = account
        .secret()
        .ok_or_else(|| "учётная запись не задана".to_owned())?;

    let password =
        dpapi::unprotect(secret).map_err(|e| format!("ошибка расшифровки пароля: {e}"))?;

    Ok(Credentials {
        host,
        port: account.port().unwrap_or(993),
        login,
        password,
        mailbox: account.mailbox().map(str::to_owned),
        preview: stored.preview,
    })
}

/// Команда: тело одного письма для всплывающего окна.
///
/// Отдельным походом и по нажатию: со списком оно не едет вовсе.
#[tauri::command]
pub async fn mail_body(state: State<'_, Mail>, account: String, uid: u32) -> Result<LetterBody, String> {
    let on = credentials(&state.store, &account)?;

    imap::fetch_body(&on.host, on.port, &on.login, &on.password, uid)
        .await
        .map_err(|e| e.to_string())
}

/// Команда: поиск писем в одном ящике.
///
/// Открывает отдельную короткую сессию в режиме только для чтения (EXAMINE INBOX),
/// экранирует запрос и выполняет поиск по TEXT. Возвращает до 30 найденных писем.
/// Состояние виджета не затрагивается, фоновый опрос продолжается. На вкладке
/// «Все» карточка спрашивает каждый ящик и сливает ответы сама.
#[tauri::command]
pub async fn mail_search(
    state: State<'_, Mail>,
    account: String,
    query: String,
    unseen: bool,
) -> Result<Inbox, String> {
    // Ни запроса, ни фильтра — искать нечего, и в сеть не идём вовсе
    if query.trim().is_empty() && !unseen {
        let stored = state.store.get();
        let mailbox = stored.account(&account).and_then(|found| {
            found
                .mailbox()
                .map(str::to_string)
                .or_else(|| found.host().and_then(web::mailbox_url))
        });
        return Ok(Inbox {
            unseen: 0,
            letters: Vec::new(),
            mailbox,
        });
    }

    let on = credentials(&state.store, &account)?;

    let found = imap::search_inbox(
        &on.host,
        on.port,
        &on.login,
        &on.password,
        imap::SearchCriteria {
            query: &query,
            unseen_only: unseen,
        },
        on.mailbox.as_deref(),
        on.preview,
    )
    .await
    .map_err(|e| e.to_string())?;

    Ok(owned(found, &account))
}

/// Команда: помечает письма ящика прочитанными по UID.
#[tauri::command]
pub async fn mail_mark_read(state: State<'_, Mail>, account: String, uids: Vec<u32>) -> Result<(), String> {
    let on = credentials(&state.store, &account)?;

    imap::mark_read(&on.host, on.port, &on.login, &on.password, &uids)
        .await
        .map_err(|e| e.to_string())?;

    state.trigger_refresh();
    Ok(())
}

/// Команда: помечает все письма ящика прочитанными.
#[tauri::command]
pub async fn mail_mark_all_read(state: State<'_, Mail>, account: String) -> Result<(), String> {
    let on = credentials(&state.store, &account)?;

    imap::mark_all_read(&on.host, on.port, &on.login, &on.password)
        .await
        .map_err(|e| e.to_string())?;

    state.trigger_refresh();
    Ok(())
}

/// Команда: удаляет письма ящика (переносит в корзину).
#[tauri::command]
pub async fn mail_delete(state: State<'_, Mail>, account: String, uids: Vec<u32>) -> Result<(), String> {
    let on = credentials(&state.store, &account)?;

    imap::delete_letter(&on.host, on.port, &on.login, &on.password, &uids)
        .await
        .map_err(|e| e.to_string())?;

    state.trigger_refresh();
    Ok(())
}

/// Команда: сохранённые реквизиты ящика (без пароля).
///
/// Незнакомый ключ — пустая форма: так открывается страница нового ящика.
#[tauri::command]
pub fn mail_account(state: State<'_, Mail>, id: String) -> MailAccount {
    state
        .store
        .get()
        .account(&id)
        .map(MailAccount::from)
        .unwrap_or_default()
}

/// Своя ссылка на ящик: пусто — значит «как выведется само».
///
/// Схему дописываем сами: человек пишет «gmail.com/login», а не
/// «https://gmail.com/login», и заставлять его — значит спорить с ним о форме
/// там, где смысл очевиден. А вот проверку схемы обойти нельзя: по этой строке
/// потом запускают браузер.
fn own_mailbox(typed: &str) -> Result<Option<String>, String> {
    match typed.trim() {
        "" => Ok(None),
        typed => {
            let full = if typed.contains("://") {
                typed.to_owned()
            } else {
                format!("https://{typed}")
            };
            if !crate::links::is_safe_link(&full) {
                return Err(tr!("mail.error.badMailbox"));
            }
            Ok(Some(full))
        }
    }
}

/// Команда: сохраняет ящик и отдаёт его ключ.
///
/// `id: None` — новый ящик, ключ ему выдаётся здесь. `password: None` или
/// `Some("")` означает «оставить прежний пароль»; непустая строка шифруется
/// через DPAPI.
#[tauri::command]
pub fn set_mail_account(
    app: AppHandle,
    state: State<'_, Mail>,
    id: Option<String>,
    account: MailAccount,
    password: Option<String>,
) -> Result<String, String> {
    let secret = match password {
        Some(pwd) if !pwd.is_empty() => {
            Some(dpapi::protect(&pwd).map_err(|e| tr!("mail.error.noEncrypt", error = e))?)
        }
        _ => None,
    };
    let mailbox = own_mailbox(&account.mailbox)?;

    let (next, saved) = state.store.update(|stored| {
        let at = match id.as_deref() {
            Some(id) => match stored.accounts.iter().position(|kept| kept.id == id) {
                Some(at) => at,
                // Ящик удалили, пока его страница была открыта: заводить на его
                // месте новый молча было бы подменой, а не сохранением
                None => return Err(tr!("mail.error.gone")),
            },
            None if stored.accounts.len() >= MAX_ACCOUNTS => {
                return Err(tr!("mail.error.tooMany", limit = MAX_ACCOUNTS));
            }
            None => {
                stored.accounts.push(Account {
                    id: crate::totp::store::new_id(),
                    ..Account::default()
                });
                stored.accounts.len() - 1
            }
        };

        let kept = &mut stored.accounts[at];
        kept.host = Some(account.host).filter(|s| !s.trim().is_empty());
        kept.port = Some(account.port).filter(|&p| p > 0);
        kept.login = Some(account.login).filter(|s| !s.trim().is_empty());
        kept.mailbox = mailbox;
        // Пустой пароль — прежний: иначе правка адреса сервера стирала бы его
        if secret.is_some() {
            kept.secret = secret;
        }
        Ok(kept.id.clone())
    });
    let saved = saved?;

    store::save(&app, &next)?;

    // Правленный ящик начинает сначала: прежний снимок был о прежнем сервере.
    // Выброшенный из снимка, он получит начальное состояние при пересборке
    state.locked().retain(|kept| kept.id != saved);
    publish_rebuilt(&app, &next);

    state.trigger_refresh();
    Ok(saved)
}

/// Команда: проверяет подключение ящика по шагам — из окна настроек.
///
/// Решение пользователя (этап 54): «не ясно, почему не подключило почту».
/// Опрос в фоне отвечает карточке двумя словами, а тут нужно знать, на каком
/// шаге встало и что сказал сервер. Проверяется то, что стоит в форме, а не
/// сохранённое: пустой пароль — значит сохранённый у ящика `id`. Введённый
/// пароль живёт ровно этот поход и никуда не записывается.
///
/// Асинхронная: ждёт сеть до пятнадцати секунд, и синхронной командой встало
/// бы всё приложение.
#[tauri::command]
pub async fn mail_check(
    state: State<'_, Mail>,
    id: Option<String>,
    account: MailAccount,
    password: Option<String>,
) -> Result<imap::Check, String> {
    let host = account.host.trim().to_owned();
    let login = account.login.trim().to_owned();
    if host.is_empty() || login.is_empty() {
        return Err(tr!("mail.check.incomplete"));
    }

    let password = match password.filter(|typed| !typed.is_empty()) {
        Some(typed) => typed,
        None => {
            let stored = state.store.get();
            let secret = id
                .as_deref()
                .and_then(|id| stored.account(id))
                .and_then(Account::secret)
                .map(str::to_owned)
                .ok_or_else(|| tr!("mail.check.noPassword"))?;
            dpapi::unprotect(&secret).map_err(|e| tr!("mail.error.noDecrypt", error = e))?
        }
    };

    let port = Some(account.port).filter(|&p| p > 0).unwrap_or(993);
    Ok(imap::check_account(&host, port, &login, &password).await)
}

/// Команда: включает или выключает строку содержимого под темой письма.
///
/// Отдельно от ящиков: это вид карточки, одна на все ящики, и правят её на
/// странице виджета, а ящики — на их собственных страницах.
#[tauri::command]
pub fn set_mail_preview(state: State<'_, Mail>, app: AppHandle, on: bool) -> Result<(), String> {
    let (next, ()) = state.store.update(|stored| stored.preview = on);
    store::save(&app, &next)?;
    publish_rebuilt(&app, &next);

    // Строка содержимого — это выборка тел, и приходит она с опросом: без
    // внеочередного захода включённая галочка ждала бы до двух минут
    if next.accounts.iter().any(Account::is_configured) {
        state.trigger_refresh();
    }
    Ok(())
}

/// Команда: удаляет ящик вместе с паролем.
#[tauri::command]
pub fn forget_mail_account(app: AppHandle, state: State<'_, Mail>, id: String) -> Result<(), String> {
    let (next, ()) = state.store.update(|stored| stored.accounts.retain(|kept| kept.id != id));
    store::save(&app, &next)?;

    publish_rebuilt(&app, &next);
    state.nudge.notify_one();
    Ok(())
}

/// Проверяет, видно ли окно виджета (правило звука).
fn watched(app: &AppHandle) -> bool {
    app.get_webview_window(WIDGET)
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false)
}

#[derive(Debug, Clone, Copy)]
struct Moment {
    has_account: bool,
    watched: bool,
    since: Option<Duration>,
    failing: bool,
    auth_failed: bool,
    forced: bool,
}

/// Определяет, пора ли идти в почтовый ящик.
fn due(moment: &Moment) -> bool {
    if !moment.has_account {
        return false;
    }

    // Ручное обновление обходит видимость, ошибку входа и интервалы
    if moment.forced {
        return true;
    }

    if !moment.watched {
        return false;
    }

    // При ошибке входа не долбим сервер автоматически каждые 2 минуты
    if moment.auth_failed {
        return false;
    }

    let Some(since) = moment.since else {
        // Окно только что открыли — опрашиваем сразу
        return true;
    };

    since >= if moment.failing { RETRY } else { STEP }
}

/// Вычисляет следующее состояние после ошибки запроса.
fn after_failure(previous: &View, reason: String, failures: u32, kind: Trouble) -> View {
    // Ошибка входа показывается сразу: сама она не пройдёт, и ждать нечего
    if kind == Trouble::Auth {
        return View::Failed { reason, kind };
    }

    let alarmed = failures >= FAILURES_BEFORE_ALARM;

    // Прежний снимок оставляем на экране, пометив его как stale при затянувшемся молчании
    if let View::Ready { inbox, .. } = previous {
        return View::Ready {
            inbox: inbox.clone(),
            stale: alarmed,
        };
    }

    if alarmed {
        View::Failed { reason, kind }
    } else {
        View::Loading
    }
}

/// Ставит состояние одного ящика и рассылает снимок.
///
/// `always` — рассылать и неизменившееся: на нажатие кнопки обновления ответ
/// обязан прийти, иначе значок в карточке крутился бы вечно.
fn set_box(app: &AppHandle, id: &str, next: View, always: bool) {
    let state = app.state::<Mail>();
    {
        let mut boxes = state.locked();
        let Some(kept) = boxes.iter_mut().find(|kept| kept.id == id) else {
            // Ящик удалили, пока шёл опрос: его ответ никому не нужен
            return;
        };
        if kept.view == next && !always {
            return;
        }
        kept.view = next;
    }

    let _ = app.emit(MAIL_EVENT, state.snapshot());
}

/// Пересобирает снимок по файлу — после правки ящиков — и рассылает его.
fn publish_rebuilt(app: &AppHandle, stored: &Stored) {
    let state = app.state::<Mail>();
    {
        let mut boxes = state.locked();
        *boxes = rebuild(stored, &boxes);
    }

    let _ = app.emit(MAIL_EVENT, state.snapshot());
}

/// Как идёт опрос одного ящика.
#[derive(Debug, Default)]
struct Track {
    last: Option<Instant>,
    failures: u32,
    auth_failed: bool,
    configured: bool,
}

/// Запускает фоновый опрос почты.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move { work(app).await });
}

async fn rest(nudge: &Notify) {
    let _ = tokio::time::timeout(TICK, nudge.notified()).await;
}

async fn work(app: AppHandle) {
    let nudge = app.state::<Mail>().nudge.clone();
    let mut tracks: HashMap<String, Track> = HashMap::new();

    loop {
        let stored = app.state::<Mail>().store.get();

        // Удалённые ящики забываются вместе со своим ходом опроса
        tracks.retain(|id, _| stored.account(id).is_some());

        // Кнопка — одна на все ящики: снимается один раз на круг
        let forced = app
            .state::<Mail>()
            .forced
            .swap(false, Ordering::Relaxed);
        let watched = watched(&app);

        for account in &stored.accounts {
            poll(&app, account, stored.preview, tracks.entry(account.id.clone()).or_default(), watched, forced).await;
        }

        rest(&nudge).await;
    }
}

/// Один ящик за один круг опроса: сходить, если пора, и разослать ответ.
async fn poll(app: &AppHandle, account: &Account, preview: bool, track: &mut Track, watched: bool, forced: bool) {
    let configured = account.is_configured();

    // Ящик дозаполнили или стёрли пароль — ход опроса начинается сначала
    if configured != track.configured {
        *track = Track {
            configured,
            ..Track::default()
        };
        set_box(app, &account.id, initial(account), false);
    }

    if forced {
        // При нажатии кнопки обновления сбрасываем признак ошибки входа
        track.auth_failed = false;
    }

    let moment = Moment {
        has_account: configured,
        watched,
        since: track.last.map(|at| at.elapsed()),
        failing: track.failures > 0,
        auth_failed: track.auth_failed,
        forced,
    };

    if !due(&moment) {
        return;
    }

    let (Some(host), Some(login), Some(secret)) = (account.host(), account.login(), account.secret()) else {
        return;
    };
    let port = account.port().unwrap_or(993);

    // Пароль расшифровывается только на время похода в сеть и никуда наружу не попадает
    let password = match dpapi::unprotect(secret) {
        Ok(pwd) => pwd,
        Err(err) => {
            eprintln!("ошибка расшифровки пароля почты: {err}");
            track.failures = track.failures.saturating_add(1);
            track.auth_failed = true;
            let next = View::Failed {
                reason: "не удалось расшифровать пароль".to_owned(),
                kind: Trouble::Auth,
            };
            set_box(app, &account.id, next, forced);
            return;
        }
    };

    track.last = Some(Instant::now());

    let answer = imap::fetch_inbox(host, port, login, &password, account.mailbox(), preview).await;

    // Пока шёл поход, ящик могли переправить или удалить: ответ прежнего
    // сервера лёг бы поверх нового ящика — чужими письмами или чужим отказом.
    // Такой ответ выбрасывается, новый ящик опросится своим чередом
    let unchanged = app.state::<Mail>().store.get().account(&account.id) == Some(account);
    if !unchanged {
        return;
    }

    match answer {
        Ok(inbox) => {
            track.failures = 0;
            track.auth_failed = false;
            let next = View::Ready {
                inbox: owned(inbox, &account.id),
                stale: false,
            };
            set_box(app, &account.id, next, forced);
        }
        Err(err) => {
            track.failures = track.failures.saturating_add(1);
            let is_auth = matches!(err, imap::MailError::Auth(_));
            if is_auth {
                track.auth_failed = true;
            }
            let previous = app
                .state::<Mail>()
                .locked()
                .iter()
                .find(|kept| kept.id == account.id)
                .map(|kept| kept.view.clone())
                .unwrap_or(View::Loading);
            let kind = if is_auth { Trouble::Auth } else { Trouble::Network };
            let next = after_failure(&previous, err.to_string(), track.failures, kind);
            set_box(app, &account.id, next, forced);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moment(watched: bool, since: Option<Duration>, failing: bool) -> Moment {
        Moment {
            has_account: true,
            watched,
            since,
            failing,
            auth_failed: false,
            forced: false,
        }
    }

    fn sample_inbox() -> Inbox {
        Inbox {
            unseen: 3,
            letters: vec![Letter {
                account: String::new(),
                uid: 101,
                from: "Иван".to_owned(),
                from_address: Some("ivan@example.org".to_owned()),
                to: vec!["me@example.org".to_owned()],
                subject: "Тема".to_owned(),
                at: 1700000000,
                seen: false,
                preview: Some("Превью письма".to_owned()),
                link: Some("https://example.com".to_owned()),
            }],
            mailbox: Some("https://example.com".to_owned()),
        }
    }

    fn account(id: &str, configured: bool) -> Account {
        Account {
            id: id.to_owned(),
            host: Some("imap.example.org".to_owned()),
            port: Some(993),
            login: Some(format!("{id}@example.org")),
            secret: configured.then(|| "YmxvYg==".to_owned()),
            mailbox: None,
        }
    }

    #[test]
    fn a_hidden_window_is_never_polled() {
        assert!(!due(&moment(false, None, false)));
        assert!(!due(&moment(false, Some(STEP * 2), false)));

        assert!(!due(&Moment {
            has_account: false,
            watched: true,
            since: None,
            failing: false,
            auth_failed: false,
            forced: false,
        }));
    }

    #[test]
    fn the_button_beats_the_step_and_visibility() {
        let pressed = Moment {
            has_account: true,
            watched: false,
            since: Some(Duration::from_secs(1)),
            failing: false,
            auth_failed: false,
            forced: true,
        };
        assert!(due(&pressed));

        // Но отсутствие аккаунта кнопка не бьёт: без него ходить некуда
        assert!(!due(&Moment {
            has_account: false,
            ..pressed
        }));
    }

    #[test]
    fn auth_failure_stops_automatic_polls_until_forced() {
        let auth_failed_moment = Moment {
            has_account: true,
            watched: true,
            since: Some(STEP * 2),
            failing: true,
            auth_failed: true,
            forced: false,
        };
        assert!(!due(&auth_failed_moment));

        // Нажатие кнопки всё равно заставляет сходить
        assert!(due(&Moment {
            forced: true,
            ..auth_failed_moment
        }));
    }

    #[test]
    fn the_step_is_kept_between_polls() {
        assert!(due(&moment(true, None, false)), "первый заход не ждёт");
        assert!(!due(&moment(true, Some(STEP / 2), false)));
        assert!(due(&moment(true, Some(STEP), false)));
    }

    #[test]
    fn failure_is_retried_sooner_than_the_step() {
        assert!(!due(&moment(true, Some(RETRY / 2), true)));
        assert!(due(&moment(true, Some(RETRY), true)));
        assert!(!due(&moment(true, Some(RETRY), false)));
    }

    #[test]
    fn a_lost_answer_keeps_the_last_snapshot() {
        let shown = View::Ready {
            inbox: sample_inbox(),
            stale: false,
        };

        // Первая осечка сохраняет снимок без отметки stale
        let once = after_failure(&shown, "таймаут".to_owned(), 1, Trouble::Network);
        assert_eq!(once, shown);

        // Вторая осечка помечает снимок как stale
        let twice = after_failure(&shown, "таймаут".to_owned(), 2, Trouble::Network);
        assert_eq!(
            twice,
            View::Ready {
                inbox: sample_inbox(),
                stale: true,
            }
        );
    }

    #[test]
    fn without_a_snapshot_alarm_takes_two_failures() {
        let waiting = View::Loading;

        assert_eq!(
            after_failure(&waiting, "нет связи".to_owned(), 1, Trouble::Network),
            waiting
        );

        assert_eq!(
            after_failure(&waiting, "нет связи".to_owned(), 2, Trouble::Network),
            View::Failed {
                reason: "нет связи".to_owned(),
                kind: Trouble::Network,
            }
        );
    }

    #[test]
    fn auth_failure_fails_immediately() {
        let shown = View::Ready {
            inbox: sample_inbox(),
            stale: false,
        };

        let res = after_failure(&shown, "неверный пароль".to_owned(), 1, Trouble::Auth);
        assert_eq!(
            res,
            View::Failed {
                reason: "неверный пароль".to_owned(),
                kind: Trouble::Auth,
            }
        );
    }

    /// Ответ проверки подключения читает окно настроек: форма поля и имена
    /// шагов обязаны совпадать с `MailCheck` в `lib/mail.ts`.
    #[test]
    fn a_check_keeps_the_shape_the_settings_read() {
        let passed = serde_json::to_string(&imap::Check::Passed {
            messages: 120,
            unseen: 3,
            millis: 840,
        })
        .expect("сериализуется");
        assert_eq!(passed, r#"{"outcome":"passed","messages":120,"unseen":3,"millis":840}"#);

        let failed = serde_json::to_string(&imap::Check::Failed {
            stage: imap::Stage::Login,
            timed_out: false,
            detail: "[AUTHENTICATIONFAILED] Invalid credentials".to_owned(),
            millis: 1200,
        })
        .expect("сериализуется");
        assert_eq!(
            failed,
            r#"{"outcome":"failed","stage":"login","timedOut":false,"detail":"[AUTHENTICATIONFAILED] Invalid credentials","millis":1200}"#
        );
    }

    /// Письма помечаются своим ящиком: на вкладке «Все» по этой метке
    /// действие уходит туда, откуда письмо.
    #[test]
    fn letters_carry_their_account() {
        let inbox = owned(sample_inbox(), "abcd");
        assert!(inbox.letters.iter().all(|letter| letter.account == "abcd"));
    }

    /// Снимок по файлу: прежние состояния живут, новые ящики начинают с
    /// начального, удалённые уходят, порядок — как в файле.
    #[test]
    fn a_rebuild_keeps_what_it_knows() {
        let ready = View::Ready {
            inbox: sample_inbox(),
            stale: false,
        };
        let before = vec![
            BoxView {
                id: "one".into(),
                address: "one@example.org".into(),
                view: ready.clone(),
            },
            BoxView {
                id: "gone".into(),
                address: "gone@example.org".into(),
                view: View::Loading,
            },
        ];
        let stored = Stored {
            accounts: vec![account("new", false), account("one", true)],
            preview: false,
        };

        let after = rebuild(&stored, &before);
        assert_eq!(after.len(), 2);
        assert_eq!(after[0].id, "new");
        assert_eq!(after[0].view, View::NoAccount, "без пароля ящик не опрашивается");
        assert_eq!(after[1].view, ready);
        assert_eq!(after[1].address, "one@example.org");
    }

    #[test]
    fn view_serializes_to_camel_case() {
        let no_account = serde_json::to_string(&View::NoAccount).expect("сериализуется");
        assert_eq!(no_account, r#"{"status":"noAccount"}"#);

        let loading = serde_json::to_string(&View::Loading).expect("сериализуется");
        assert_eq!(loading, r#"{"status":"loading"}"#);

        let ready = serde_json::to_string(&View::Ready {
            inbox: owned(sample_inbox(), "abcd"),
            stale: false,
        })
        .expect("сериализуется");
        assert!(ready.contains(r#""status":"ready""#));
        assert!(ready.contains(r#""stale":false"#));
        assert!(ready.contains(r#""unseen":3"#));
        assert!(ready.contains(r#""account":"abcd""#));
        assert!(ready.contains(r#""mailbox":"https://example.com""#));
        assert!(ready.contains(r#""fromAddress":"ivan@example.org""#));
        assert!(ready.contains(r#""to":["me@example.org"]"#));
        assert!(
            !ready.contains(r#""body""#),
            "тело письма со списком не едет: оно приходит обрубленным"
        );
        assert!(ready.contains(r#""link":"https://example.com""#));

        let body = LetterBody {
            text: Some("Текст письма".to_owned()),
            attachments: vec![Attachment {
                name: "документ.pdf".to_owned(),
                size: 1024,
                kind: "pdf".to_owned(),
            }],
        };
        let body_json = serde_json::to_string(&body).expect("сериализуется");
        assert!(body_json.contains(r#""text":"Текст письма""#));
        assert!(body_json.contains(r#""attachments":[{""#));
        assert!(body_json.contains(r#""name":"документ.pdf""#));
        assert!(body_json.contains(r#""size":1024"#));
        assert!(body_json.contains(r#""kind":"pdf""#));

        let failed = serde_json::to_string(&View::Failed {
            reason: "ошибка связи".to_owned(),
            kind: Trouble::Network,
        })
        .expect("сериализуется");
        assert_eq!(
            failed,
            r#"{"status":"failed","reason":"ошибка связи","kind":"network"}"#
        );

        let refused = serde_json::to_string(&View::Failed {
            reason: "сервер не пустил".to_owned(),
            kind: Trouble::Auth,
        })
        .expect("сериализуется");
        assert!(refused.contains(r#""kind":"auth""#));

        // Снимок целиком: ящики с ключом, подписью и состоянием, строка
        // содержимого — рядом, одна на всех
        let view = MailView {
            accounts: vec![BoxView {
                id: "abcd".into(),
                address: "me@example.org".into(),
                view: View::Loading,
            }],
            preview: true,
        };
        assert_eq!(
            serde_json::to_string(&view).expect("сериализуется"),
            r#"{"accounts":[{"id":"abcd","address":"me@example.org","view":{"status":"loading"}}],"preview":true}"#
        );
    }
}
