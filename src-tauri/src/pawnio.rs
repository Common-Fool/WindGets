//! Драйвер PawnIO — по согласию человека, как докачиваемые программы.
//!
//! Через него LibreHardwareMonitor читает процессор (MSR и SMU) и чип платы
//! (Super I/O): температуру, частоту и потребление ЦП, вентиляторы, Vcore и
//! температуру платы. Без драйвера всё это приходит нулями или не приходит
//! вовсе, а видеокарте, памяти, сети и накопителям он не нужен.
//!
//! ВАЖНО: **это драйвер ядра для всей системы, а не файл в нашей папке.**
//! Поэтому так же прозрачно, как ffmpeg, но строже: ставим только по нажатию,
//! официальным установщиком автора с его выпусков на GitHub и со сверкой
//! SHA-256 из API. Удаляем только поставленный нами — стоящий от FanControl
//! или самого LHM ставил человек, и на нём держатся его программы. Удаление
//! Windgets драйвер не трогает по той же причине.
//!
//! Готовность — не запись в реестре, а открывшееся устройство: ровно так его
//! открывает сама библиотека (`PawnIo.LoadModuleFromResource`). Запись есть,
//! а устройство не открылось — драйвер стоит, но не загружен, и нужна
//! перезагрузка.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::tools::catalog::AssetRule;
use crate::tools::{fetch, github, Failure as Download};

const REPO: &str = "namazso/PawnIO.Setup";
const ASSET: &str = "PawnIO_setup.exe";

/// Куда ведут «Что это» в карточке, пункт меню и кнопка в настройках: выпуски
/// автора — оттуда же человек может поставить драйвер и сам.
pub const PAGE: &str = "https://github.com/namazso/PawnIO.Setup/releases/latest";

const EVENT: &str = "pawnio";

/// Размер установщика для текста согласия — с выпуска 2.2.0, а не из сети: до
/// нажатия кнопки в сеть не ходим.
const DOWNLOAD_BYTES: u64 = 3_410_960;

/// Старше LHM 0.9.6 не работает: сам LHM на версии ниже второй предлагает
/// обновиться.
const MIN_MAJOR: u32 = 2;

/// Установщик ставит драйвер за секунды; срок — на случай, если он встал.
const INSTALLER_LIMIT: Duration = Duration::from_secs(300);

/// Как часто опрос датчиков проверяет, не поставили ли драйвер мимо нас.
const RECHECK: Duration = Duration::from_secs(10);

/// Пауза между гашением сайдкара и установщиком: процесс уходит не мгновенно,
/// а открытый им драйвер установщик заменить не сможет.
const SIDECAR_GONE: Duration = Duration::from_millis(1500);

/// Коды выхода установщика — ошибки DOS (с PawnIO 2.2.0). 3010 и 1641 —
/// «готово, нужна перезагрузка»: это не отказ.
const DONE: [i32; 3] = [0, 3010, 1641];

const UNINSTALL_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PawnIO";
const DEVICE: &str = r"\\?\GLOBALROOT\Device\PawnIO";
const STORE: &str = "pawnio.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    /// Устройство открывается — датчики процессора и платы читаются.
    Ready,
    Missing,
    /// Стоит версия, с которой LHM не работает.
    Outdated,
    /// Стоит, но не загружен: после установки нужна перезагрузка.
    Restart,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "stage", rename_all = "camelCase")]
