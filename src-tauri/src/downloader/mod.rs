//! Загрузчик: видео или звук по ссылке через yt-dlp.
//!
//! Любая ссылка, которую понимает yt-dlp, — YouTube и тысячи других сайтов.
//! Видео — MP4 с потолком 720p, 1080p или лучшее; звук — MP3. Плейлисты не
//! качаем: ссылка на плейлист даёт первый ролик.
//!
//! yt-dlp качает в рабочую папку каталога компонентов, а готовый файл
//! переезжает в папку вывода (`Рабочий стол\WinGets\Downloads` или своя) под
//! свободным именем. Так отмена — это «погасить и стереть рабочую папку», а
//! существующий файл с тем же названием не затирается никогда.
//!
//! ВАЖНО: **yt-dlp ломается вслед за сайтами.** Перед загрузкой он
//! проверяется на свежесть не чаще раза в сутки, а отказ, похожий на поломку
//! сайта, обновляет его сразу и повторяет загрузку один раз.
//!
//! Команды только кладут работу в очередь: ожидание процесса живёт в своём
//! потоке ОС, а не в главном: синхронная команда Tauri идёт главным потоком.

mod args;

pub use args::{Mode, Quality};

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::child::{self, Line, Next};
use crate::tools::{Component, Module, Tools};
use args::{Event, Reason};

const DOWNLOADER_EVENT: &str = "downloader";

/// Как часто рассылаем прогресс.
const PUBLISH_EVERY: Duration = Duration::from_millis(250);

/// Сколько последних жалоб yt-dlp держим для разбора причины.
const COMPLAINTS: usize = 20;

/// Сколько скачанного помним — как у буфера обмена, двадцать строк.
const HISTORY: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    #[default]
    Idle,
    Preparing,
    /// Проверяем или ставим свежий yt-dlp.
    Updating,
    Downloading,
    /// Склейка видео со звуком или перевод в MP3.
    Processing,
    /// Переезд готового файла в папку вывода — с другого диска это копия.
    Saving,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Step {
    Merging,
    Converting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    index: usize,
    count: usize,
}

#[derive(Debug, Clone, Default)]
struct State {
    status: Status,
    url: Option<String>,
    title: Option<String>,
    format_ids: Option<String>,
    part: Option<Part>,
    downloaded: Option<u64>,
    total: Option<u64>,
    speed: Option<f64>,
    eta: Option<u64>,
    step: Option<Step>,
    reason: Option<Reason>,
    detail: Option<String>,
    output: Option<PathBuf>,
    mode: Mode,
    quality: Quality,
}

