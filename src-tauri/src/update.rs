//! Обновление самого Windgets — по согласию человека, как докачиваемые
//! программы.
//!
//! Свежий выпуск спрашиваем у GitHub тем же API, что у ffmpeg и PawnIO, без
//! новой зависимости и без своих ключей подписи (12 октября, слово
//! пользователя). Подлинность держат GitHub, HTTPS и SHA-256 из поля `digest`
//! самого API. Плагин updater с ключами отвергнут: ключ лежал бы в секретах
//! того же аккаунта, что собирает выпуск, и от угона аккаунта защищал бы
//! немногим больше, а потерянный ключ навсегда отрезал бы установленные копии
//! от обновлений. Следующий слой — проверка подписи SignPath у установщика,
//! когда она появится.
//!
//! ВАЖНО: **проверка — раз в неделю, а не на каждом запуске** (слово
//! пользователя). Машину включают каждый день, и проверка «при старте» была
//! бы ежедневной. Срок считается от метки в `settings.json`; «Проверить
//! сейчас» в «Общих» спрашивает в обход срока. Галочка снята — в сеть сами не
//! ходим вовсе.
//!
//! Ставит только нажатие: установщик качается, сверяется и запускается с
//! `/P /UPDATE /R` — без вопросов, поверх, с перезапуском приложения. Ключ
//! `/UPDATE` же оставляет задачу автозапуска (`windows/hooks.nsh`).

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::settings::{self, SettingsState};
use crate::tools::catalog::AssetRule;
use crate::tools::{fetch, github, Failure as Download};

const REPO: &str = "Common-Fool/WindGets";

/// Установщик выпуска: `Windgets_0.1.1_x64-setup.exe`.
const RULE: AssetRule = AssetRule::Around { prefix: "Windgets_", suffix: "_x64-setup.exe" };

/// Страница выпусков — для «Что нового».
const PAGE: &str = "https://github.com/Common-Fool/WindGets/releases/latest";

const EVENT: &str = "update";

/// Срок между автопроверками — неделя.
const EVERY: i64 = 7 * 24 * 60 * 60;

/// Первая проверка — не сразу после запуска: в первые минуты машина и так
/// занята входом в систему, а неделя от пары минут не пострадает.
const SETTLE: Duration = Duration::from_secs(120);

/// Как часто поток смотрит, не подошёл ли срок. Приложение живёт неделями, и
/// проверка только на старте у не выключаемой машины не случилась бы никогда.
const TICK: Duration = Duration::from_secs(6 * 60 * 60);

/// Ключи установщика Tauri: без страниц, поверх установленного, с запуском
/// приложения по окончании.
const INSTALLER_ARGS: [&str; 3] = ["/P", "/UPDATE", "/R"];

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
    /// Установщик не запустился.
    Launch,
    /// Сборка разработки: ставить поверх установленного ей нечего.
    Dev,
}

impl From<Download> for Code {
    fn from(failure: Download) -> Self {
        match failure {
            Download::NoBuild => Code::NoBuild,
            Download::NoChecksum => Code::NoChecksum,
            Download::BadChecksum => Code::BadChecksum,
            Download::Network => Code::Network,
            Download::RateLimited => Code::RateLimited,
            Download::Cancelled => Code::Cancelled,
            Download::Unpack | Download::Disk | Download::InUse => Code::Disk,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "stage", rename_all = "camelCase")]
pub enum Job {
    Checking,
    Downloading { received: u64, total: Option<u64> },
    /// Установщик запущен, приложение сейчас закроется.
    Installing,
}

/// Найденный выпуск новее нынешнего.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
    version: String,
    size: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    current: &'static str,
    /// Проверять раз в неделю самим.
    auto: bool,
    /// Когда спрашивали GitHub в последний раз, секунды Unix.
    checked_at: Option<i64>,
    available: Option<Offer>,
    job: Option<Job>,
    failure: Option<Code>,
    page: &'static str,
}

#[derive(Default)]
struct Inner {
    available: Option<Offer>,
    job: Option<Job>,
    failure: Option<Code>,
}

pub struct Updates {
    inner: Mutex<Inner>,
    busy: AtomicBool,
    cancel: AtomicBool,
}