pub enum Job {
    Checking,
    Downloading { received: u64, total: Option<u64> },
    Installing,
    Removing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Code {
    NoBuild,
    NoChecksum,
    BadChecksum,
    Network,
    RateLimited,
    Disk,
    Cancelled,
    /// Установщик вернул ошибку — её код в `exit`.
    Installer,
    Uninstaller,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    code: Code,
    /// Код выхода установщика или удаления, если дошло до них.
    exit: Option<i32>,
}

impl From<Download> for Problem {
    fn from(failure: Download) -> Self {
        let code = match failure {
            Download::NoBuild => Code::NoBuild,
            Download::NoChecksum => Code::NoChecksum,
            Download::BadChecksum => Code::BadChecksum,
            Download::Network => Code::Network,
            Download::RateLimited => Code::RateLimited,
            Download::Cancelled => Code::Cancelled,
            Download::Unpack | Download::Disk | Download::InUse => Code::Disk,
        };
        Problem { code, exit: None }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    status: Status,
    /// Версия из записи установщика в реестре.
    version: Option<String>,
    /// Поставлен нами — только такой предлагаем удалить.
    ours: bool,
    job: Option<Job>,
    failure: Option<Problem>,
    download_bytes: u64,
    page: &'static str,
}

/// Что поставили мы — рядом с докачанными программами, это данные машины.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct Stored {
    ours: bool,
    tag: Option<String>,
}

struct Inner {
    stored: Stored,
    job: Option<Job>,
    failure: Option<Problem>,
    /// Что видела последняя проверка опроса и когда.
    seen: Option<Status>,
    checked: Option<Instant>,
}

pub struct Pawnio {
    root: Option<PathBuf>,
    inner: Mutex<Inner>,
    busy: AtomicBool,
    cancel: AtomicBool,
}

impl Pawnio {
    pub fn new(app: &AppHandle) -> Self {
        let root = app.path().app_local_data_dir().ok().map(|dir| dir.join("tools"));
        let stored = root.as_deref().map(load).unwrap_or_default();
        Self {
            root,
            inner: Mutex::new(Inner { stored, job: None, failure: None, seen: None, checked: None }),
            busy: AtomicBool::new(false),
            cancel: AtomicBool::new(false),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn view(&self) -> View {
        let version = registry_value("DisplayVersion");
        let status = pretend().unwrap_or_else(|| decide(device_opens(), version.as_deref()));
        let inner = self.lock();
        View {
            status,
            version,
            ours: inner.stored.ours,
            job: inner.job,
            failure: inner.failure,
            download_bytes: DOWNLOAD_BYTES,
            page: PAGE,
        }
    }

    fn set_job(&self, job: Option<Job>) {
        self.lock().job = job;
    }

    fn remember(&self, stored: Stored) {
        let mut inner = self.lock();
        inner.stored = stored;
        if let Some(root) = &self.root {
            save(root, &inner.stored);
        }
    }
}

/// Отладочная проверка слоя вживую на машине, где драйвер стоит:
/// `HW_PAWNIO_PRETEND=missing` (или `outdated`, `restart`) в окружении
/// разработки. Только вид — проверка опроса и кнопки работают по-настоящему,
/// и «Установить» запустит настоящий установщик. В собранном приложении не
/// действует.
fn pretend() -> Option<Status> {
    if !cfg!(debug_assertions) {
        return None;
    }
    match std::env::var("HW_PAWNIO_PRETEND").ok()?.as_str() {
        "missing" => Some(Status::Missing),
        "outdated" => Some(Status::Outdated),
        "restart" => Some(Status::Restart),
        _ => None,
    }
}

/// Готов ли драйвер — по устройству и записи в реестре.
fn decide(opens: bool, version: Option<&str>) -> Status {
    let major = version.and_then(|text| text.split('.').next()).and_then(|part| part.trim().parse::<u32>().ok());
    match (opens, version, major) {
        (_, Some(_), Some(major)) if major < MIN_MAJOR => Status::Outdated,
        (true, _, _) => Status::Ready,
        (false, Some(_), _) => Status::Restart,
        (false, None, _) => Status::Missing,
    }
}

/// Открывается ли устройство драйвера. Сразу закрываем: держать его — дело
/// сайдкара. Права на это есть — приложение повышено.
fn device_opens() -> bool {
    std::fs::OpenOptions::new().read(true).write(true).open(DEVICE).is_ok()
}

/// Строка из записи установщика PawnIO в `HKLM\…\Uninstall`.
#[cfg(windows)]
fn registry_value(name: &str) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};