/// Скачанное за этот запуск.
///
/// ВАЖНО: **только в памяти, как у буфера обмена.** Так решил пользователь:
/// история — до выхода из приложения, на диск она не попадает.
#[derive(Debug, Clone)]
struct Saved {
    id: u64,
    url: String,
    title: String,
    output: PathBuf,
    mode: Mode,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedView {
    id: u64,
    url: String,
    title: String,
    /// Имя файла в папке вывода.
    file: String,
    mode: Mode,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    status: Status,
    url: Option<String>,
    title: Option<String>,
    part: Option<Part>,
    /// Доля скачанного у текущей части.
    percent: Option<f64>,
    downloaded: Option<u64>,
    total: Option<u64>,
    /// Байт в секунду.
    speed: Option<f64>,
    eta: Option<u64>,
    step: Option<Step>,
    reason: Option<Reason>,
    detail: Option<String>,
    /// Имя готового файла.
    output: Option<String>,
    mode: Mode,
    quality: Quality,
    out_dir: Option<String>,
    out_dir_custom: bool,
    /// Свежее сверху.
    history: Vec<SavedView>,
}

struct Job {
    url: String,
    mode: Mode,
    quality: Quality,
}

#[derive(Default)]
pub struct Downloader {
    state: Mutex<State>,
    orders: OnceLock<Sender<Job>>,
    running: AtomicBool,
    cancel: AtomicBool,
    /// Номер рабочей папки: у каждой загрузки своя.
    jobs: AtomicU64,
    /// Отдельно от `state`: тот обнуляется на каждой новой загрузке.
    history: Mutex<Vec<Saved>>,
    saved: AtomicU64,
}

/// Идёт ли загрузка: yt-dlp, QuickJS и ffmpeg тогда заняты. Спрашивает
/// удаление программ — работающий exe Windows не удалит.
pub fn uses_tools(app: &AppHandle) -> bool {
    app.try_state::<Downloader>()
        .is_some_and(|downloader| downloader.running.load(Ordering::SeqCst))
}

impl Downloader {
    fn lock(&self) -> MutexGuard<'_, State> {
        // Отравленный мьютекс не повод падать: данные внутри осмысленные
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn history(&self) -> MutexGuard<'_, Vec<Saved>> {
        self.history.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Кладёт скачанное наверх истории.
    fn remember(&self, url: String, title: String, output: PathBuf, mode: Mode) {
        let id = self.saved.fetch_add(1, Ordering::SeqCst);
        remember(&mut self.history(), Saved { id, url, title, output, mode });
    }
}

/// Повтор той же ссылки в том же режиме переезжает наверх, а не ложится второй
/// строкой — как повтор в буфере обмена. Видео и звук одной ссылки — две
/// разные строки: файлы у них разные.
fn remember(history: &mut Vec<Saved>, saved: Saved) {
    history.retain(|old| !(old.url == saved.url && old.mode == saved.mode));
    history.insert(0, saved);
    history.truncate(HISTORY);
}

fn snapshot(app: &AppHandle) -> Option<View> {
    let downloader = app.try_state::<Downloader>()?;
    let (dir, custom) = crate::output::dir_for(app, Module::Downloader);
    let history = downloader
        .history()
        .iter()
        .map(|saved| SavedView {
            id: saved.id,
            url: saved.url.clone(),
            title: saved.title.clone(),
            file: file_name(&saved.output),
            mode: saved.mode,
        })
        .collect();
    let state = downloader.lock();

    let percent = match (state.downloaded, state.total) {
        (Some(done), Some(total)) if total > 0 => Some((done as f64 / total as f64).clamp(0.0, 1.0)),
        _ => None,
    };

    Some(View {
        status: state.status,
        url: state.url.clone(),
        title: state.title.clone(),
        part: state.part,
        percent,
        downloaded: state.downloaded,
        total: state.total,
        speed: state.speed,
        eta: state.eta,
        step: state.step,
        reason: state.reason,
        detail: state.detail.clone(),
        output: state.output.as_deref().map(file_name),
        mode: state.mode,
        quality: state.quality,
        out_dir: dir.map(|dir| dir.to_string_lossy().into_owned()),
        out_dir_custom: custom,
        history,
    })
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
}

pub fn publish(app: &AppHandle) {
    if let Some(view) = snapshot(app) {
        let _ = app.emit(DOWNLOADER_EVENT, view);
    }
}

fn update(app: &AppHandle, change: impl FnOnce(&mut State)) {
    change(&mut app.state::<Downloader>().lock());
}

pub fn spawn(app: AppHandle) {
    let (orders, inbox) = mpsc::channel();
    if app.state::<Downloader>().orders.set(orders).is_err() {
        eprintln!("[загрузчик] поток уже поднят");
        return;
    }

    if let Err(error) = std::thread::Builder::new()
        .name("downloader".to_owned())
        .spawn(move || work(app, inbox))
    {
        eprintln!("[загрузчик] поток не поднялся: {error}");
    }
}

fn work(app: AppHandle, inbox: Receiver<Job>) {
    for job in inbox {
        run(&app, &job);
        let downloader = app.state::<Downloader>();
        downloader.cancel.store(false, Ordering::SeqCst);
        downloader.running.store(false, Ordering::SeqCst);
        publish(&app);
    }
}

/// Чем кончилась попытка.
enum Failure {
    Cancelled,
    Reason(Reason, Option<String>),
}

fn run(app: &AppHandle, job: &Job) {
    let downloader = app.state::<Downloader>();
    // Отмену во время обновления проверяем сами: у обновления своя отмена
    // (её дёргает `downloader_cancel`), и без проверки загрузка пошла бы
    // дальше, а отменённая после поломки сайта кончилась бы «сайт сломался»
    // вместо «отменено». Нашло это ревью второй модели
    let cancelled = || downloader.cancel.load(Ordering::SeqCst);

    // Свежесть yt-dlp — раз в сутки, и не повод срывать загрузку: без сети
    // проверка просто не пройдёт, а загрузка скажет о сети сама
    if app.state::<Tools>().update_due(Component::YtDlp) {
        update(app, |state| state.status = Status::Updating);
        publish(app);
        let _ = crate::tools::refresh_blocking(app, Component::YtDlp, false);
    }

    let mut outcome = if cancelled() { Err(Failure::Cancelled) } else { attempt(app, job) };

    // Похоже на поломку сайта — обновляем и пробуем ещё раз, один.
    //
    // ВАЖНО: **повтор — и тогда, когда свежего yt-dlp не нашлось.** Отказ
    // «403 Forbidden» у YouTube бывает и мимолётным, а по тексту он неотличим
    // от поломки. Повтор только после обновления сдавался на таком отказе
    // сразу — «сайт сломался», — а та же ссылка со второго нажатия качалась
    // (живая проверка 25 сентября)
    if let Err(Failure::Reason(Reason::Broken, _)) = outcome {
        update(app, |state| state.status = Status::Updating);
        publish(app);
        let _ = crate::tools::refresh_blocking(app, Component::YtDlp, true);
        outcome = if cancelled() { Err(Failure::Cancelled) } else { attempt(app, job) };
    }

    // Название — из того, что сказал yt-dlp, а без него — имя файла
    if let Ok(output) = &outcome {
        let downloader = app.state::<Downloader>();
        let title = downloader.lock().title.clone();
        let title = title.unwrap_or_else(|| {
            output.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default()
        });
        downloader.remember(job.url.clone(), title, output.clone(), job.mode);
    }

    update(app, |state| {
        state.speed = None;
        state.eta = None;
        state.step = None;
        match outcome {
            Ok(output) => {
                state.status = Status::Done;
                state.output = Some(output);
            }
            Err(Failure::Cancelled) => state.status = Status::Cancelled,
            Err(Failure::Reason(reason, detail)) => {
                state.status = Status::Failed;
                state.reason = Some(reason);
                state.detail = detail;
            }
        }
    });
}

/// Одна попытка: от запуска yt-dlp до готового файла в папке вывода.
fn attempt(app: &AppHandle, job: &Job) -> Result<PathBuf, Failure> {
    let downloader = app.state::<Downloader>();
    let tools = app.state::<Tools>();
    let failed = |reason| Failure::Reason(reason, None);

    let (Some(ytdlp), Some(quickjs), Some(ffmpeg)) = (
        tools.locate(Component::YtDlp),
        tools.locate(Component::QuickJs),
        tools.locate(Component::Ffmpeg),
    ) else {
        return Err(failed(Reason::Failed));
    };
    let (Some(work_root), Some(cache)) = (tools.work_dir(), tools.cache_dir(Component::YtDlp)) else {
        return Err(failed(Reason::Failed));
    };
    let work = work_root.join(downloader.jobs.fetch_add(1, Ordering::SeqCst).to_string());
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|error| {
        eprintln!("[загрузчик] рабочая папка не создалась: {error}");
        failed(Reason::Failed)
    })?;

