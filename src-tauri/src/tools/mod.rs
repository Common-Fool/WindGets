//! Докачиваемые программы конвертера и загрузчика: ffmpeg, yt-dlp, QuickJS.
//!
//! ВАЖНО: **в установщик они не кладутся — принцип пользователя.** Минимум
//! ненужного, ни к чему не привязываться, что можно докачать — докачивать.
//! Приложение скачивает программы само, только по нажатию человека, с
//! официальных выпусков на GitHub, сверяет SHA-256 и само же их обновляет:
//! выпускать приложение ради свежего yt-dlp не нужно.
//!
//! До нажатия кнопки в сеть не ходим вовсе — ровно как погода без города.
//! Размеры для текста согласия берутся из каталога, а не у GitHub.
//!
//! Уже стоящий в системе ffmpeg берётся как есть, если в нём есть нужные
//! кодировщики. Свой скачанный — главнее: он известной сборки.
//!
//! Программы лежат в `app_local_data_dir()/tools/<программа>/`: это данные этой
//! машины, а не настройки, и в перемещаемый профиль им незачем.

mod archive;
pub mod catalog;
pub(crate) mod fetch;
pub(crate) mod github;
mod store;

pub use catalog::{Component, Module};

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use catalog::{spec, Install};

const TOOLS_EVENT: &str = "tools";

/// Как часто спрашиваем GitHub о свежем yt-dlp перед загрузкой. Он ломается
/// вслед за YouTube, но не каждый час; раз в сутки — и после любой поломки
/// на стороне сайта сразу (см. загрузчик).
pub const YTDLP_CHECK_EVERY: i64 = 86_400;

/// Неудачную проверку без кнопки повторяем не раньше чем через час.
const RETRY_AFTER_FAILURE: i64 = 60 * 60;

