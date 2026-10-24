//! Резервная копия настроек: выгрузка в один файл и восстановление из него.
//!
//! Открытые секреты живут только в выбранной копии и разобранном снимке.
//! Восстановление готовится отдельно и применяется до запуска хранилищ.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tauri::{AppHandle, Manager, State, WebviewWindow};

const FILES: [&str; 10] = [
    "settings.json", ".window-state.json", "sound-window-anchor.json",
    "notes.json", "todo.json", "mail.json", "totp.json", "spotify.json",
    "media.json", "speedtest.json",
];
// Производные места окон не выгружаются, но не должны пережить замену раскладки.
const PLACE_COMPANIONS: [&str; 3] = [
    ".window-state.good.json", ".window-state.monitors", ".window-state.unread.json",
];
const FORMAT: &str = "windgets-backup";
const WARNING: &str = "Contains mail passwords, 2FA keys and the Spotify token in plain text.";

/// Что лежит в копии — для подтверждения перед восстановлением.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    /// Имя выбранного файла, без папки.
    pub file: String,
    /// Когда копию сняли, ISO 8601 в UTC.
    pub created: String,
    /// Версия приложения, снявшего копию.
    pub app: String,
    pub mail: usize,
    pub totp: usize,
    pub notes: usize,
    pub todo: usize,
    pub spotify: bool,
    /// Сколько виджетов включено в копии.
    pub widgets: usize,
}

/// Короткий код отказа; текст берёт фронт из каталога (backup.error.*).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum BackupError {
    /// Уже идёт выгрузка, восстановление или открыт диалог.
    Busy,
    /// Файл данных не прочитался — выгрузка не пропускает его молча.
    Unreadable { file: String },
    /// Секрет не расшифровался (выгрузка) или не зашифровался (восстановление).
    Secret { file: String },
    /// Файл копии не записался.
    Write,
    /// Выбранный файл — не копия Windgets.
    Format,
    /// Копия от более новой версии формата.
    Version,
    /// Файл внутри копии не разбирается типами своего хранилища.
    Invalid { file: String },
    /// Не удалось поставить файлы на восстановление.
    Stage,
    /// Восстанавливать нечего: копию не выбирали.
    Nothing,
}

/// Разобранная копия между backup_inspect и backup_restore.
#[derive(Default)]
pub struct Backup {
    inspected: Mutex<Option<Inspected>>,
    busy: Arc<AtomicBool>,
}

// Без Debug и Clone: секреты не печатаются и не размножаются вместе со снимком.
struct Inspected {
    files: Map<String, Value>,
    created: String,
    app: String,
}

/// Замок принадлежит работе в пуле: отмена IPC не отпускает его раньше диска.
struct Operation(Arc<AtomicBool>);

impl Operation {
    fn begin(busy: &Arc<AtomicBool>) -> Result<Self, BackupError> {
        busy.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| BackupError::Busy)?;
        Ok(Self(Arc::clone(busy)))
    }
}

impl Drop for Operation {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Выгрузка: окно сохранения, затем файл. Ok(None) — человек передумал.
#[tauri::command]
pub async fn backup_export(app: AppHandle, window: WebviewWindow) -> Result<Option<String>, BackupError> {
    let operation = Operation::begin(&app.state::<Backup>().busy)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = operation;
        let owner = window.hwnd().map(|hwnd| hwnd.0 as isize).unwrap_or(0);
        let what = crate::picker::Pick::Save { name: export_name(), filter: crate::picker::Filter::JSON };
        let Some(path) = choose(owner, what)? else { return Ok(None) };
        let dir = app.path().app_config_dir().map_err(|_| BackupError::Write)?;
        let backup = collect_backup(&dir, &app.package_info().version.to_string(), now(), crate::dpapi::unprotect)?;
        let text = serde_json::to_string_pretty(&backup).map_err(|_| BackupError::Write)?;
        drop(backup);
        crate::disk::write_atomically(&path, text).map_err(|error| {
            eprintln!("[копия] не записать выбранный файл: {error}");
            BackupError::Write
        })?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
    .map_err(|_| BackupError::Write)?
}

/// Открыть копию и показать, что в ней. Ok(None) — человек передумал.
#[tauri::command]
pub async fn backup_inspect(app: AppHandle, window: WebviewWindow, state: State<'_, Backup>) -> Result<Option<Summary>, BackupError> {
    let operation = Operation::begin(&state.busy)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = operation;
        let state = app.state::<Backup>();
        // После отмены или отказа нового выбора прежний снимок не восстанавливаем.
        let _ = state.inspected.lock().unwrap_or_else(|p| p.into_inner()).take();
        let owner = window.hwnd().map(|hwnd| hwnd.0 as isize).unwrap_or(0);
        let Some(path) = choose(owner, crate::picker::Pick::File { filter: crate::picker::Filter::JSON })? else {
            return Ok(None);
        };
        let file = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let text = crate::disk::read_existing(&path)
            .map_err(|_| BackupError::Unreadable { file: file.clone() })?
            .ok_or_else(|| BackupError::Unreadable { file: file.clone() })?;
        let inspected = parse_backup(&text)?;
        drop(text);
        let summary = summarize(file, &inspected);
        *state.inspected.lock().unwrap_or_else(|p| p.into_inner()) = Some(inspected);
        Ok(Some(summary))
    })
    .await
    .map_err(|_| BackupError::Format)?
}

/// Поставить разобранную копию на восстановление и перезапуститься.
#[tauri::command]
pub async fn backup_restore(app: AppHandle, state: State<'_, Backup>) -> Result<(), BackupError> {
    let operation = Operation::begin(&state.busy)?;
    let handle = app.clone();
    // После постановки замок остаётся у команды до запроса перезапуска.
    let _operation = tauri::async_runtime::spawn_blocking(move || {
        let inspected = handle.state::<Backup>().inspected.lock()
            .unwrap_or_else(|p| p.into_inner()).take().ok_or(BackupError::Nothing)?;
        let dir = handle.path().app_config_dir().map_err(|_| BackupError::Stage)?;
        stage_backup(&dir, inspected, crate::dpapi::protect)?;
        Ok::<_, BackupError>(operation)
    })
    .await
    .map_err(|_| BackupError::Stage)??;
    // Запрос идёт через очередь главного потока, команда завершения не ждёт.
    crate::windows::on_main(&app, |app| {
        app.state::<crate::windows::Shutdown>().begin();
        app.request_restart();
    });
    Ok(())
}