    update(app, |state| {
        state.status = Status::Preparing;
        state.title = None;
        state.format_ids = None;
        state.part = None;
        state.downloaded = None;
        state.total = None;
        state.speed = None;
        state.eta = None;
        state.step = None;
        state.reason = None;
        state.detail = None;
    });
    publish(app);

    let paths = args::Paths { ffmpeg: &ffmpeg, quickjs: &quickjs, work: &work, cache: &cache };
    let mut command = Command::new(&ytdlp);
    command
        .args(args::ytdlp_args(job.mode, job.quality, &paths, &job.url))
        .current_dir(&work)
        // Python в трубу пишет кусками и в кодировке системы: без этого
        // прогресс приходил бы пачками, а кириллица в названиях — вопросами
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUNBUFFERED", "1");

    let outcome = follow(app, command);
    let result = outcome.and_then(|file| save(app, &file));
    let _ = std::fs::remove_dir_all(&work);
    result
}

/// Ведёт yt-dlp до конца, разбирая его строки. Отдаёт путь готового файла в
/// рабочей папке.
fn follow(app: &AppHandle, command: Command) -> Result<PathBuf, Failure> {
    let downloader = app.state::<Downloader>();
    let mut spawned = child::spawn(command).map_err(|error| {
        eprintln!("[загрузчик] yt-dlp не запустился: {error}");
        Failure::Reason(Reason::Failed, None)
    })?;

    let mut file = None;
    let mut complaints: Vec<String> = Vec::new();
    let mut shown = Instant::now();

    loop {
        if downloader.cancel.load(Ordering::SeqCst) {
            spawned.kill();
        }
        let line = match spawned.next(Duration::from_millis(200)) {
            Next::Line(line) => line,
            Next::Idle => continue,
            Next::Closed => break,
        };

        match args::parse_line(line.text()) {
            Some(Event::Title { format_id, title }) => update(app, |state| {
                state.title = Some(title);
                state.format_ids = Some(format_id);
            }),
            Some(Event::Progress { finished, downloaded, total, speed, eta, format_id }) => {
                update(app, |state| {
                    state.status = Status::Downloading;
                    state.part = state
                        .format_ids
                        .as_deref()
                        .and_then(|ids| args::part_of(ids, &format_id))
                        .map(|(index, count)| Part { index, count });
                    state.downloaded = if finished { total.or(downloaded) } else { downloaded };
                    state.total = total;
                    state.speed = speed;
                    state.eta = eta;
                });
                if !finished && shown.elapsed() < PUBLISH_EVERY {
                    continue;
                }
                shown = Instant::now();
            }
            Some(Event::Step { started, name }) => {
                let step = match name.as_str() {
                    "Merger" | "VideoRemuxer" => Some(Step::Merging),
                    "ExtractAudio" => Some(Step::Converting),
                    _ => None,
                };
                if started && step.is_some() {
                    update(app, |state| {
                        state.status = Status::Processing;
                        state.step = step;
                        state.speed = None;
                        state.eta = None;
                    });
                }
            }
            Some(Event::File(path)) => file = Some(path),
            None => {
                if let Line::Err(text) = &line {
                    if !text.trim().is_empty() {
                        complaints.push(text.clone());
                        if complaints.len() > COMPLAINTS {
                            complaints.remove(0);
                        }
                    }
                }
                continue;
            }
        }
        publish(app);
    }

    let code = spawned.wait();
    drop(spawned);

    if downloader.cancel.load(Ordering::SeqCst) {
        return Err(Failure::Cancelled);
    }
    if code != Some(0) {
        eprintln!("[загрузчик] yt-dlp отказал ({code:?}): {}", complaints.join(" | "));
        return Err(Failure::Reason(args::classify_error(&complaints), args::detail(&complaints)));
    }
    file.filter(|path| path.is_file()).ok_or_else(|| {
        eprintln!("[загрузчик] yt-dlp кончил без файла: {}", complaints.join(" | "));
        Failure::Reason(Reason::Failed, args::detail(&complaints))
    })
}