impl Updates {
    pub fn new() -> Self {
        Self { inner: Mutex::new(Inner::default()), busy: AtomicBool::new(false), cancel: AtomicBool::new(false) }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn view(&self, app: &AppHandle) -> View {
        let settings = app.state::<SettingsState>().get();
        let inner = self.lock();
        View {
            current: env!("CARGO_PKG_VERSION"),
            auto: settings.update_auto,
            checked_at: settings.update_checked_at,
            available: inner.available.clone(),
            job: inner.job,
            failure: inner.failure,
            page: PAGE,
        }
    }

    fn set_job(&self, job: Option<Job>) {
        self.lock().job = job;
    }
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|since| since.as_secs() as i64).unwrap_or(0)
}

/// `v0.1.1` или `0.1.1` в числа. Хвост вроде `-beta` — не выпуск для всех:
/// такие `releases/latest` и не отдаёт, но разбирать их числом нельзя.
fn version(text: &str) -> Option<(u64, u64, u64)> {
    let text = text.trim().trim_start_matches('v');
    let mut parts = text.split('.');
    let mut next = || parts.next()?.parse::<u64>().ok();
    let parsed = (next()?, next()?, next()?);
    parts.next().is_none().then_some(parsed)
}

/// Новее ли выпуск нынешней версии. Неразборчивое — не новее: предлагать
/// непонятно что хуже, чем промолчать неделю.
fn newer(tag: &str, current: &str) -> bool {
    matches!((version(tag), version(current)), (Some(tag), Some(current)) if tag > current)
}

pub fn publish(app: &AppHandle) {
    if let Some(updates) = app.try_state::<Updates>() {
        let _ = app.emit(EVENT, updates.view(app));
    }
}

/// Опускает `busy` и снимает ход работы — и при панике тоже. Установщик, уже
/// запущенный, ход не снимает: приложение закроется само.
struct Busy(AppHandle);

impl Drop for Busy {
    fn drop(&mut self) {
        if let Some(updates) = self.0.try_state::<Updates>() {
            updates.busy.store(false, Ordering::SeqCst);
            let mut inner = updates.lock();
            if !matches!(inner.job, Some(Job::Installing)) {
                inner.job = None;
            }
        }
        publish(&self.0);
    }
}

fn occupy(app: &AppHandle) -> Option<Busy> {
    let updates = app.state::<Updates>();
    if updates.busy.swap(true, Ordering::SeqCst) {
        return None;
    }
    updates.cancel.store(false, Ordering::SeqCst);
    updates.lock().failure = None;
    Some(Busy(app.clone()))
}

/// Спрашивает GitHub и запоминает, есть ли что новее. Метку вопроса пишет и
/// при отказе: сеть, упавшая на неделе, — не повод спрашивать каждые шесть
/// часов. «Проверено» — только по ответу.
async fn check(app: &AppHandle) -> Result<(), Code> {
    let updates = app.state::<Updates>();
    updates.set_job(Some(Job::Checking));
    publish(app);

    let state = app.state::<SettingsState>();
    settings::update(app, &state, |settings| settings.update_asked_at = Some(now()));

    let client = crate::tools::client().map_err(Code::from)?;
    let release = latest(&client, &updates.cancel).await?;
    settings::update(app, &state, |settings| settings.update_checked_at = Some(now()));
    let offer = if newer(&release.tag_name, env!("CARGO_PKG_VERSION")) {
        let asset = github::pick_asset(&release, RULE)?;
        Some(Offer { version: release.tag_name.trim_start_matches('v').to_owned(), size: asset.size })
    } else {
        None
    };
    if let Some(offer) = &offer {
        eprintln!("[обновление] есть {}", offer.version);
    }
    updates.lock().available = offer;
    Ok(())
}