/// Почему программа не встала. Код, а не текст: карточка говорит по-английски,
/// настройки — по-русски, и каждая сторона переводит его сама.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Failure {
    /// Нужного файла в выпуске нет — проект переименовал сборку.
    NoBuild,
    /// GitHub не дал суммы файла. Без неё не ставим.
    NoChecksum,
    BadChecksum,
    Network,
    RateLimited,
    Unpack,
    Disk,
    Cancelled,
    /// Удалить нельзя: программа сейчас работает — идёт конвертация,
    /// загрузка или проба. Работающий exe Windows не удаляет.
    InUse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Presence {
    Missing,
    /// Скачана приложением.
    Installed,
    /// Взята из системы как есть.
    System,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentView {
    id: Component,
    name: &'static str,
    presence: Presence,
    version: Option<String>,
    download_bytes: u64,
    disk_bytes: u64,
    checked_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleView {
    id: Module,
    /// Что модулю нужно вообще. Фронт прежде держал копию этого списка у себя.
    needs: &'static [Component],
    /// Пусто — модуль готов к работе.
    missing: Vec<Component>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    Checking,
    Downloading,
    Unpacking,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    component: Component,
    stage: Stage,
    received: u64,
    total: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    component: Component,
    code: Failure,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NoticeCode {
    UpToDate,
    Updated,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    component: Component,
    code: NoticeCode,
    version: String,
}

/// Всё, что видят карточки и настройки.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    components: Vec<ComponentView>,
    modules: Vec<ModuleView>,
    /// Идущая установка или проверка.
    job: Option<Job>,
    failure: Option<Problem>,
    notice: Option<Notice>,
    /// Системный ffmpeg ещё ищется: до конца поиска «нет ffmpeg» — не ответ.
    probing: bool,
}

struct Inner {
    stored: store::Stored,
    job: Option<Job>,
    failure: Option<Problem>,
    notice: Option<Notice>,
    /// Годный ffmpeg из PATH, если нашёлся.
    system: Option<PathBuf>,
    probing: bool,
    /// Когда в последний раз спрашивали GitHub без кнопки, удачно или нет.
    /// Только в памяти: отказ не меняет `checked_at`, и без этой метки
    /// недоступный GitHub спрашивали бы перед каждой загрузкой.
    tried: std::collections::BTreeMap<Component, i64>,
}

pub struct Tools {
    /// `None` — каталог данных не нашёлся; тогда и ставить некуда.
    root: Option<PathBuf>,
    inner: Mutex<Inner>,
    /// Идёт установка или проверка. Одна за раз: две параллельные делили бы
    /// один файл выпуска и одну полосу прогресса.
    busy: AtomicBool,
    cancel: AtomicBool,
    /// Поиск системного ffmpeg уже запущен. Идёт он не при старте, а по
    /// первому вопросу о программах: выключенным конвертеру и загрузчику
    /// ffmpeg искать незачем.
    probed: AtomicBool,
}

impl Tools {
    /// Просит идущую установку или обновление остановиться. Флаг читает
    /// скачивание на каждом куске; следующая работа снимает его сама.
    pub fn stop(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    pub fn new(app: &AppHandle) -> Self {
        let root = app.path().app_local_data_dir().ok().map(|dir| dir.join("tools"));
        let stored = root.as_deref().map(store::load).unwrap_or_default();

        Self {
            root,
            inner: Mutex::new(Inner {
                stored,
                job: None,
                failure: None,
                notice: None,
                system: None,
                probing: true,
                tried: Default::default(),
            }),
            busy: AtomicBool::new(false),
            cancel: AtomicBool::new(false),
            probed: AtomicBool::new(false),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // Отравленный мьютекс не повод падать: данные внутри осмысленные
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn own(&self, component: Component) -> Option<PathBuf> {
        let spec = spec(component);
        self.root.as_ref().map(|root| root.join(spec.dir).join(spec.file))
    }

    /// Где программа: своя, иначе системная, иначе нигде.
    pub fn locate(&self, component: Component) -> Option<PathBuf> {
        if let Some(own) = self.own(component).filter(|path| path.is_file()) {
            return Some(own);
        }
        // Путь может вести на уснувший сетевой ресурс. Синхронные команды
        // берут снимок поиска; отказ запуска обнаружит рабочий поток
        if spec(component).system.is_some() {
            return self.lock().system.clone();
        }
        None
    }

    fn presence(&self, component: Component) -> Presence {
        if self.own(component).is_some_and(|path| path.is_file()) {
            Presence::Installed
        } else if spec(component).system.is_some() && self.lock().system.is_some() {
            Presence::System
        } else {
            Presence::Missing
        }
    }

    pub fn missing(&self, module: Module) -> Vec<Component> {
        module
            .needs()
            .iter()
            .copied()
            .filter(|component| self.locate(*component).is_none())
            .collect()
    }

    pub fn ready(&self, module: Module) -> bool {
        self.missing(module).is_empty()
    }

    /// Пора ли спросить GitHub о свежей версии своей программы.
    pub fn update_due(&self, component: Component) -> bool {
        let own = self.own(component).is_some_and(|path| path.is_file());
        let inner = self.lock();
        let tried = inner.tried.get(&component).is_some_and(|at| now() - at < RETRY_AFTER_FAILURE);
        own && !tried && github::check_due(inner.stored.entry(component).checked_at, now(), YTDLP_CHECK_EVERY)
    }

    /// Рабочая папка загрузчика: туда yt-dlp качает, оттуда файл переезжает
    /// в папку вывода.
    pub fn work_dir(&self) -> Option<PathBuf> {
        self.root.as_ref().map(|root| root.join("work"))
    }

    /// Кеш yt-dlp — рядом с ним самим, а не в профиле человека.
    pub fn cache_dir(&self, component: Component) -> Option<PathBuf> {
        self.root.as_ref().map(|root| root.join(spec(component).dir).join("cache"))
    }

    pub fn view(&self) -> View {
        let components = Component::ALL
            .into_iter()
            .map(|component| {
                let spec = spec(component);
                let presence = self.presence(component);
                let entry = self.lock().stored.entry(component);
                ComponentView {
                    id: component,
                    name: spec.name,
                    presence,
                    version: (presence == Presence::Installed).then_some(entry.tag).flatten(),
                    download_bytes: spec.download_bytes,
                    disk_bytes: spec.disk_bytes,
                    checked_at: entry.checked_at,
                }
            })
            .collect();

        let modules = Module::ALL
            .into_iter()
            .map(|module| ModuleView { id: module, needs: module.needs(), missing: self.missing(module) })
            .collect();

        let inner = self.lock();
        View {
            components,
            modules,
            job: inner.job,
            failure: inner.failure,
            notice: inner.notice.clone(),
            probing: inner.probing,
        }
    }

    fn set_job(&self, job: Option<Job>) {
        self.lock().job = job;
    }
}

pub fn publish(app: &AppHandle) {
    if let Some(tools) = app.try_state::<Tools>() {
        let _ = app.emit(TOOLS_EVENT, tools.view());
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or(0)
}

/// Поднятая `busy`, опускающаяся сама — и при панике в задаче тоже. Тот же
/// приём, что у спидтеста: строка после `await` при панике не исполнилась бы,
/// и кнопка осталась бы запертой до перезапуска.
struct Busy(AppHandle);

impl Drop for Busy {
    fn drop(&mut self) {
        if let Some(tools) = self.0.try_state::<Tools>() {
            // Сперва ход работы, потом место: в обратном порядке следующая
            // работа успевала занять место и выставить свой ход, а этот `drop`
            // его стирал — и кнопка отмены пропадала посреди скачивания
            tools.set_job(None);
            tools.busy.store(false, Ordering::SeqCst);
        }
        publish(&self.0);
    }
}

/// Занимает единственное место установки. `None` — оно занято.
fn occupy(app: &AppHandle) -> Option<Busy> {
    let tools = app.state::<Tools>();
    if tools.busy.swap(true, Ordering::SeqCst) {
        return None;
    }
    tools.cancel.store(false, Ordering::SeqCst);
    {
        let mut inner = tools.lock();
        inner.failure = None;
        inner.notice = None;
    }
    Some(Busy(app.clone()))
}

/// Клиент для GitHub.
///
/// ВАЖНО: **`User-Agent` обязателен** — без него API GitHub отвечает 403. Общего
/// таймаута нет намеренно: 35 МБ на медленном канале идут минутами, и срок на
/// весь запрос обрывал бы живое скачивание. Вместо него сроки на соединение и
/// на молчание между кусками.
pub(crate) fn client() -> Result<reqwest::Client, Failure> {
    reqwest::Client::builder()
        .user_agent(concat!("windgets/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| {
            eprintln!("[компоненты] клиент не собрался: {error}");
            Failure::Network
        })
}

/// Ставит программу из свежего выпуска.
async fn install(app: &AppHandle, component: Component) -> Result<(), Failure> {
    let tools = app.state::<Tools>();
    tools.set_job(Some(Job { component, stage: Stage::Checking, received: 0, total: None }));
    publish(app);

    let client = client()?;
    let release = latest(&tools.cancel, &client, spec(component).repo).await?;
    install_release(app, component, &client, &release).await
}

async fn install_release(
    app: &AppHandle,
    component: Component,
    client: &reqwest::Client,
    release: &github::Release,
) -> Result<(), Failure> {
    let tools = app.state::<Tools>();
    let spec = spec(component);
    let root = tools.root.clone().ok_or(Failure::Disk)?;

    let asset = github::pick_asset(release, spec.asset)?;
    let digest = github::parse_digest(asset.digest.as_deref())?;

    let dir = root.join(spec.dir);
    std::fs::create_dir_all(&dir).map_err(disk)?;
    let archive = dir.join(format!("{}.download", asset.name));

    tools.set_job(Some(Job {
        component,
        stage: Stage::Downloading,
        received: 0,
        total: Some(asset.size),
    }));
    publish(app);

    fetch::download(client, &asset.browser_download_url, digest, &archive, &tools.cancel, |received, total| {
        tools.set_job(Some(Job {
            component,
            stage: Stage::Downloading,
            received,
            total: total.or(Some(asset.size)),
        }));
        publish(app);
    })
    .await?;

    let fresh = dir.join(format!("{}.new", spec.file));
    match spec.install {
        Install::Exe => std::fs::rename(&archive, &fresh).map_err(disk)?,
        Install::Archive { member } => {
            tools.set_job(Some(Job { component, stage: Stage::Unpacking, received: 0, total: None }));
            publish(app);

            let unpack_dir = dir.join("unpack");
            let _ = std::fs::remove_dir_all(&unpack_dir);
            let (source, into) = (archive.clone(), unpack_dir.clone());
            // tar ждёт своего конца — не в потоке асинхронного рантайма
            let unpacked = tauri::async_runtime::spawn_blocking(move || {
                archive::unpack(&source, member, &into)
            })
            .await
            .map_err(|_| Failure::Unpack)?;

            let outcome = unpacked.and_then(|path| std::fs::rename(path, &fresh).map_err(disk));
            let _ = std::fs::remove_dir_all(&unpack_dir);
            let _ = std::fs::remove_file(&archive);
            outcome?;
        }
    }

    // Отмену могли нажать и на распаковке: флаг проверяет только скачивание,
    // и без этой строки отменённая программа всё равно встала бы на место
    if tools.cancel.load(Ordering::SeqCst) {
        let _ = std::fs::remove_file(dir.join(format!("{}.new", spec.file)));
        return Err(Failure::Cancelled);
    }

    swap(&dir, spec.file)?;

    let at = now();
    {
        let mut inner = tools.lock();
        inner.stored.components.insert(
            component,
            store::Entry {
                tag: Some(release.tag_name.clone()),
                installed_at: Some(at),
                checked_at: Some(at),
            },
        );
        store::save(&root, &inner.stored);
    }
    Ok(())
}

/// Ставит новую программу на место старой.
///
/// Работающий exe Windows удалить не даст, а переименовать даст. Поэтому
/// старая уходит в `.old`, новая встаёт на её имя, а `.old` убирается сразу
/// или — если её держит работающая загрузка — при следующем запуске
/// приложения (см. [`spawn`]).
fn swap(dir: &Path, file: &str) -> Result<(), Failure> {
    let target = dir.join(file);
    let fresh = dir.join(format!("{file}.new"));
    let old = dir.join(format!("{file}.old"));

    let _ = std::fs::remove_file(&old);
    // Прежний `.old` ещё держит работающая загрузка (второе обновление за
    // одну её жизнь) — старой версии нужно другое имя, а не отказ записи при
    // целой программе. Все они уберутся при следующем запуске
    let old = (1..100)
        .map(|number| match number {
            1 => old.clone(),
            _ => dir.join(format!("{file}.old{number}")),
        })
        .find(|candidate| !candidate.exists())
        .unwrap_or(old);
    if target.exists() {
        std::fs::rename(&target, &old).map_err(disk)?;
    }
    if let Err(error) = std::fs::rename(&fresh, &target) {
        // Вернуть старую на место: лучше прежняя версия, чем никакой
        let _ = std::fs::rename(&old, &target);
        return Err(disk(error));
    }
    let _ = std::fs::remove_file(&old);
    Ok(())
}

fn disk(error: std::io::Error) -> Failure {
    eprintln!("[компоненты] файловая операция не прошла: {error}");
    Failure::Disk
}

/// Проверяет свежий выпуск и ставит его, если он новее стоящего.
///
/// `force` — по кнопке: спрашиваем GitHub сразу. Без него — не чаще
/// [`YTDLP_CHECK_EVERY`]. Отдаёт, поставилось ли новое.
///
/// Системную программу не трогает: её обновляет тот, кто её поставил.
pub async fn refresh(app: &AppHandle, component: Component, force: bool) -> Result<bool, Failure> {
    let tools = app.state::<Tools>();
    let own = tools.own(component).is_some_and(|path| path.is_file());
    if !own {
        return Ok(false);
    }

    let entry = tools.lock().stored.entry(component);
    if !force && !github::check_due(entry.checked_at, now(), YTDLP_CHECK_EVERY) {
        return Ok(false);
    }

    if !force {
        tools.lock().tried.insert(component, now());
    }
    tools.set_job(Some(Job { component, stage: Stage::Checking, received: 0, total: None }));
    publish(app);

    let client = client()?;
    let release = latest(&tools.cancel, &client, spec(component).repo).await?;

    if entry.tag.as_deref() == Some(release.tag_name.as_str()) {
        let root = tools.root.clone();
        let mut inner = tools.lock();
        inner.stored.components.entry(component).or_default().checked_at = Some(now());
        if let Some(root) = root {
            store::save(&root, &inner.stored);
        }
        inner.notice = Some(Notice {
            component,
            code: NoticeCode::UpToDate,
            version: release.tag_name,
        });
        return Ok(false);
    }

    install_release(app, component, &client, &release).await?;
    tools.lock().notice = Some(Notice {
        component,
        code: NoticeCode::Updated,
        version: release.tag_name,
    });
    Ok(true)
}

/// Свежий выпуск с GitHub, но с отменой, слышной и на самом вопросе, а не
/// только на скачивании: без ответа GitHub ждёт до сорока пяти секунд, и
/// «Cancel» загрузчика или установки ждал бы их целиком. Общий у программ и
/// драйвера PawnIO.
pub(crate) async fn latest(cancel: &AtomicBool, client: &reqwest::Client, repo: &str) -> Result<github::Release, Failure> {
    let mut asked = std::pin::pin!(github::latest(client, repo));
    loop {
        if let Ok(answer) = tokio::time::timeout(Duration::from_millis(100), &mut asked).await {
            return answer;
        }
        if cancel.load(Ordering::SeqCst) {
            return Err(Failure::Cancelled);
        }
    }
}

/// Проверка свежего yt-dlp перед загрузкой — из рабочего потока загрузчика.
///
/// Место установки занято (человек как раз ставит программы) — проверку
/// пропускаем: она несрочная, а ждать чужую установку загрузке незачем.
///
/// `stop` — отмена самого задания. Её смотрим уже после `occupy`: тот снимает
/// флаг установки, и «Cancel», нажатый в миг между «обновляется» и занятием
/// места, иначе пропадал бы, а отменённое задание качало бы yt-dlp до конца.
pub fn refresh_blocking(app: &AppHandle, component: Component, force: bool, stop: &AtomicBool) -> Result<bool, Failure> {
    let Some(_busy) = occupy(app) else {
        return Ok(false);
    };
    if stop.load(Ordering::SeqCst) {
        return Err(Failure::Cancelled);
    }
    let outcome = tauri::async_runtime::block_on(refresh(app, component, force));
    if let Err(code) = outcome {
        app.state::<Tools>().lock().failure = Some(Problem { component, code });
    }
    outcome
}

/// Убирает хвосты прерванных установок. Своим потоком: это диск, а зовут это
/// из `setup`. Всегда, даже при выключенных конвертере и загрузчике: хвосты
/// занимают место независимо от того, включён ли модуль.
///
/// ВАЖНО: **до запуска модулей, а не рядом с ними.** Своим потоком уборка
/// гонялась с первым заданием: загрузчик успевал завести `tools/work/<id>`, а
/// она удаляла его вместе с готовым файлом. Работа короткая — несколько
/// недокачанных файлов и папка распаковки.
pub fn clean_leftovers(app: &AppHandle) {
    if let Some(root) = &app.state::<Tools>().root {
        clean(root);
    }
}

/// Ищет системный ffmpeg — один раз, по первому вопросу о программах.
///
/// Не при старте: этот вопрос задают только карточки конвертера и загрузчика и
/// их страницы настроек. Выключенным модулям искать ffmpeg незачем, а поиск —
/// это запуск процесса. До конца поиска вид честно говорит «ищем».
fn ensure_probed(app: &AppHandle) {
    let tools = app.state::<Tools>();
    if tools.probed.swap(true, Ordering::SeqCst) {
        return;
    }

    let app = app.clone();
    std::thread::spawn(move || {
        let system = system_ffmpeg();
        {
            let tools = app.state::<Tools>();
            let mut inner = tools.lock();
            inner.system = system;
            inner.probing = false;
        }
        publish(&app);
    });
}

/// Готовая загрузка, которую не удалось перенести, не является мусором.
pub(crate) const KEEP_WORK: &str = ".windgets-keep";

/// Сколько держать такую загрузку, прежде чем уборка при запуске её снимет.
const KEEP_WORK_FOR: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Хвосты прерванных установок и загрузок: недокачанные файлы, недоставленные
/// замены, старые версии, которые держала работающая программа, и рабочая
/// папка загрузчика.
fn clean(root: &Path) {
    for spec in &catalog::SPECS {
        let dir = root.join(spec.dir);
        let _ = std::fs::remove_dir_all(dir.join("unpack"));
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let old = name.rsplit_once(".old").is_some_and(|(_, number)| number.chars().all(|c| c.is_ascii_digit()));
            if name.ends_with(".download") || name.ends_with(".new") || old {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir(root.join("work")) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Недоставленная загрузка ждёт человека неделю, а не вечно:
                // путь к ней сказан в карточке, а гигабайты в папке программ
                // копились бы молча
                let kept = std::fs::metadata(path.join(KEEP_WORK))
                    .and_then(|marker| marker.modified())
                    .ok()
                    .and_then(|at| at.elapsed().ok())
                    .is_some_and(|age| age < KEEP_WORK_FOR);
                if !kept {
                    let _ = std::fs::remove_dir_all(path);
                }
            } else {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

/// ffmpeg из PATH, если он есть и умеет то, что нужно.
///
/// Урезанные сборки без x264 (LGPL) и без LAME бывают — с такой конвертер
/// падал бы на каждом MP4 и MP3. Её считаем отсутствующей и предлагаем свою.
///
/// ВАЖНО: **приложение повышено, и ffmpeg из PATH запустится от
/// администратора.** Папки из PATH бывают открыты на запись обычному
/// пользователю — та же поверхность, что и у своих программ в
/// `%LOCALAPPDATA%`. Для машины на одного человека это принято сознательно;
/// уйдёт вместе с правами администратора, когда датчики переедут в службу.
pub(crate) fn system_ffmpeg() -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    let found = find_in_path("ffmpeg.exe", &path_var, |path| path.is_file())?;

    let mut command = Command::new(&found);
    command.args(["-hide_banner", "-encoders"]);
    let (_, lines) = crate::child::run(command, Duration::from_secs(10)).ok()?;
    let texts: Vec<&str> = lines.iter().map(|line| line.text()).collect();

    if has_encoders(&texts) {
        Some(found)
    } else {
        eprintln!("[компоненты] ffmpeg из PATH без x264 или LAME: {}", found.display());
        None
    }
}

/// Первая папка PATH, в которой лежит `name`.
fn find_in_path(name: &str, path_var: &OsStr, is_file: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    std::env::split_paths(path_var)
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(name))
        .find(|path| is_file(path))
}

/// Есть ли в списке кодировщиков те, без которых конвертеру делать нечего.
fn has_encoders(lines: &[&str]) -> bool {
    let has = |name: &str| lines.iter().any(|line| line.split_whitespace().nth(1) == Some(name));
    has("libx264") && has("libmp3lame")
}

#[tauri::command]
pub fn tools_state(app: AppHandle, state: State<'_, Tools>) -> View {
    ensure_probed(&app);
    state.view()
}

/// Удаляет скачанные программы — кнопкой в настройках модуля или разом, когда
/// модуль выключили и программы ему больше не нужны. Списком, а не по одной:
/// место установки одно, и вторая команда подряд упёрлась бы в занятое.
///
/// Только свои: системный ffmpeg ставил человек, и обновляет и убирает его
/// тоже он. Своя лежит в `tools\<папка>`, и удаляется папка целиком — вместе с
/// кешем yt-dlp.
///
/// Работающую программу Windows удалить не даст, поэтому сперва спрашиваем
/// модули, не заняты ли они ей: отказ заранее понятнее, чем полуудалённая
/// папка. Проба и превью конвертера коротки и флага не имеют — их ловит сам
/// отказ удаления, тем же кодом `inUse`.
#[tauri::command]
pub fn tools_remove(app: AppHandle, components: Vec<Component>) -> Result<(), String> {
    let mut own: Vec<Component> = Vec::new();
    for component in components {
        if app.state::<Tools>().presence(component) == Presence::Installed && !own.contains(&component) {
            own.push(component);
        }
    }
    let Some(&first) = own.first() else {
        return Err("notInstalled".to_owned());
    };

    let busy = occupy(&app).ok_or_else(|| "busy".to_owned())?;

    if crate::converter::uses_tools(&app) || crate::downloader::uses_tools(&app) || crate::fps::uses_tools() {
        app.state::<Tools>().lock().failure = Some(Problem { component: first, code: Failure::InUse });
        // Опустившийся `busy` разошлёт отказ сам
        return Err("inUse".to_owned());
    }

    // Диск — своим потоком: синхронная команда выполняется в главном
    std::thread::spawn(move || {
        let _busy = busy;
        let tools = app.state::<Tools>();
        let Some(root) = tools.root.clone() else {
            return;
        };

        for component in own {
            match remove(&root, component) {
                Ok(()) => {
                    let mut inner = tools.lock();
                    inner.stored.components.remove(&component);
                    store::save(&root, &inner.stored);
                }
                Err(code) => {
                    tools.lock().failure = Some(Problem { component, code });
                    break;
                }
            }
        }
    });

    Ok(())
}

/// Папка программы внутри корня — только простое имя, без разделителей и
/// переходов наверх. Имена берутся из каталога, но удаление обязано
/// проверять само: `remove_dir_all` с кривым путём снёс бы чужое.
fn plain_folder(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':'])
}

/// Стирает папку программы. Сперва сам exe: работающий Windows удалить не
/// даст, и тогда папка остаётся целой, а не наполовину.
fn remove(root: &Path, component: Component) -> Result<(), Failure> {
    let spec = spec(component);
    if !plain_folder(spec.dir) {
        return Err(Failure::Disk);
    }
    let dir = root.join(spec.dir);

    match std::fs::remove_file(dir.join(spec.file)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        // Отказ в доступе (5) и занятость файла (32) — программа работает
        Err(error) if matches!(error.raw_os_error(), Some(5 | 32)) => return Err(Failure::InUse),
        Err(error) => {
            eprintln!("[компоненты] {} не удалился: {error}", spec.name);
            return Err(Failure::Disk);
        }
    }

    // Остальное — кеш, хвосты обновления. Не удалилось — не беда: exe уже нет,
    // программа считается отсутствующей, а хвосты уберёт следующий запуск
    if let Err(error) = std::fs::remove_dir_all(&dir) {
        if error.kind() != std::io::ErrorKind::NotFound {
            eprintln!("[компоненты] папка {} удалилась не целиком: {error}", dir.display());
        }
    }
    Ok(())
}

/// Ставит всё, чего не хватает модулю. Только по нажатию человека.
#[tauri::command]
pub fn tools_install(app: AppHandle, module: Module) -> Result<(), String> {
    let busy = occupy(&app).ok_or_else(|| "busy".to_owned())?;
    publish(&app);

    tauri::async_runtime::spawn(async move {
        let _busy = busy;
        let missing = app.state::<Tools>().missing(module);
        for component in missing {
            if let Err(code) = install(&app, component).await {
                app.state::<Tools>().lock().failure = Some(Problem { component, code });
                break;
            }
            publish(&app);
        }
    });

    Ok(())
}

/// Проверка обновления по кнопке из настроек.
#[tauri::command]
pub fn tools_update(app: AppHandle, component: Component) -> Result<(), String> {
    let busy = occupy(&app).ok_or_else(|| "busy".to_owned())?;
    publish(&app);

    tauri::async_runtime::spawn(async move {
        let _busy = busy;
        if let Err(code) = refresh(&app, component, true).await {
            app.state::<Tools>().lock().failure = Some(Problem { component, code });
        }
    });

    Ok(())
}

#[tauri::command]
pub fn tools_cancel(state: State<'_, Tools>) {
    state.stop();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Системный путь уже проверил рабочий поток поиска. Вопрос о готовности
    /// не должен снова трогать файл, даже если он уже исчез или недоступен.
    #[test]
    fn system_ffmpeg_is_read_from_the_probe_snapshot() {
        let path = PathBuf::from("C:\\missing-tools\\ffmpeg.exe");
        let tools = Tools {
            root: None,
            inner: Mutex::new(Inner {
                stored: Default::default(),
                job: None,
                failure: None,
                notice: None,
                system: Some(path.clone()),
                probing: false,
                tried: Default::default(),
            }),
            busy: AtomicBool::new(false),
            cancel: AtomicBool::new(false),
            probed: AtomicBool::new(true),
        };
        assert_eq!(tools.locate(Component::Ffmpeg), Some(path));
        assert_eq!(tools.presence(Component::Ffmpeg), Presence::System);
        assert!(tools.ready(Module::Converter));
        assert_eq!(tools.view().components.iter().find(|item| item.id == Component::Ffmpeg).unwrap().presence, Presence::System);
        assert_eq!(tools.locate(Component::YtDlp), None);
    }

    #[test]
    fn startup_cleanup_keeps_a_download_that_failed_to_move() {
        let root = std::env::temp_dir().join(format!("hw-tools-keep-{}", std::process::id()));
        let kept = root.join("work").join("0");
        let partial = root.join("work").join("1");
        std::fs::create_dir_all(&kept).unwrap();
        std::fs::create_dir_all(&partial).unwrap();
        std::fs::write(kept.join(KEEP_WORK), b"").unwrap();
        std::fs::write(kept.join("clip.mp4"), b"downloaded").unwrap();
        std::fs::write(partial.join("clip.part"), b"partial").unwrap();
        clean(&root);
        assert_eq!(std::fs::read(kept.join("clip.mp4")).unwrap(), b"downloaded");
        assert!(!partial.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_program_is_found_in_the_first_folder_that_has_it() {
        let path_var = std::env::join_paths(["C:\\one", "", "C:\\two", "C:\\three"]).unwrap();
        let found = find_in_path("ffmpeg.exe", &path_var, |path| {
            path.starts_with("C:\\two") || path.starts_with("C:\\three")
        });
        assert_eq!(found, Some(PathBuf::from("C:\\two\\ffmpeg.exe")));
        assert_eq!(find_in_path("ffmpeg.exe", &path_var, |_| false), None);
    }

    /// Строки `-encoders` — как у настоящей сборки; имя кодировщика стоит
    /// вторым словом, а не где попало (слово `libx264` в описании не в счёт).
    #[test]
    fn encoders_are_read_from_the_second_column() {
        let full = [
            " V....D libx264              libx264 H.264 / AVC",
            " A....D libmp3lame           libmp3lame MP3 (MPEG audio layer 3)",
        ];
        assert!(has_encoders(&full));

        let lgpl = [
            " V....D h264_nvenc           NVIDIA NVENC H.264 encoder (codec h264, libx264-like)",
            " A....D libmp3lame           libmp3lame MP3",
        ];
        assert!(!has_encoders(&lgpl));
    }

    /// Живая проверка сети: свежий выпуск QuickJS с GitHub тем же клиентом, со
    /// сверкой суммы, — и отказ с заведомо неверной суммой, после которого
    /// недокачанного файла не остаётся.
    #[tokio::test]
    #[ignore = "ходит в GitHub: cargo test -- --ignored quickjs_downloads"]
    async fn quickjs_downloads_and_verifies() {
        let spec = spec(Component::QuickJs);
        let client = client().unwrap();
        let release = github::latest(&client, spec.repo).await.expect("GitHub ответил");
        let asset = github::pick_asset(&release, spec.asset).expect("сборка нашлась");
        let digest = github::parse_digest(asset.digest.as_deref()).expect("сумма есть");

        let dir = std::env::temp_dir().join(format!("hw-tools-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("qjs.exe");
        let cancel = AtomicBool::new(false);

        let mut calls = 0;
        fetch::download(&client, &asset.browser_download_url, digest, &dest, &cancel, |_, _| calls += 1)
            .await
            .expect("скачалось и сошлось");
        assert_eq!(dest.metadata().unwrap().len(), asset.size);
        assert!(calls >= 2, "прогресс хотя бы в начале и в конце");

        let wrong = fetch::download(&client, &asset.browser_download_url, [0; 32], &dest, &cancel, |_, _| {}).await;
        assert_eq!(wrong, Err(Failure::BadChecksum));
        assert!(!dest.exists(), "файл с неверной суммой стёрт");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn views_arrive_in_camel_case() {
        let view = View {
            components: vec![ComponentView {
                id: Component::YtDlp,
                name: "yt-dlp",
                presence: Presence::Missing,
                version: None,
                download_bytes: 1,
                disk_bytes: 1,
                checked_at: None,
            }],
            modules: vec![ModuleView {
                id: Module::Downloader,
                needs: Module::Downloader.needs(),
                missing: vec![Component::QuickJs],
            }],
            job: Some(Job { component: Component::Ffmpeg, stage: Stage::Downloading, received: 1, total: Some(2) }),
            failure: Some(Problem { component: Component::Ffmpeg, code: Failure::BadChecksum }),
            notice: None,
            probing: false,
        };
        let json = serde_json::to_string(&view).unwrap();
        for key in ["downloadBytes", "checkedAt", "\"ytDlp\"", "\"quickJs\"", "\"badChecksum\"", "\"downloading\"", "\"needs\""] {
            assert!(json.contains(key), "{key} нет в {json}");
        }
        assert_eq!(serde_json::to_string(&Failure::InUse).unwrap(), "\"inUse\"");
    }

    /// Удаление стирает только папку программы — никогда выше корня.
    #[test]
    fn every_program_folder_is_a_plain_name() {
        for spec in &catalog::SPECS {
            assert!(plain_folder(spec.dir), "папка {:?} не простое имя", spec.dir);
        }
        for bad in ["", ".", "..", "a/b", "a\\b", "C:", "..\\x"] {
            assert!(!plain_folder(bad), "{bad:?} прошло проверку");
        }
    }

    /// Удаление убирает папку программы целиком и не трогает соседей.
    #[test]
    fn removing_a_program_takes_its_folder_and_nothing_else() {
        let root = std::env::temp_dir().join(format!("hw-tools-remove-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);

        let ytdlp = spec(Component::YtDlp);
        let quickjs = spec(Component::QuickJs);
        std::fs::create_dir_all(root.join(ytdlp.dir).join("cache")).unwrap();
        std::fs::write(root.join(ytdlp.dir).join(ytdlp.file), b"exe").unwrap();
        std::fs::write(root.join(ytdlp.dir).join("cache").join("x"), b"cache").unwrap();
        std::fs::create_dir_all(root.join(quickjs.dir)).unwrap();
        std::fs::write(root.join(quickjs.dir).join(quickjs.file), b"exe").unwrap();

        assert_eq!(remove(&root, Component::YtDlp), Ok(()));
        assert!(!root.join(ytdlp.dir).exists(), "папка yt-dlp с кешем ушла");
        assert!(root.join(quickjs.dir).join(quickjs.file).exists(), "сосед на месте");

        // Нечего удалять — не отказ: программы и так нет
        assert_eq!(remove(&root, Component::YtDlp), Ok(()));

        let _ = std::fs::remove_dir_all(&root);
    }
}