/// Переносит готовый файл в папку вывода под свободным именем.
fn save(app: &AppHandle, file: &Path) -> Result<PathBuf, Failure> {
    update(app, |state| {
        state.status = Status::Saving;
        state.step = None;
    });
    publish(app);

    let failed = || Failure::Reason(Reason::Failed, Some(tr!("output.error.cannotWrite")));
    let (Some(dir), _) = crate::output::dir_for(app, Module::Downloader) else {
        return Err(failed());
    };
    let stem = file
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "download".to_owned());
    let ext = file
        .extension()
        .map(|ext| ext.to_string_lossy().into_owned())
        .unwrap_or_else(|| "bin".to_owned());

    let reserved = crate::output::reserve(&dir, &stem, &ext).map_err(|error| {
        eprintln!("[загрузчик] имя не заняли в {}: {error}", dir.display());
        failed()
    })?;
    crate::output::place(file, &reserved).map_err(|error| {
        eprintln!("[загрузчик] файл не переехал: {error}");
        let _ = std::fs::remove_file(&reserved);
        failed()
    })?;
    Ok(reserved)
}

#[tauri::command]
pub fn downloader_state(app: AppHandle) -> Option<View> {
    snapshot(&app)
}

#[tauri::command]
pub fn downloader_start(app: AppHandle, url: String, mode: Mode, quality: Quality) -> Result<(), String> {
    let url = url.trim().to_owned();
    // Та же проверка, что у ссылок из заметок: только http и https, без
    // пробелов и управляющих знаков. Остальное yt-dlp и не нужно
    if !crate::links::is_safe_link(&url) {
        return Err("badUrl".to_owned());
    }
    if !app.state::<Tools>().ready(Module::Downloader) {
        return Err("notReady".to_owned());
    }

    let downloader = app.state::<Downloader>();
    if downloader.running.swap(true, Ordering::SeqCst) {
        return Err("busy".to_owned());
    }
    downloader.cancel.store(false, Ordering::SeqCst);

    *downloader.lock() = State {
        status: Status::Preparing,
        url: Some(url.clone()),
        mode,
        quality,
        ..State::default()
    };
    publish(&app);

    match downloader.orders.get() {
        Some(orders) if orders.send(Job { url, mode, quality }).is_ok() => Ok(()),
        _ => {
            // Поток загрузчика не поднялся или умер: «подготовка» осталась бы
            // на карточке навсегда — снимать её больше некому
            eprintln!("[загрузчик] очередь не принимает задание");
            {
                let mut state = downloader.lock();
                state.status = Status::Failed;
                state.reason = Some(Reason::Failed);
                state.detail = Some("The downloader stopped, restart the app".to_owned());
            }
            downloader.running.store(false, Ordering::SeqCst);
            publish(&app);
            Err("notReady".to_owned())
        }
    }
}