/// Вызывается только в пуле: человек может выбирать файл сколько угодно.
fn choose(owner: isize, what: crate::picker::Pick) -> Result<Option<PathBuf>, BackupError> {
    if crate::picker::busy() {
        return Err(BackupError::Busy);
    }
    let (send, receive) = std::sync::mpsc::channel();
    crate::picker::pick(owner, what, None, move |chosen| { let _ = send.send(chosen); });
    // Если другой диалог успел занять окно после busy(), pick уронит замыкание
    // вместе с отправителем: канал закроется сразу, без ожидания и таймаута.
    let chosen = receive.recv().map_err(|_| BackupError::Busy)?;
    Ok(chosen.into_iter().next())
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// Григорианский календарь: полный цикл в 400 лет содержит 146097 дней.
fn utc_parts(seconds: u64) -> (u64, u64, u64, u64, u64, u64) {
    let mut days = seconds / 86400;
    let mut year = 1970 + days / 146097 * 400;
    days %= 146097;
    let leap = |year: u64| year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    loop {
        let length = if leap(year) { 366 } else { 365 };
        if days < length { break; }
        days -= length;
        year += 1;
    }
    let lengths = [31, if leap(year) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 0;
    while days >= lengths[month] {
        days -= lengths[month];
        month += 1;
    }
    (year, month as u64 + 1, days + 1, seconds / 3600 % 24, seconds / 60 % 60, seconds % 60)
}

fn utc_stamp(seconds: u64) -> String {
    let (y, m, d, h, min, s) = utc_parts(seconds);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{min:02}:{s:02}Z")
}

fn export_name() -> String {
    #[cfg(windows)]
    let (year, month, day) = {
        let time = unsafe { ::windows::Win32::System::SystemInformation::GetLocalTime() };
        (u64::from(time.wYear), u64::from(time.wMonth), u64::from(time.wDay))
    };
    #[cfg(not(windows))]
    let (year, month, day, _, _, _) = utc_parts(now());
    format!("windgets-backup-{year:04}-{month:02}-{day:02}.json")
}

/// Меняет только явно названное поле. Пустой/отсутствующий пароль — не DPAPI.
fn change_secret(
    value: &mut Value, from: &str, to: &str, file: &str, exporting: bool,
    transform: &impl Fn(&str) -> Result<String, String>,
) -> Result<(), BackupError> {
    let Some(object) = value.as_object_mut() else { return Ok(()) };
    // Из копии принимаем только открытое поле: чужой DPAPI-блоб не переносим.
    if !exporting { object.remove(to); }
    let Some(secret) = object.remove(from) else { return Ok(()) };
    let secret = match secret {
        Value::Null => Value::Null,
        Value::String(text) if text.is_empty() => Value::String(text),
        Value::String(text) => Value::String(transform(&text)
            .map_err(|_| BackupError::Secret { file: file.to_owned() })?),
        _ => return Err(BackupError::Secret { file: file.to_owned() }),
    };
    object.insert(to.to_owned(), secret);
    Ok(())
}

fn change_secrets(
    file: &str, value: &mut Value, exporting: bool,
    transform: &impl Fn(&str) -> Result<String, String>,
) -> Result<(), BackupError> {
    let (from, to) = if exporting { ("secret", "secret_plain") } else { ("secret_plain", "secret") };
    match file {
        "mail.json" | "totp.json" => {
            if let Some(accounts) = value.get_mut("accounts").and_then(Value::as_array_mut) {
                for account in accounts {
                    change_secret(account, from, to, file, exporting, transform)?;
                }
            } else if file == "mail.json" {
                // Старый одиночный ящик остаётся одиночным, включая все лишние поля.
                change_secret(value, from, to, file, exporting, transform)?;
            }
        }
        "spotify.json" => {
            let (from, to) = if exporting {
                ("refresh_token_protected", "refresh_token_plain")
            } else {
                ("refresh_token_plain", "refresh_token_protected")
            };
            change_secret(value, from, to, file, exporting, transform)?;
        }
        _ => {}
    }
    Ok(())
}

fn collect_backup(
    dir: &Path, app: &str, seconds: u64,
    unprotect: impl Fn(&str) -> Result<String, String>,
) -> Result<Value, BackupError> {
    let mut files = Map::new();
    for file in FILES {
        let Some(text) = crate::disk::read_existing(&dir.join(file))
            .map_err(|_| BackupError::Unreadable { file: file.to_owned() })? else { continue };
        let mut value: Value = serde_json::from_str(&text)
            .map_err(|_| BackupError::Unreadable { file: file.to_owned() })?;
        drop(text);
        change_secrets(file, &mut value, true, &unprotect)?;
        files.insert(file.to_owned(), value);
    }
    Ok(serde_json::json!({
        "format": FORMAT, "version": 1, "app": app, "created": utc_stamp(seconds),
        "warning": WARNING, "files": files,
    }))
}

fn plain_field(value: &Value, field: &str, nullable: bool) -> bool {
    value.get(field).is_none_or(|secret| secret.is_string() || (nullable && secret.is_null()))
}

/// Ничего не нормализуем: проверка не должна выбросить неизвестные поля снимка.
fn valid_file(file: &str, value: &Value) -> bool {
    if !value.is_object() { return false; }
    match file {
        "settings.json" => crate::settings::Settings::deserialize(value).is_ok(),
        "notes.json" => crate::notes::Book::deserialize(value).is_ok(),
        "todo.json" => crate::todo::Board::deserialize(value).is_ok(),
        "media.json" => crate::media::store::Stored::deserialize(value).is_ok(),
        "totp.json" => value.get("accounts").and_then(Value::as_array).is_some_and(|accounts| {
            accounts.iter().all(|account| account.is_object() && plain_field(account, "secret_plain", false))
                && crate::totp::store::Stored::deserialize(value).is_ok()
        }),
        "mail.json" => {
            let valid_account = |account: &Value| account.is_object()
                && plain_field(account, "secret_plain", true)
                && crate::mail::store::Account::deserialize(account).is_ok();
            value.get("preview").is_none_or(Value::is_boolean)
                && match value.get("accounts") {
                    Some(accounts) => accounts.as_array().is_some_and(|accounts| accounts.iter().all(valid_account)),
                    None => valid_account(value),
                }
        }
        "spotify.json" => plain_field(value, "refresh_token_plain", true),
        // Форматы этих хранилищ закрыты или не имеют публичного типа разбора.
        _ => true,
    }
}

fn parse_backup(text: &str) -> Result<Inspected, BackupError> {
    let mut value: Value = serde_json::from_str(text).map_err(|_| BackupError::Format)?;
    if value.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(BackupError::Format);
    }
    let version = value.get("version").and_then(Value::as_u64).ok_or(BackupError::Format)?;
    if version > 1 { return Err(BackupError::Version); }
    if version != 1 { return Err(BackupError::Format); }
    let created = value.get("created").and_then(Value::as_str).unwrap_or_default().to_owned();
    let app = value.get("app").and_then(Value::as_str).unwrap_or_default().to_owned();
    let source = value.get_mut("files").and_then(Value::as_object_mut).ok_or(BackupError::Format)?;
    let mut files = Map::new();
    // Путь никогда не строится из имени, пришедшего из выбранного файла.
    for file in FILES {
        if let Some(value) = source.remove(file) {
            if !valid_file(file, &value) {
                return Err(BackupError::Invalid { file: file.to_owned() });
            }
            files.insert(file.to_owned(), value);
        }
    }
    Ok(Inspected { files, created, app })
}

fn summarize(file: String, inspected: &Inspected) -> Summary {
    let count = |file: &str, field: &str| inspected.files.get(file)
        .and_then(|value| value.get(field)).and_then(Value::as_array).map_or(0, Vec::len);
    let spotify = inspected.files.get("spotify.json").is_some_and(|value| {
        ["refresh_token_plain", "refresh_token"].iter().any(|field| value.get(*field)
            .and_then(Value::as_str).is_some_and(|token| !token.is_empty()))
    });
    let widgets = inspected.files.get("settings.json").and_then(|value| value.get("widgets"))
        .and_then(Value::as_object).map_or(0, |widgets| widgets.values()
            .filter(|widget| widget.get("enabled").and_then(Value::as_bool) == Some(true)).count());
    Summary {
        file, created: inspected.created.clone(), app: inspected.app.clone(),
        mail: count("mail.json", "accounts"), totp: count("totp.json", "accounts"),
        notes: count("notes.json", "pages"), todo: count("todo.json", "lists"), spotify, widgets,
    }
}

fn remove_restore(restore: &Path) -> io::Result<()> {
    match fs::remove_dir_all(restore) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn stage_backup(
    dir: &Path, mut inspected: Inspected,
    protect: impl Fn(&str) -> Result<String, String>,
) -> Result<(), BackupError> {
    let restore = dir.join("restore");
    let result = (|| {
        remove_restore(&restore).map_err(|_| BackupError::Stage)?;
        fs::create_dir_all(&restore).map_err(|_| BackupError::Stage)?;
        for file in FILES {
            let Some(mut value) = inspected.files.remove(file) else { continue };
            change_secrets(file, &mut value, false, &protect)?;
            let text = serde_json::to_string_pretty(&value).map_err(|_| BackupError::Stage)?;
            drop(value);
            crate::disk::write_atomically(&restore.join(file), text).map_err(|_| BackupError::Stage)?;
        }
        // Только полностью записанный набор может быть применён при запуске.
        crate::disk::write_atomically(&restore.join("ready"), b"").map_err(|_| BackupError::Stage)
    })();
    if result.is_err() {
        if let Err(error) = remove_restore(&restore) {
            eprintln!("[копия] не убрать незавершённую постановку: {error}");
        }
    }
    result
}

/// При запуске, до всех читателей: поставленная копия встаёт на место.
pub fn apply_pending(dir: &Path) {
    if let Err(error) = apply_pending_at(dir, now()) {
        eprintln!("[копия] восстановление не завершено: {error}");
    }
}

fn before_dir(dir: &Path, seconds: u64) -> io::Result<PathBuf> {
    let (y, m, d, h, min, s) = utc_parts(seconds);
    let name = format!("before-restore-{y:04}{m:02}{d:02}-{h:02}{min:02}{s:02}");
    for number in 1..=1000 {
        let path = dir.join(if number == 1 { name.clone() } else { format!("{name}-{number}") });
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "нет свободного имени прежней копии"))
}

/// Не принимаем каталог за файл и не теряем ошибки доступа за exists().
fn has_file(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(true),
        Ok(_) => Err(io::Error::new(io::ErrorKind::InvalidData, "вместо файла найден каталог или ссылка")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Имя фиксируется до первого переноса: после сбоя страховочная папка остаётся общей.
fn resolve_before_dir(dir: &Path, restore: &Path, seconds: u64) -> io::Result<PathBuf> {
    match fs::read_to_string(restore.join("before")) {
        Ok(name) => {
            // В журнале допустимо только наше имя, без пути за пределы папки данных.
            if !name.starts_with("before-restore-")
                || !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "неверное имя прежней копии"));
            }
            let before = dir.join(name);
            if !fs::symlink_metadata(&before)?.file_type().is_dir() {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "прежняя копия не является каталогом"));
            }
            Ok(before)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if has_file(&restore.join("installing"))? || has_file(&restore.join("rolling-back"))? {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "утрачено имя прежней копии"));
            }
            let before = before_dir(dir, seconds)?;
            let name = before.file_name().and_then(|name| name.to_str()).unwrap();
            crate::disk::write_atomically(&restore.join("before"), name)?;
            Ok(before)
        }
        Err(error) => Err(error),
    }
}