/// Свежий выпуск. Ответ 404 тут — не «сборки нет», а «выпуска не видно»:
/// закрытый репозиторий или ещё ни одного выпуска. Сеть, а не «нет установщика».
///
/// Кнопка «Отмена» стоит и на проверке, поэтому вопрос отменяемый: флаг
/// смотрится раз в десятую долю секунды, и брошенный запрос уходит вместе с
/// будущим, а не ждёт ответа сети до своего срока.
async fn latest(client: &reqwest::Client, cancel: &AtomicBool) -> Result<github::Release, Code> {
    let mut asked = std::pin::pin!(github::latest(client, REPO));
    loop {
        if let Ok(answer) = tokio::time::timeout(Duration::from_millis(100), &mut asked).await {
            return answer.map_err(|failure| match failure {
                Download::NoBuild => Code::Network,
                other => Code::from(other),
            });
        }
        if cancel.load(Ordering::SeqCst) {
            return Err(Code::Cancelled);
        }
    }
}

/// Скачивает установщик свежего выпуска, сверяет сумму и запускает его, а
/// приложение закрывает: заменить запущенный exe установщик не сможет.
async fn install(app: &AppHandle) -> Result<(), Code> {
    if cfg!(debug_assertions) {
        return Err(Code::Dev);
    }
    let updates = app.state::<Updates>();
    updates.set_job(Some(Job::Checking));
    publish(app);

    let client = crate::tools::client().map_err(Code::from)?;
    let release = latest(&client, &updates.cancel).await?;
    if !newer(&release.tag_name, env!("CARGO_PKG_VERSION")) {
        updates.lock().available = None;
        return Ok(());
    }
    let asset = github::pick_asset(&release, RULE)?;
    let digest = github::parse_digest(asset.digest.as_deref())?;

    let dir = folder(app).ok_or(Code::Disk)?;
    // Прежний недокачанный или уже поставленный установщик здесь не нужен
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|error| disk(&error))?;
    let partial = dir.join(format!("{}.download", asset.name));
    let setup = dir.join(&asset.name);

    updates.set_job(Some(Job::Downloading { received: 0, total: Some(asset.size) }));
    publish(app);
    fetch::download(&client, &asset.browser_download_url, digest, &partial, &updates.cancel, |received, total| {
        updates.set_job(Some(Job::Downloading { received, total: total.or(Some(asset.size)) }));
        publish(app);
    })
    .await?;
    std::fs::rename(&partial, &setup).map_err(|error| disk(&error))?;

    // Запускаем и не ждём: установщик сам дождётся, пока мы выйдем, и сам
    // поднимет новую версию. Права он наследует — приложение повышено.
    Command::new(&setup).args(INSTALLER_ARGS).spawn().map_err(|error| {
        eprintln!("[обновление] установщик не запустился: {error}");
        Code::Launch
    })?;
    eprintln!("[обновление] запущен установщик {}", release.tag_name);
    updates.set_job(Some(Job::Installing));
    publish(app);

    tokio::time::sleep(Duration::from_millis(500)).await;
    // ВАЖНО: **выходим штатной дорогой, а не голым `app.exit`.** Приложение
    // живёт в трее, и `ExitRequested` отменяется, пока не поднят признак
    // `Shutdown`: голый выход отменялся бы, а установщик упирался бы в
    // работающий процесс. `quit` поднимает признак и пишет места окон — а писать
    // их можно только из главного потока, отсюда `on_main`.
    crate::windows::on_main(app, |app| crate::windows::quit(&app));
    Ok(())
}

/// Папка установщика — рядом с докачанными программами. Удалением приложения
/// с «Удалить данные» уходит вместе с ними.
fn folder(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_local_data_dir().ok().map(|dir| dir.join("tools").join("update"))
}

fn disk(error: &std::io::Error) -> Code {
    eprintln!("[обновление] файловая операция не прошла: {error}");
    Code::Disk
}

/// Запускает поток автопроверки. Живёт до выхода и почти всё время спит.
pub fn start(app: &AppHandle) {
    // Установщик прошлого обновления своё дело сделал
    if let Some(dir) = folder(app) {
        let _ = std::fs::remove_dir_all(dir);
    }

    let app = app.clone();
    let spawned = std::thread::Builder::new().name("update-check".into()).spawn(move || {
        std::thread::sleep(SETTLE);
        loop {
            let settings = app.state::<SettingsState>().get();
            // Старый файл знает только удачную метку — она же и последний вопрос
            let asked = settings.update_asked_at.max(settings.update_checked_at);
            if settings.update_auto && github::check_due(asked, now(), EVERY) {
                run_check(&app);
            }
            std::thread::sleep(TICK);
        }
    });
    if let Err(error) = spawned {
        eprintln!("поток проверки обновлений не запустился: {error}");
    }
}