/// Режим и потолок запоминаются до следующего запуска приложения — в памяти,
/// как и опции конвертера: файла такое не заслуживает.
#[tauri::command]
pub fn downloader_options(app: AppHandle, mode: Mode, quality: Quality) {
    update(&app, |state| {
        state.mode = mode;
        state.quality = quality;
    });
    publish(&app);
}

#[tauri::command]
pub fn downloader_cancel(app: AppHandle) {
    let downloader = app.state::<Downloader>();
    downloader.cancel.store(true, Ordering::SeqCst);
    // Обновление yt-dlp качают компоненты, и нашей отмены оно не видит: без
    // этого «Cancel» ждал бы конца скачивания в семнадцать мегабайт
    if downloader.lock().status == Status::Updating {
        app.state::<Tools>().stop();
    }
}

/// Показывает файл в папке: последний скачанный или строку истории.
#[tauri::command]
pub fn downloader_reveal(app: AppHandle, id: Option<u64>) {
    let downloader = app.state::<Downloader>();
    let output = match id {
        Some(id) => downloader.history().iter().find(|saved| saved.id == id).map(|saved| saved.output.clone()),
        None => downloader.lock().output.clone(),
    };
    if let Some(output) = output {
        crate::links::reveal(&output);
    }
}

/// Убирает строку из истории, а без номера — всю историю. Скачанные файлы
/// остаются на месте: забывается только список.
#[tauri::command]
pub fn downloader_forget(app: AppHandle, id: Option<u64>) {
    {
        let downloader = app.state::<Downloader>();
        let mut history = downloader.history();
        match id {
            Some(id) => history.retain(|saved| saved.id != id),
            None => history.clear(),
        }
    }
    publish(&app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_view_arrives_in_camel_case() {
        let view = View {
            status: Status::Processing,
            url: None,
            title: None,
            part: Some(Part { index: 1, count: 2 }),
            percent: Some(0.5),
            downloaded: None,
            total: None,
            speed: None,
            eta: None,
            step: Some(Step::Merging),
            reason: Some(Reason::SignIn),
            detail: None,
            output: None,
            mode: Mode::Audio,
            quality: Quality::P1080,
            out_dir: None,
            out_dir_custom: true,
            history: vec![SavedView {
                id: 1,
                url: "https://example.com".to_owned(),
                title: "Clip".to_owned(),
                file: "Clip.mp4".to_owned(),
                mode: Mode::Video,
            }],
        };
        let json = serde_json::to_string(&view).unwrap();
        for key in ["outDirCustom", "\"processing\"", "\"merging\"", "\"signIn\"", "\"p1080\"", "\"audio\"", "\"history\""] {
            assert!(json.contains(key), "{key} нет в {json}");
        }
    }

    fn saved(id: u64, url: &str, mode: Mode) -> Saved {
        Saved { id, url: url.to_owned(), title: String::new(), output: PathBuf::new(), mode }
    }

    /// Свежее сверху; повтор той же ссылки в том же режиме переезжает
    /// наверх, звук той же ссылки — своя строка.
    #[test]
    fn a_repeated_link_moves_up_instead_of_doubling() {
        let mut history = Vec::new();
        remember(&mut history, saved(1, "a", Mode::Video));
        remember(&mut history, saved(2, "b", Mode::Video));
        remember(&mut history, saved(3, "a", Mode::Audio));
        remember(&mut history, saved(4, "a", Mode::Video));
        let ids: Vec<u64> = history.iter().map(|saved| saved.id).collect();
        assert_eq!(ids, vec![4, 3, 2]);
    }

    #[test]
    fn the_history_keeps_only_the_latest() {
        let mut history = Vec::new();
        for id in 0..(HISTORY as u64 + 5) {
            remember(&mut history, saved(id, &id.to_string(), Mode::Video));
        }
        assert_eq!(history.len(), HISTORY);
        assert_eq!(history[0].id, HISTORY as u64 + 4);
    }
}