fn apply_files(dir: &Path, restore: &Path, before: &Path) -> io::Result<()> {
    if !has_file(&restore.join("installing"))? {
        for file in FILES.into_iter().chain(PLACE_COMPANIONS) {
            if has_file(&dir.join(file))? {
                if has_file(&before.join(file))? {
                    return Err(io::Error::new(io::ErrorKind::AlreadyExists, "прежний файл уже сохранён"));
                }
                fs::rename(dir.join(file), before.join(file))?;
            }
        }
        // До этой метки в dir нет новых файлов; после неё первый цикл не повторяем.
        crate::disk::write_atomically(&restore.join("installing"), b"")?;
    }
    for file in FILES {
        if has_file(&restore.join(file))? {
            fs::rename(restore.join(file), dir.join(file))?;
        }
    }
    Ok(())
}

/// Откат тоже возобновляется: сначала убираем новые, затем возвращаем прежние.
fn rollback_files(dir: &Path, restore: &Path, before: &Path) -> io::Result<()> {
    if has_file(&restore.join("installing"))? {
        for file in FILES.into_iter().rev() {
            // Оставшийся в restore файл ещё не ставился. Каталог здесь мог как
            // раз вызвать отказ применения; он не мешает вернуть остальные файлы.
            match fs::symlink_metadata(restore.join(file)) {
                Ok(_) => continue,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            if has_file(&dir.join(file))? {
                fs::rename(dir.join(file), restore.join(file))?;
            }
        }
        // rolling-back остаётся: при сбое во время возврата старых файлов
        // следующий запуск уже не примет их за новые и не начнёт установку.
        fs::remove_file(restore.join("installing"))?;
    }
    for file in FILES.into_iter().chain(PLACE_COMPANIONS).rev() {
        if has_file(&before.join(file))? {
            fs::rename(before.join(file), dir.join(file))?;
        }
    }
    Ok(())
}

fn apply_pending_at(dir: &Path, seconds: u64) -> io::Result<()> {
    let restore = dir.join("restore");
    if !has_file(&restore.join("ready"))? { return remove_restore(&restore) }
    let before = resolve_before_dir(dir, &restore, seconds)?;
    let result = if has_file(&restore.join("rolling-back"))? {
        rollback_files(dir, &restore, &before)?;
        Err(io::Error::other("завершён отложенный откат восстановления"))
    } else {
        let result = apply_files(dir, &restore, &before);
        if result.is_err() {
            // Пишем намерение до первого обратного переноса, иначе сбой посреди
            // отката заставил бы следующий запуск продолжить прямую установку.
            crate::disk::write_atomically(&restore.join("rolling-back"), b"")?;
            rollback_files(dir, &restore, &before)?;
            eprintln!("[копия] применение прервано; прежние файлы возвращены");
        }
        result
    };
    // Граница снятия ready — полная установка либо полный откат, включая
    // работу прежнего запуска. До неё любой отказ сохраняет журнал для
    // продолжения после сбоя питания; после неё неудачу больше не повторяем.
    let marker = match fs::remove_file(restore.join("ready")) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    };
    if let Err(error) = &marker {
        eprintln!("[копия] не снять метку восстановления: {error}");
        return result.and(marker);
    }
    let cleanup = remove_restore(&restore);
    if let Err(error) = &cleanup {
        eprintln!("[копия] не убрать папку восстановления: {error}");
    }
    result.and(cleanup)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::AtomicU64;

    struct Temp(PathBuf);

    impl Temp {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
            let path = std::env::temp_dir().join(format!("windgets-backup-test-{}-{stamp}-{id}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn put(&self, file: &str, value: Value) {
            put(&self.0.join(file), &value);
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn put(path: &Path, value: &Value) {
        crate::disk::write_atomically(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
    }

    fn read(path: &Path) -> Value {
        serde_json::from_str(&crate::disk::read_existing(path).unwrap().unwrap()).unwrap()
    }

    fn protect(text: &str) -> Result<String, String> {
        Ok(format!("protected:{text}"))
    }

    fn unprotect(text: &str) -> Result<String, String> {
        text.strip_prefix("protected:").map(str::to_owned).ok_or_else(|| "не расшифровать".to_owned())
    }

    fn document(files: Value) -> Value {
        json!({"format": FORMAT, "version": 1, "app": "0.1.0", "created": "2024-02-29T12:34:56Z", "files": files})
    }

    fn inspect(value: &Value) -> Result<Inspected, BackupError> {
        parse_backup(&serde_json::to_string(value).unwrap())
    }

    fn before(dir: &Path) -> Vec<PathBuf> {
        fs::read_dir(dir).unwrap().map(|entry| entry.unwrap().path())
            .filter(|path| path.file_name().unwrap().to_string_lossy().starts_with("before-restore-")).collect()
    }

    #[test]
    fn export_inspect_stage_apply_preserves_files_and_secrets() {
        let dir = Temp::new();
        let originals = json!({
            "settings.json": {"widgets": {"cpu": {"enabled": true}, "gpu": {"enabled": false}}},
            ".window-state.json": {"notes": {"x": 100, "y": 200}},
            "sound-window-anchor.json": {"x": 20, "y": 30},
            "notes.json": {"pages": ["Первая", "Вторая"], "active": 1},
            "todo.json": {"lists": [[{"id": 1, "text": "Дело", "done": false}], []], "active": 0},
            "mail.json": {"accounts": [
                {"id": "one", "login": "sample@example.test", "secret": "protected:mail-password", "extra": true},
                {"id": "two", "secret": null}, {"id": "three", "secret": ""}
            ], "preview": true},
            "totp.json": {"accounts": [{"id": "key", "label": "Sample", "secret": "protected:JBSWY3DP", "extra": 7}]},
            "spotify.json": {"client_id": "sample-client", "refresh_token_protected": "protected:spotify-token", "refresh_token": "legacy-token"},
            "media.json": {"sources": [{"id": "player", "enabled": false}]},
            "speedtest.json": {"reports": [{"download": 100}]}
        });
        for (file, value) in originals.as_object().unwrap() {
            dir.put(file, value.clone());
        }
        dir.put("totp.bad.json", json!({"secret": "не включать"}));
        dir.put(".window-state.good.json", json!({"не": "включать"}));
        let exported = collect_backup(&dir.0, "0.1.0", 1709210096, unprotect).unwrap();
        assert_eq!(exported["warning"], WARNING);
        assert_eq!(exported["created"], "2024-02-29T12:34:56Z");
        assert_eq!(exported["files"].as_object().unwrap().len(), FILES.len());
        assert!(exported["files"]["mail.json"]["accounts"][0].get("secret").is_none());
        assert_eq!(exported["files"]["mail.json"]["accounts"][0]["secret_plain"], "mail-password");
        assert_eq!(exported["files"]["mail.json"]["accounts"][1]["secret_plain"], Value::Null);
        assert_eq!(exported["files"]["mail.json"]["accounts"][2]["secret_plain"], "");
        assert_eq!(exported["files"]["totp.json"]["accounts"][0]["secret_plain"], "JBSWY3DP");
        assert!(exported["files"]["spotify.json"].get("refresh_token_protected").is_none());
        assert_eq!(exported["files"]["spotify.json"]["refresh_token_plain"], "spotify-token");
        assert_eq!(exported["files"]["spotify.json"]["refresh_token"], "legacy-token");
        let inspected = inspect(&exported).unwrap();
        let summary = summarize("copy.json".to_owned(), &inspected);
        assert_eq!((summary.mail, summary.totp, summary.notes, summary.todo, summary.widgets), (3, 1, 2, 2, 1));
        assert!(summary.spotify);
        assert_eq!(summary.file, "copy.json");
        assert_eq!(summary.created, "2024-02-29T12:34:56Z");
        stage_backup(&dir.0, inspected, protect).unwrap();
        assert_eq!(fs::read(dir.0.join("restore/ready")).unwrap(), b"");
        for (file, value) in originals.as_object().unwrap() {
            assert_eq!(&read(&dir.0.join("restore").join(file)), value);
        }
        apply_pending(&dir.0);
        assert!(!dir.0.join("restore").exists());
        let previous = before(&dir.0);
        assert_eq!(previous.len(), 1);
        for (file, value) in originals.as_object().unwrap() {
            assert_eq!(&read(&dir.0.join(file)), value);
            assert_eq!(&read(&previous[0].join(file)), value);
        }
        assert!(dir.0.join("totp.bad.json").is_file());
    }

    #[test]
    fn legacy_single_mail_keeps_shape_and_protects_its_secret() {
        let dir = Temp::new();
        let original = json!({"host": "imap.example.test", "secret": "protected:old-password", "preview": true, "extra": [1, 2]});
        dir.put("mail.json", original.clone());
        let exported = collect_backup(&dir.0, "0.1.0", 0, unprotect).unwrap();
        assert_eq!(exported["files"]["mail.json"]["secret_plain"], "old-password");
        assert!(exported["files"]["mail.json"].get("accounts").is_none());
        stage_backup(&dir.0, inspect(&exported).unwrap(), protect).unwrap();
        assert_eq!(read(&dir.0.join("restore/mail.json")), original);
    }

    #[test]
    fn foreign_format_and_future_version_are_rejected() {
        let mut value = document(json!({}));
        value["format"] = json!("other-app");
        assert!(matches!(inspect(&value), Err(BackupError::Format)));
        value["format"] = json!(FORMAT);
        value["version"] = json!(2);
        assert!(matches!(inspect(&value), Err(BackupError::Version)));
        for version in [json!(0), json!(-1), json!("1"), Value::Null] {
            value["version"] = version;
            assert!(matches!(inspect(&value), Err(BackupError::Format)));
        }
        assert!(matches!(parse_backup("не JSON"), Err(BackupError::Format)));
    }

    #[test]
    fn unreadable_or_malformed_file_is_not_silently_omitted() {
        let dir = Temp::new();
        assert!(collect_backup(&dir.0, "0.1.0", 0, unprotect).is_ok());
        fs::create_dir(dir.0.join("settings.json")).unwrap();
        let expected = BackupError::Unreadable { file: "settings.json".to_owned() };
        assert_eq!(collect_backup(&dir.0, "0.1.0", 0, unprotect).unwrap_err(), expected);
        fs::remove_dir(dir.0.join("settings.json")).unwrap();
        crate::disk::write_atomically(&dir.0.join("settings.json"), b"{broken").unwrap();
        assert_eq!(collect_backup(&dir.0, "0.1.0", 0, unprotect).unwrap_err(), expected);
        crate::disk::write_atomically(&dir.0.join("settings.json"), [0xff]).unwrap();
        assert_eq!(collect_backup(&dir.0, "0.1.0", 0, unprotect).unwrap_err(), expected);
    }

    #[test]
    fn decryption_failure_aborts_export_without_changing_source() {
        let dir = Temp::new();
        dir.put("totp.json", json!({"accounts": [{"secret": "unavailable"}]}));
        assert_eq!(collect_backup(&dir.0, "0.1.0", 0, unprotect).unwrap_err(),
            BackupError::Secret { file: "totp.json".to_owned() });
        assert_eq!(read(&dir.0.join("totp.json"))["accounts"][0]["secret"], "unavailable");
    }

    #[test]
    fn unknown_names_never_reach_staging_or_destination() {
        let dir = Temp::new();
        let config = dir.0.join("config");
        fs::create_dir(&config).unwrap();
        let value = document(json!({
            "settings.json": {}, "../escaped.json": {"bad": true},
            "..\\escaped.json": {}, "C:\\escaped.json": {}, "extra.json": {},
            ".window-state.good.json": {}, "restore/ready": {}
        }));
        let inspected = inspect(&value).unwrap();
        assert_eq!(inspected.files.len(), 1);
        stage_backup(&config, inspected, protect).unwrap();
        assert_eq!(fs::read_dir(config.join("restore")).unwrap().count(), 2);
        apply_pending_at(&config, 0).unwrap();
        assert!(!dir.0.join("escaped.json").exists());
        assert!(!config.join("extra.json").exists());
        assert!(!config.join(".window-state.good.json").exists());
        assert_eq!(read(&config.join("settings.json")), json!({}));
    }

    #[test]
    fn invalid_known_files_are_rejected_before_staging() {
        for (file, value) in [
            (".window-state.json", json!([])), ("settings.json", json!({"widgets": []})),
            ("notes.json", json!({"pages": [3]})), ("todo.json", json!({"lists": [false]})),
            ("mail.json", json!({"accounts": null})), ("mail.json", json!({"accounts": [5]})),
            ("mail.json", json!({"accounts": [{"port": "993"}]})),
            ("totp.json", json!({})), ("totp.json", json!({"accounts": [null]})),
            ("totp.json", json!({"accounts": [{"secret_plain": null}]})),
            ("spotify.json", json!({"refresh_token_plain": 9})),
            ("media.json", json!({"sources": false})),
        ] {
            let mut files = Map::new();
            files.insert(file.to_owned(), value);
            assert!(matches!(inspect(&document(Value::Object(files))),
                Err(BackupError::Invalid { file: invalid }) if invalid == file));
        }
    }

    #[test]
    fn empty_summary_and_legacy_spotify_token_are_counted() {
        let inspected = inspect(&document(json!({}))).unwrap();
        let summary = summarize("empty.json".to_owned(), &inspected);
        assert_eq!((summary.mail, summary.totp, summary.notes, summary.todo, summary.widgets), (0, 0, 0, 0, 0));
        assert!(!summary.spotify);
        let inspected = inspect(&document(json!({"spotify.json": {"refresh_token": "legacy"}}))).unwrap();
        assert!(summarize("legacy.json".to_owned(), &inspected).spotify);
    }

    #[test]
    fn failed_protection_cleans_partial_stage_and_old_marker() {
        let dir = Temp::new();
        fs::create_dir(dir.0.join("restore")).unwrap();
        crate::disk::write_atomically(&dir.0.join("restore/ready"), b"").unwrap();
        let inspected = inspect(&document(json!({"settings.json": {}, "totp.json": {"accounts": [{"secret_plain": "key"}]}}))).unwrap();
        assert_eq!(stage_backup(&dir.0, inspected, |_| Err("отказ".to_owned())),
            Err(BackupError::Secret { file: "totp.json".to_owned() }));
        assert!(!dir.0.join("restore").exists());
        assert!(!dir.0.join("settings.json").exists());
    }

    #[test]
    fn unwritable_stage_returns_stage_error() {
        let dir = Temp::new();
        // Файл вместо родительского каталога: отказ записи без смены ACL.
        let blocked = dir.0.join("blocked");
        crate::disk::write_atomically(&blocked, b"keep").unwrap();
        let inspected = inspect(&document(json!({"settings.json": {}}))).unwrap();
        assert_eq!(stage_backup(&blocked, inspected, protect), Err(BackupError::Stage));
        assert_eq!(fs::read(blocked).unwrap(), b"keep");
    }

    #[test]
    fn applying_moves_current_files_and_replaces_whole_set() {
        let dir = Temp::new();
        dir.put("settings.json", json!({"old": true}));
        dir.put("notes.json", json!({"pages": ["old"]}));
        fs::create_dir(dir.0.join("restore")).unwrap();
        put(&dir.0.join("restore/settings.json"), &json!({"new": true}));
        put(&dir.0.join("restore/unknown.json"), &json!({"bad": true}));
        crate::disk::write_atomically(&dir.0.join("restore/ready"), b"").unwrap();
        apply_pending_at(&dir.0, 1709210096).unwrap();
        let previous = dir.0.join("before-restore-20240229-123456");
        assert_eq!(read(&previous.join("settings.json")), json!({"old": true}));
        assert_eq!(read(&previous.join("notes.json")), json!({"pages": ["old"]}));
        assert_eq!(read(&dir.0.join("settings.json")), json!({"new": true}));
        assert!(!dir.0.join("notes.json").exists());
        assert!(!dir.0.join("unknown.json").exists());
        assert!(!dir.0.join("restore").exists());
        // Две попытки в одну секунду не затирают предыдущую страховочную копию.
        fs::create_dir(dir.0.join("restore")).unwrap();
        crate::disk::write_atomically(&dir.0.join("restore/ready"), b"").unwrap();
        apply_pending_at(&dir.0, 1709210096).unwrap();
        assert_eq!(before(&dir.0).len(), 2);
        assert_eq!(read(&previous.join("settings.json")), json!({"old": true}));
    }

    #[test]
    fn incomplete_stage_is_removed_without_touching_current_files() {
        let dir = Temp::new();
        dir.put("settings.json", json!({"old": true}));
        fs::create_dir(dir.0.join("restore")).unwrap();
        put(&dir.0.join("restore/settings.json"), &json!({"incomplete": true}));
        apply_pending_at(&dir.0, 0).unwrap();
        assert_eq!(read(&dir.0.join("settings.json")), json!({"old": true}));
        assert!(!dir.0.join("restore").exists());
        assert!(before(&dir.0).is_empty());
        apply_pending_at(&dir.0, 0).unwrap();
    }

    #[test]
    fn failed_install_rolls_back_old_and_removes_new_only_files() {
        let dir = Temp::new();
        dir.put("settings.json", json!({"old": true}));
        dir.put("notes.json", json!({"pages": ["old"]}));
        fs::create_dir(dir.0.join("restore")).unwrap();
        put(&dir.0.join("restore/settings.json"), &json!({"new": true}));
        put(&dir.0.join("restore/.window-state.json"), &json!({"new": true}));
        // Отказ после двух успешных переносов: каталог не может стать JSON.
        fs::create_dir(dir.0.join("restore/notes.json")).unwrap();
        crate::disk::write_atomically(&dir.0.join("restore/ready"), b"").unwrap();
        assert!(apply_pending_at(&dir.0, 0).is_err());
        assert_eq!(read(&dir.0.join("settings.json")), json!({"old": true}));
        assert_eq!(read(&dir.0.join("notes.json")), json!({"pages": ["old"]}));
        assert!(!dir.0.join(".window-state.json").exists());
        assert!(!dir.0.join("restore").exists());
        apply_pending_at(&dir.0, 0).unwrap();
    }

    #[test]
    fn failed_move_of_current_files_rolls_back_already_moved_files() {
        let dir = Temp::new();
        dir.put("settings.json", json!({"old": true}));
        fs::create_dir(dir.0.join("notes.json")).unwrap();
        fs::create_dir(dir.0.join("restore")).unwrap();
        crate::disk::write_atomically(&dir.0.join("restore/ready"), b"").unwrap();
        assert!(apply_pending_at(&dir.0, 0).is_err());
        assert_eq!(read(&dir.0.join("settings.json")), json!({"old": true}));
        assert!(dir.0.join("notes.json").is_dir());
        assert!(!dir.0.join("restore").exists());
    }

    #[test]
    fn restore_discards_foreign_protected_secrets_and_encrypts_only_plain_fields() {
        let dir = Temp::new();
        let value = document(json!({
            "mail.json": {"accounts": [
                {"id": "foreign", "secret": "foreign-dpapi"},
                {"id": "plain", "secret": "foreign-dpapi", "secret_plain": "mail-password"},
                {"id": "null", "secret": "foreign-dpapi", "secret_plain": null},
                {"id": "empty", "secret": "foreign-dpapi", "secret_plain": ""}
            ]},
            "totp.json": {"accounts": [
                {"id": "foreign", "secret": "foreign-dpapi"},
                {"id": "plain", "secret": "foreign-dpapi", "secret_plain": "JBSWY3DP"}
            ]},
            "spotify.json": {"refresh_token_protected": "foreign-dpapi"}
        }));
        stage_backup(&dir.0, inspect(&value).unwrap(), protect).unwrap();
        let mail = read(&dir.0.join("restore/mail.json"));
        assert!(mail["accounts"][0].get("secret").is_none());
        assert_eq!(mail["accounts"][1]["secret"], "protected:mail-password");
        assert_eq!(mail["accounts"][2]["secret"], Value::Null);
        assert_eq!(mail["accounts"][3]["secret"], "");
        let totp = read(&dir.0.join("restore/totp.json"));
        assert!(totp["accounts"][0].get("secret").is_none());
        assert_eq!(totp["accounts"][1]["secret"], "protected:JBSWY3DP");
        assert!(read(&dir.0.join("restore/spotify.json")).get("refresh_token_protected").is_none());
        let value = document(json!({
            "mail.json": {"secret": "foreign-dpapi", "extra": true},
            "spotify.json": {"refresh_token_protected": "foreign-dpapi", "refresh_token_plain": "spotify-token"}
        }));
        stage_backup(&dir.0, inspect(&value).unwrap(), protect).unwrap();
        let mail = read(&dir.0.join("restore/mail.json"));
        assert!(mail.get("secret").is_none());
        assert_eq!(mail["extra"], true);
        assert_eq!(read(&dir.0.join("restore/spotify.json"))["refresh_token_protected"], "protected:spotify-token");
    }

    #[test]
    fn export_keeps_existing_plain_fields_when_protected_fields_are_absent() {
        for (file, value) in [
            ("mail.json", json!({"secret_plain": "keep-mail"})),
            ("totp.json", json!({"accounts": [{"secret_plain": "keep-totp"}]})),
            ("spotify.json", json!({"refresh_token_plain": "keep-spotify"})),
        ] {
            let mut exported = value.clone();
            change_secrets(file, &mut exported, true, &unprotect).unwrap();
            assert_eq!(exported, value);
        }
    }

    #[test]
    fn interrupted_install_resumes_without_moving_installed_files() {
        // Последний вариант — сбой после всей установки, до снятия ready.
        for installed in [1, FILES.len() / 2, FILES.len()] {
            let dir = Temp::new();
            let restore = dir.0.join("restore");
            fs::create_dir(&restore).unwrap();
            let previous = resolve_before_dir(&dir.0, &restore, 0).unwrap();
            for file in FILES {
                put(&previous.join(file), &json!({"old": file}));
                put(&restore.join(file), &json!({"new": file}));
            }
            crate::disk::write_atomically(&restore.join("installing"), b"").unwrap();
            crate::disk::write_atomically(&restore.join("ready"), b"").unwrap();
            for file in &FILES[..installed] {
                fs::rename(restore.join(file), dir.0.join(file)).unwrap();
            }
            apply_pending_at(&dir.0, 1).unwrap();
            assert_eq!(before(&dir.0), vec![previous.clone()]);
            for file in FILES {
                assert_eq!(read(&dir.0.join(file)), json!({"new": file}));
                assert_eq!(read(&previous.join(file)), json!({"old": file}));
            }
            assert!(!restore.exists());
        }
    }

    #[test]
    fn interrupted_move_resumes_in_the_same_before_directory() {
        for moved in [1, FILES.len()] {
            let dir = Temp::new();
            let restore = dir.0.join("restore");
            fs::create_dir(&restore).unwrap();
            let previous = resolve_before_dir(&dir.0, &restore, 0).unwrap();
            for file in FILES {
                dir.put(file, json!({"old": file}));
                put(&restore.join(file), &json!({"new": file}));
            }
            for file in &FILES[..moved] {
                fs::rename(dir.0.join(file), previous.join(file)).unwrap();
            }
            crate::disk::write_atomically(&restore.join("ready"), b"").unwrap();
            apply_pending_at(&dir.0, 1).unwrap();
            assert_eq!(before(&dir.0), vec![previous.clone()]);
            for file in FILES {
                assert_eq!(read(&dir.0.join(file)), json!({"new": file}));
                assert_eq!(read(&previous.join(file)), json!({"old": file}));
            }
            assert!(!restore.exists());
        }
    }

    #[test]
    fn failed_resumed_move_returns_files_saved_by_the_previous_run() {
        let dir = Temp::new();
        let restore = dir.0.join("restore");
        fs::create_dir(&restore).unwrap();
        let previous = resolve_before_dir(&dir.0, &restore, 0).unwrap();
        put(&previous.join("settings.json"), &json!({"old": true}));
        dir.put(".window-state.json", json!({"old-places": true}));
        fs::create_dir(dir.0.join("notes.json")).unwrap();
        crate::disk::write_atomically(&restore.join("ready"), b"").unwrap();
        assert!(apply_pending_at(&dir.0, 1).is_err());
        assert_eq!(read(&dir.0.join("settings.json")), json!({"old": true}));
        assert_eq!(read(&dir.0.join(".window-state.json")), json!({"old-places": true}));
        assert_eq!(before(&dir.0), vec![previous]);
        assert!(!restore.exists());
    }

    #[cfg(windows)]
    #[test]
    fn failed_move_rename_rolls_back_files_saved_before_and_after_restart() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = Temp::new();
        let restore = dir.0.join("restore");
        fs::create_dir(&restore).unwrap();
        let previous = resolve_before_dir(&dir.0, &restore, 0).unwrap();
        put(&previous.join("settings.json"), &json!({"old": true}));
        dir.put(".window-state.json", json!({"old-places": true}));
        dir.put("notes.json", json!({"pages": ["old"]}));
        // Чтение атрибутов разрешено, а rename занятого файла Windows отклонит.
        let locked = fs::OpenOptions::new().read(true).share_mode(0)
            .open(dir.0.join("notes.json")).unwrap();
        crate::disk::write_atomically(&restore.join("ready"), b"").unwrap();
        assert!(apply_pending_at(&dir.0, 1).is_err());
        drop(locked);
        assert_eq!(read(&dir.0.join("settings.json")), json!({"old": true}));
        assert_eq!(read(&dir.0.join(".window-state.json")), json!({"old-places": true}));
        assert_eq!(read(&dir.0.join("notes.json")), json!({"pages": ["old"]}));
        assert_eq!(before(&dir.0), vec![previous]);
        assert!(!restore.exists());
    }

    #[test]
    fn failed_resumed_install_rolls_back_files_installed_by_the_previous_run() {
        let dir = Temp::new();
        let restore = dir.0.join("restore");
        fs::create_dir(&restore).unwrap();
        let previous = resolve_before_dir(&dir.0, &restore, 0).unwrap();
        put(&previous.join("settings.json"), &json!({"old": true}));
        dir.put("settings.json", json!({"new": true}));
        dir.put(".window-state.json", json!({"new-only": true}));
        // Настоящий отказ rename: целевое имя занято каталогом после сбоя.
        fs::create_dir(dir.0.join("notes.json")).unwrap();
        put(&restore.join("notes.json"), &json!({"pages": ["new"]}));
        crate::disk::write_atomically(&restore.join("installing"), b"").unwrap();
        crate::disk::write_atomically(&restore.join("ready"), b"").unwrap();
        assert!(apply_pending_at(&dir.0, 1).is_err());
        assert_eq!(read(&dir.0.join("settings.json")), json!({"old": true}));
        assert!(!dir.0.join(".window-state.json").exists());
        assert!(dir.0.join("notes.json").is_dir());
        assert_eq!(before(&dir.0), vec![previous]);
        assert!(!restore.exists());
    }

    #[test]
    fn window_state_companions_are_saved_but_never_exported() {
        let dir = Temp::new();
        let old_places = json!({"notes": {"x": 10}});
        let new_places = json!({"notes": {"x": 200}});
        dir.put(".window-state.json", old_places.clone());
        for file in PLACE_COMPANIONS {
            crate::disk::write_atomically(&dir.0.join(file), file).unwrap();
        }
        let exported = collect_backup(&dir.0, "0.1.0", 0, unprotect).unwrap();
        assert_eq!(exported["files"].as_object().unwrap().len(), 1);
        stage_backup(&dir.0, inspect(&document(json!({".window-state.json": new_places}))).unwrap(), protect).unwrap();
        apply_pending_at(&dir.0, 0).unwrap();
        let previous = before(&dir.0);
        assert_eq!(read(&previous[0].join(".window-state.json")), old_places);
        assert_eq!(read(&dir.0.join(".window-state.json")), new_places);
        for file in PLACE_COMPANIONS {
            assert!(!dir.0.join(file).exists());
            assert_eq!(fs::read_to_string(previous[0].join(file)).unwrap(), file);
        }
    }

    #[test]
    fn failed_install_returns_all_window_state_companions() {
        let dir = Temp::new();
        for file in PLACE_COMPANIONS {
            crate::disk::write_atomically(&dir.0.join(file), file).unwrap();
        }
        stage_backup(&dir.0, inspect(&document(json!({"settings.json": {}}))).unwrap(), protect).unwrap();
        fs::create_dir(dir.0.join("restore/notes.json")).unwrap();
        assert!(apply_pending_at(&dir.0, 0).is_err());
        for file in PLACE_COMPANIONS {
            assert_eq!(fs::read_to_string(dir.0.join(file)).unwrap(), file);
        }
        assert!(!dir.0.join("settings.json").exists());
        assert!(!dir.0.join("restore").exists());
    }

    #[test]
    fn interrupted_rollback_resumes_in_both_phases() {
        for removing_new in [true, false] {
            let dir = Temp::new();
            let restore = dir.0.join("restore");
            fs::create_dir(&restore).unwrap();
            let previous = resolve_before_dir(&dir.0, &restore, 0).unwrap();
            put(&previous.join("settings.json"), &json!({"old": true}));
            if removing_new {
                dir.put("settings.json", json!({"new": true}));
                dir.put(".window-state.json", json!({"new-only": true}));
                crate::disk::write_atomically(&restore.join("installing"), b"").unwrap();
            } else {
                // Этот прежний файл уже возвращён до сбоя: больше его не уносим.
                dir.put(".window-state.json", json!({"old-places": true}));
                put(&restore.join("settings.json"), &json!({"new": true}));
                put(&restore.join(".window-state.json"), &json!({"new-places": true}));
            }
            crate::disk::write_atomically(&restore.join("rolling-back"), b"").unwrap();
            crate::disk::write_atomically(&restore.join("ready"), b"").unwrap();
            assert!(apply_pending_at(&dir.0, 1).is_err());
            assert_eq!(read(&dir.0.join("settings.json")), json!({"old": true}));
            if removing_new {
                assert!(!dir.0.join(".window-state.json").exists());
            } else {
                assert_eq!(read(&dir.0.join(".window-state.json")), json!({"old-places": true}));
            }
            assert_eq!(before(&dir.0), vec![previous]);
            assert!(!restore.exists());
            apply_pending_at(&dir.0, 1).unwrap();
        }
    }

    #[test]
    fn failed_rollback_keeps_ready_until_old_files_can_be_returned() {
        let dir = Temp::new();
        let restore = dir.0.join("restore");
        fs::create_dir(&restore).unwrap();
        let previous = resolve_before_dir(&dir.0, &restore, 0).unwrap();
        put(&previous.join("settings.json"), &json!({"old": true}));
        put(&restore.join("settings.json"), &json!({"new": true}));
        fs::create_dir(dir.0.join("settings.json")).unwrap();
        crate::disk::write_atomically(&restore.join("installing"), b"").unwrap();
        crate::disk::write_atomically(&restore.join("ready"), b"").unwrap();
        assert!(apply_pending_at(&dir.0, 1).is_err());
        assert!(restore.join("ready").is_file());
        assert!(restore.join("rolling-back").is_file());
        assert_eq!(read(&previous.join("settings.json")), json!({"old": true}));
        fs::remove_dir(dir.0.join("settings.json")).unwrap();
        assert!(apply_pending_at(&dir.0, 2).is_err());
        assert_eq!(read(&dir.0.join("settings.json")), json!({"old": true}));
        assert!(!restore.exists());
        apply_pending_at(&dir.0, 3).unwrap();
    }

    #[test]
    fn operations_are_exclusive_and_release_on_drop() {
        let state = Backup::default();
        let operation = Operation::begin(&state.busy).unwrap();
        assert!(matches!(Operation::begin(&state.busy), Err(BackupError::Busy)));
        drop(operation);
        assert!(Operation::begin(&state.busy).is_ok());
    }

    #[test]
    fn utc_dates_cover_leap_centuries_and_year_boundaries() {
        for (seconds, expected) in [
            (0, "1970-01-01T00:00:00Z"), (86399, "1970-01-01T23:59:59Z"),
            (951782400, "2000-02-29T00:00:00Z"), (1709164800, "2024-02-29T00:00:00Z"),
            (1735689599, "2024-12-31T23:59:59Z"), (1735689600, "2025-01-01T00:00:00Z"),
            (4107542400, "2100-03-01T00:00:00Z"), (13574563200, "2400-02-29T00:00:00Z"),
        ] {
            assert_eq!(utc_stamp(seconds), expected);
        }
    }
}