/// Одна проверка в своей задаче, если ничего другого сейчас не идёт.
fn run_check(app: &AppHandle) {
    let Some(busy) = occupy(app) else {
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _busy = busy;
        if let Err(code) = check(&app).await {
            app.state::<Updates>().lock().failure = Some(code);
        }
    });
}

#[tauri::command]
pub fn update_state(app: AppHandle, state: State<'_, Updates>) -> View {
    state.view(&app)
}

/// «Проверить сейчас» — в обход недельного срока.
#[tauri::command]
pub fn update_check(app: AppHandle) {
    run_check(&app);
}

/// «Обновить» — только по нажатию человека.
#[tauri::command]
pub fn update_install(app: AppHandle) -> Result<(), String> {
    let busy = occupy(&app).ok_or_else(|| "busy".to_owned())?;
    publish(&app);
    tauri::async_runtime::spawn(async move {
        let _busy = busy;
        if let Err(code) = install(&app).await {
            app.state::<Updates>().lock().failure = Some(code);
        }
    });
    Ok(())
}

/// Отменяет скачивание. Запущенный установщик уже не наш.
#[tauri::command]
pub fn update_cancel(state: State<'_, Updates>) {
    state.cancel.store(true, Ordering::SeqCst);
}

/// Галочка «Проверять раз в неделю».
#[tauri::command]
pub fn update_set_auto(app: AppHandle, auto: bool) {
    let state = app.state::<SettingsState>();
    settings::update(&app, &state, |settings| settings.update_auto = auto);
    publish(&app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_newer_release_is_offered() {
        assert!(newer("v0.1.1", "0.1.0"));
        assert!(newer("0.2.0", "0.1.9"));
        assert!(newer("v1.0.0", "0.99.99"));
        // Десятки сравниваются числом, а не строкой
        assert!(newer("v0.10.0", "0.9.0"));
        assert!(!newer("v0.1.0", "0.1.0"));
        assert!(!newer("v0.0.9", "0.1.0"));
        // Неразборчивое не предлагаем
        assert!(!newer("v0.2.0-beta", "0.1.0"));
        assert!(!newer("latest", "0.1.0"));
        assert!(!newer("v0.2", "0.1.0"));
    }

    #[test]
    fn the_installer_is_recognised_by_its_name() {
        assert!(RULE.matches("Windgets_0.1.1_x64-setup.exe"));
        assert!(RULE.matches("Windgets_12.0.3_x64-setup.exe"));
        assert!(!RULE.matches("Windgets_0.1.1_x64_en-US.msi"));
        assert!(!RULE.matches("windgets-sensors.exe"));
    }

    #[test]
    fn the_own_version_parses() {
        assert!(version(env!("CARGO_PKG_VERSION")).is_some(), "версия в Cargo.toml не вида X.Y.Z");
    }

    #[test]
    fn a_week_passes_between_checks() {
        let start = 1_700_000_000;
        assert!(github::check_due(None, start, EVERY));
        assert!(!github::check_due(Some(start), start + EVERY - 1, EVERY));
        assert!(github::check_due(Some(start), start + EVERY, EVERY));
    }

    #[test]
    fn a_view_arrives_in_camel_case() {
        let view = View {
            current: "0.1.0",
            auto: true,
            checked_at: Some(1),
            available: Some(Offer { version: "0.1.1".to_owned(), size: 5 }),
            job: Some(Job::Downloading { received: 1, total: Some(2) }),
            failure: Some(Code::RateLimited),
            page: PAGE,
        };
        let json = serde_json::to_string(&view).unwrap();
        for key in ["\"checkedAt\":1", "\"stage\":\"downloading\"", "\"rateLimited\"", "\"version\":\"0.1.1\""] {
            assert!(json.contains(key), "{key} нет в {json}");
        }
    }
}