    let key: Vec<u16> = UNINSTALL_KEY.encode_utf16().chain(std::iter::once(0)).collect();
    let value: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut buffer = [0u16; 1024];
    let mut size = std::mem::size_of_val(&buffer) as u32;

    // SAFETY: обе строки оканчиваются нулём, буфер живёт до конца вызова, а
    // его размер в байтах передан вместе с ним
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(key.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }

    let length = (size as usize / 2).min(buffer.len());
    let text = String::from_utf16_lossy(&buffer[..length]);
    let text = text.trim_end_matches('\0').trim();
    (!text.is_empty()).then(|| text.to_owned())
}

#[cfg(not(windows))]
fn registry_value(_name: &str) -> Option<String> {
    None
}

/// Путь удаления из `UninstallString`: `"C:\Program Files\PawnIO\uninstall.exe" -uninstall`.
///
/// Строка из `HKLM` — туда без прав не пишут, — но запускаем мы её с правами,
/// поэтому берём только exe по имени `uninstall.exe`, без чужих ключей.
fn uninstaller(command: &str) -> Option<PathBuf> {
    let command = command.trim();
    let path = match command.strip_prefix('"') {
        Some(rest) => rest.split('"').next()?,
        None => command.split(' ').next()?,
    };
    let path = PathBuf::from(path);
    let named = path.file_name().is_some_and(|name| name.eq_ignore_ascii_case("uninstall.exe"));
    (named && path.is_absolute()).then_some(path)
}

fn load(root: &Path) -> Stored {
    std::fs::read_to_string(root.join(STORE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save(root: &Path, stored: &Stored) {
    let write = || -> Result<(), String> {
        std::fs::create_dir_all(root).map_err(|error| error.to_string())?;
        let text = serde_json::to_string_pretty(stored).map_err(|error| error.to_string())?;
        crate::disk::write_atomically(&root.join(STORE), &text).map_err(|error| error.to_string())
    };
    if let Err(error) = write() {
        eprintln!("[pawnio] {STORE} не записался: {error}");
    }
}

pub fn publish(app: &AppHandle) {
    if let Some(pawnio) = app.try_state::<Pawnio>() {
        let _ = app.emit(EVENT, pawnio.view());
    }
}

/// Проверка из опроса датчиков: не поставили ли драйвер мимо нас.
///
/// Только пока он не готов и не чаще [`RECHECK`]. Стал готов — сайдкар
/// перезапускается, иначе LHM так и читал бы без драйвера до выхода.
pub fn watch(app: &AppHandle) {
    let Some(pawnio) = app.try_state::<Pawnio>() else {
        return;
    };
    if pawnio.busy.load(Ordering::SeqCst) {
        return;
    }
    {
        let inner = pawnio.lock();
        if inner.seen == Some(Status::Ready) || inner.checked.is_some_and(|at| at.elapsed() < RECHECK) {
            return;
        }
    }

    let status = decide(device_opens(), registry_value("DisplayVersion").as_deref());
    let before = {
        let mut inner = pawnio.lock();
        inner.checked = Some(Instant::now());
        inner.seen.replace(status)
    };
    if before.is_some() && before != Some(status) {
        if status == Status::Ready {
            crate::poller::restart();
        }
        publish(app);
    }
}

/// Опускает `busy` и снимает ход работы — и при панике тоже.
struct Busy(AppHandle);

impl Drop for Busy {
    fn drop(&mut self) {
        crate::poller::hold(false);
        if let Some(pawnio) = self.0.try_state::<Pawnio>() {
            pawnio.busy.store(false, Ordering::SeqCst);
            let mut inner = pawnio.lock();
            inner.job = None;
            // Проверка опроса начнёт заново: состояние только что сменилось
            inner.seen = None;
            inner.checked = None;
        }
        publish(&self.0);
    }
}

fn occupy(app: &AppHandle) -> Option<Busy> {
    let pawnio = app.state::<Pawnio>();
    if pawnio.busy.swap(true, Ordering::SeqCst) {
        return None;
    }
    pawnio.cancel.store(false, Ordering::SeqCst);
    pawnio.lock().failure = None;
    Some(Busy(app.clone()))
}

/// Скачивает установщик и ставит драйвер.
async fn install(app: &AppHandle) -> Result<(), Problem> {
    let pawnio = app.state::<Pawnio>();
    let root = pawnio.root.clone().ok_or(Problem { code: Code::Disk, exit: None })?;

    pawnio.set_job(Some(Job::Checking));
    publish(app);

    let client = crate::tools::client()?;
    let release = github::latest(&client, REPO).await?;
    let asset = github::pick_asset(&release, AssetRule::Exact(ASSET))?;
    let digest = github::parse_digest(asset.digest.as_deref())?;

    let dir = root.join("pawnio");
    std::fs::create_dir_all(&dir).map_err(|error| disk(&error))?;
    let partial = dir.join(format!("{ASSET}.download"));
    let setup = dir.join(ASSET);

    pawnio.set_job(Some(Job::Downloading { received: 0, total: Some(asset.size) }));
    publish(app);
    fetch::download(&client, &asset.browser_download_url, digest, &partial, &pawnio.cancel, |received, total| {
        pawnio.set_job(Some(Job::Downloading { received, total: total.or(Some(asset.size)) }));
        publish(app);
    })
    .await?;
    let _ = std::fs::remove_file(&setup);
    std::fs::rename(&partial, &setup).map_err(|error| disk(&error))?;

    pawnio.set_job(Some(Job::Installing));
    publish(app);

    let outcome = run_quietly(Command::new(&setup), &["-install", "-silent"]).await;
    // Установщик своё дело сделал: драйвер стоит в Program Files, и для
    // удаления у него свой uninstall.exe
    let _ = std::fs::remove_dir_all(&dir);

    match outcome {
        Some(code) if DONE.contains(&code) => {
            eprintln!("[pawnio] установлен {} (код {code})", release.tag_name);
            pawnio.remember(Stored { ours: true, tag: Some(release.tag_name.clone()) });
            Ok(())
        }
        exit => {
            eprintln!("[pawnio] установщик отказал: {exit:?}");
            Err(Problem { code: Code::Installer, exit })
        }
    }
}

/// Запускает установщик или удаление с погашенным сайдкаром и ждёт конца.
/// `None` — не запустился, убит по сроку или код не узнать.
async fn run_quietly(mut command: Command, args: &[&str]) -> Option<i32> {
    command.args(args);
    crate::poller::hold(true);
    tokio::time::sleep(SIDECAR_GONE).await;

    // Процесс ждём в пуле, а не в потоке рантайма
    let ran = tauri::async_runtime::spawn_blocking(move || crate::child::run(command, INSTALLER_LIMIT)).await;
    match ran {
        Ok(Ok((code, lines))) => {
            for line in lines.iter().filter(|line| !line.text().trim().is_empty()) {
                eprintln!("[pawnio] {}", line.text());
            }
            code
        }
        Ok(Err(error)) => {
            eprintln!("[pawnio] установщик не запустился: {error}");
            None
        }
        Err(error) => {
            eprintln!("[pawnio] поток установщика упал: {error}");
            None
        }
    }
}

fn disk(error: &std::io::Error) -> Problem {
    eprintln!("[pawnio] файловая операция не прошла: {error}");
    Problem { code: Code::Disk, exit: None }
}

#[tauri::command]
pub fn pawnio_state(state: State<'_, Pawnio>) -> View {
    state.view()
}

/// Ставит или обновляет драйвер. Только по нажатию человека.
#[tauri::command]
pub fn pawnio_install(app: AppHandle) -> Result<(), String> {
    let busy = occupy(&app).ok_or_else(|| "busy".to_owned())?;
    publish(&app);

    tauri::async_runtime::spawn(async move {
        let _busy = busy;
        if let Err(problem) = install(&app).await {
            app.state::<Pawnio>().lock().failure = Some(problem);
        }
    });
    Ok(())
}

/// Отменяет скачивание. Сам установщик не прерываем: драйвер, брошенный
/// посреди установки, хуже любого исхода.
#[tauri::command]
pub fn pawnio_cancel(state: State<'_, Pawnio>) {
    state.cancel.store(true, Ordering::SeqCst);
}

/// Удаляет драйвер — только поставленный нами, его же удалением.
#[tauri::command]
pub fn pawnio_remove(app: AppHandle) -> Result<(), String> {
    if !app.state::<Pawnio>().lock().stored.ours {
        return Err("notOurs".to_owned());
    }
    let Some(path) = registry_value("UninstallString").as_deref().and_then(uninstaller) else {
        // Записи нет — драйвер уже удалили мимо нас; забываем о нём
        app.state::<Pawnio>().remember(Stored::default());
        publish(&app);
        return Err("missing".to_owned());
    };
    let busy = occupy(&app).ok_or_else(|| "busy".to_owned())?;
    app.state::<Pawnio>().set_job(Some(Job::Removing));
    publish(&app);

    tauri::async_runtime::spawn(async move {
        let _busy = busy;
        let pawnio = app.state::<Pawnio>();
        match run_quietly(Command::new(&path), &["-uninstall", "-silent"]).await {
            Some(code) if DONE.contains(&code) => {
                eprintln!("[pawnio] удалён (код {code})");
                pawnio.remember(Stored::default());
            }
            exit => {
                eprintln!("[pawnio] удаление отказало: {exit:?}");
                pawnio.lock().failure = Some(Problem { code: Code::Uninstaller, exit });
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_device_decides_and_the_registry_explains() {
        assert_eq!(decide(true, Some("2.2.0.0")), Status::Ready);
        // Устройство открылось, а записи нет — драйвер поставили без
        // установщика; читать он всё равно даёт
        assert_eq!(decide(true, None), Status::Ready);
        assert_eq!(decide(false, Some("2.2.0.0")), Status::Restart);
        assert_eq!(decide(false, None), Status::Missing);
        assert_eq!(decide(true, Some("1.0.3.0")), Status::Outdated);
        assert_eq!(decide(false, Some("1.0")), Status::Outdated);
        // Нечитаемая версия — не повод считать драйвер устаревшим
        assert_eq!(decide(false, Some("beta")), Status::Restart);
    }

    #[test]
    fn the_uninstaller_is_taken_from_its_own_record_only() {
        assert_eq!(
            uninstaller(r#""C:\Program Files\PawnIO\uninstall.exe" -uninstall"#),
            Some(PathBuf::from(r"C:\Program Files\PawnIO\uninstall.exe"))
        );
        assert_eq!(uninstaller(r"C:\PawnIO\uninstall.exe -uninstall"), Some(PathBuf::from(r"C:\PawnIO\uninstall.exe")));
        assert_eq!(uninstaller(r#""C:\Windows\System32\cmd.exe" /c del"#), None);
        assert_eq!(uninstaller(r#""uninstall.exe" -uninstall"#), None);
        assert_eq!(uninstaller(""), None);
    }

    #[test]
    fn a_view_arrives_in_camel_case() {
        let view = View {
            status: Status::Restart,
            version: Some("2.2.0.0".to_owned()),
            ours: true,
            job: Some(Job::Downloading { received: 1, total: Some(2) }),
            failure: Some(Problem { code: Code::Installer, exit: Some(5) }),
            download_bytes: DOWNLOAD_BYTES,
            page: PAGE,
        };
        let json = serde_json::to_string(&view).unwrap();
        for key in ["\"restart\"", "\"downloadBytes\"", "\"stage\":\"downloading\"", "\"installer\"", "\"exit\":5"] {
            assert!(json.contains(key), "{key} нет в {json}");
        }
    }

    #[test]
    fn a_download_failure_keeps_its_meaning() {
        assert_eq!(Problem::from(Download::BadChecksum).code, Code::BadChecksum);
        assert_eq!(Problem::from(Download::Unpack).code, Code::Disk);
    }
}
