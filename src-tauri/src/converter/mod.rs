//! Конвертер: пачка файлов в MP4, GIF, MP3, PNG или JPG через ffmpeg.
//!
//! Файлы приходят кнопкой выбора (окно Windows) или перетаскиванием на
//! карточку. Каждый сперва пробуется `ffmpeg -i`: видео, звук или картинка, —
//! и пачке остаются только общие для всех её файлов цели. Одна пачка — одни
//! настройки; обрезка — только когда файл в пачке один.
//!
//! Готовое ложится в папку вывода (`Рабочий стол\WinGets\Converter` или своя),
//! никогда не затирая существующее. Пока ffmpeg пишет, файл называется
//! `…​.part`; отмена или отказ стирают и его, и занятое за ним имя.
//!
//! ВАЖНО: **команды только кладут работу в очередь.** Синхронная команда
//! Tauri выполняется в главном потоке, а проба файла и кодирование — это
//! ожидание процесса. Всё ожидание живёт в своём потоке ОС.

mod args;
#[cfg(windows)]
mod popup;
mod preview;
mod probe;

pub use args::{Options, Target};

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

use crate::child::{self, Line, Next};
use crate::tools::{Component, Module, Tools};
use args::{Encoder, TrimError, Trim};
use probe::{Kind, Probe};

const CONVERTER_EVENT: &str = "converter";

/// Над карточкой несут файлы (`true`) или унесли (`false`) — для подсветки.
#[cfg(windows)]
const DROP_EVENT: &str = "converter-drop";

/// Как часто рассылаем прогресс. Чаще — лишние перерисовки всех окон.
const PUBLISH_EVERY: Duration = Duration::from_millis(250);

/// Сколько ждём `ffmpeg -i` на один файл. Он читает только заголовок, но
/// файл на сетевом диске или засыпающем внешнем может отвечать долго.
const PROBE_LIMIT: Duration = Duration::from_secs(20);

/// Потолок пачки. Снимок со всеми строками рассылается до четырёх раз в
/// секунду, и тысяча строк в нём — это уже не карточка, а поломка.
const MAX_ITEMS: usize = 100;

/// Сколько ждём кадр или кусок звука для превью. Кадр из 4K AV1 — меньше
/// секунды; десять — на файл, засыпающий на внешнем диске.
const PREVIEW_LIMIT: Duration = Duration::from_secs(10);

/// Сколько ждём волну: ffmpeg декодирует весь файл, и у часовой записи это
/// десятки секунд.
const WAVE_LIMIT: Duration = Duration::from_secs(120);

/// Сколько ждём пробу NVENC. Отвечает она за пятую долю секунды, а без
/// драйвера отказывает сразу; десять секунд — на первый запуск драйвера.
const NVENC_PROBE_LIMIT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Probing,
    Ready,
    /// ffmpeg файла не прочёл: не видео, не звук и не картинка.
    Unsupported,
    Running,
    Done,
    Failed,
    Cancelled,
}

struct Item {
    id: u64,
    path: PathBuf,
    name: String,
    probe: Option<Probe>,
    kind: Option<Kind>,
    targets: Vec<Target>,
    status: Status,
    progress: Option<f64>,
    /// Последняя строка жалобы ffmpeg — по-английски, как есть.
    reason: Option<String>,
    output: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemView {
    id: u64,
    name: String,
    kind: Option<Kind>,
    duration: Option<f64>,
    status: Status,
    progress: Option<f64>,
    reason: Option<String>,
    /// Имя готового файла — без папки, она одна на всех.
    output: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrimView {
    /// Обрезка доступна: в пачке один видео- или звуковой файл.
    enabled: bool,
    duration: Option<f64>,
    error: Option<TrimError>,
    /// Поля, прочитанные в секунды, — для полосы под ними. Нечитаемое или
    /// пустое поле — `None`: ручка стоит у своего края.
    from: Option<f64>,
    to: Option<f64>,
    /// Что обрезаем: у видео под ручкой кадр, у звука по полосе волна.
    kind: Option<Kind>,
    /// Номер файла: сменился — волну и кадры надо просить заново.
    subject: Option<u64>,
}

/// Ряд потолков для пачки и тот, что пойдёт в работу.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SizeChoice {
    /// Только те, что хоть что-то уменьшат (`args::offered_sizes`).
    offered: Vec<u32>,
    /// Выбранный, приведённый к ряду (`args::chosen_size`). Карточка
    /// подсвечивает его, а кодирует ffmpeg ровно его же.
    chosen: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sizes {
    video: SizeChoice,
    gif: SizeChoice,
    image: SizeChoice,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    items: Vec<ItemView>,
    options: Options,
    /// Общие цели готовых к работе файлов.
    targets: Vec<Target>,
    /// Во что превратится пачка: выбранное, а если оно ей не подходит —
    /// первая из её целей. Звук сразу идёт в MP3: выбирать там не из чего.
    target: Option<Target>,
    sizes: Sizes,
    /// Готовые файлы есть, а общих целей нет: видео и картинки вперемешку.
    mixed: bool,
    trim: TrimView,
    running: bool,
    /// Вся пачка: доля от нуля до единицы.
    overall: Option<f64>,
    out_dir: Option<String>,
    out_dir_custom: bool,
}

enum Order {
    Add(Vec<PathBuf>),
    /// Снимок того, с чем нажали «Старт». Взят в команде, а не в рабочем
    /// потоке: очередь могла стоять за пробой файлов, и чипы, тронутые после
    /// нажатия, — по подписи это настройка следующей пачки — попали бы в эту.
    ///
    /// Файлы — тоже снимком: потолки и обрезка посчитаны по тем, что были
    /// готовы в миг нажатия, а разобравшиеся позже (их пробы стоят в очереди
    /// впереди) остаются следующей пачке — иначе 4K-ролик ушёл бы с потолком,
    /// отсеянным по маленькому, и с чужой обрезкой.
    Start { ids: Vec<u64>, target: Target, options: Options, trim: Option<Trim> },
}

struct Inner {
    items: Vec<Item>,
    options: Options,
    running: bool,
    next_id: u64,
    /// Готово файлов из скольких — пока идёт пачка.
    batch: Option<(usize, usize)>,
}

#[derive(Default)]
pub struct Converter {
    inner: Mutex<Inner>,
    orders: OnceLock<Sender<Order>>,
    cancel: AtomicBool,
    /// Выход необратим: новый файл пачки после него запускать нельзя.
    stopping: AtomicBool,
    stopped: AtomicBool,
    /// Процесс и занятые пути меняют владельца одним шагом при выходе.
    active: Mutex<Option<Active>>,
    /// Чем кодирует этот ffmpeg — проба одна на запуск и на путь к нему:
    /// компонент могут обновить, а системный — сменить.
    encoder: Mutex<Option<(PathBuf, Encoder)>>,
    /// Волна файла, которую уже считали, — по его номеру.
    wave: Mutex<Option<(u64, Vec<f32>)>>,
    /// Номера временных файлов превью: запросы идут внахлёст.
    previews: AtomicU64,
    /// Последняя просьба послушать: запоздавший ответ прежней не играет.
    listens: AtomicU64,
    /// Номер последней просьбы о кадре или его уборки: кадр, вынутый после
    /// «убрать» (мышь ушла, модуль выключили), не всплывает поверх всех.
    frames: AtomicU64,
}

struct Active {
    part: PathBuf,
    reserved: PathBuf,
    process: Option<child::Spawned>,
}

impl Active {
    fn stop(&mut self) {
        if let Some(mut process) = self.process.take() {
            process.kill();
            // Drop ждёт процесс; уборку файлов и выход он задерживать не должен.
            std::thread::spawn(move || drop(process));
        }
    }

    fn discard(&mut self) {
        self.stop();
        discard(&[&self.part, &self.reserved]);
    }
}

/// Обычный выход откладывается до уборки, главный поток продолжает отвечать.
/// Повторный ExitRequested пропускается только после завершения уборки.
pub fn shutdown_later(app: &AppHandle, code: i32) -> bool {
    let converter = app.state::<Converter>();
    // Перезапуск runtime `prevent_exit` не слушает: уборку сделает `RunEvent::Exit`,
    // а свой `app.exit` из потока превратил бы перезапуск в выход
    if converter.stopped.load(Ordering::SeqCst) || code == tauri::RESTART_EXIT_CODE {
        return false;
    }
    if !converter.stopping.swap(true, Ordering::SeqCst) {
        converter.cancel.store(true, Ordering::SeqCst);
        let app = app.clone();
        std::thread::spawn(move || {
            shutdown(&app);
            app.state::<Converter>().stopped.store(true, Ordering::SeqCst);
            app.exit(code);
        });
    }
    true
}

fn clean_shutdown(app: &AppHandle) {
    let converter = app.state::<Converter>();
    converter.shutdown_files();
}

/// Запасной путь, когда runtime уже завершает цикл без ExitRequested.
/// Ждём только ограниченную уборку, а не рабочий поток пачки или ffmpeg.
pub fn shutdown(app: &AppHandle) {
    if app.state::<Converter>().stopped.load(Ordering::SeqCst) {
        return;
    }
    app.state::<Converter>().stopping.store(true, Ordering::SeqCst);
    app.state::<Converter>().cancel.store(true, Ordering::SeqCst);
    let (done, wait) = mpsc::channel();
    let app = app.clone();
    std::thread::spawn(move || {
        clean_shutdown(&app);
        let _ = done.send(());
    });
    if wait.recv_timeout(Duration::from_secs(3)).is_err() {
        eprintln!("[конвертер] срок уборки при выходе истёк");
    }
}

/// Файл, у которого есть обрезка, — всё, что нужно превью.
struct Subject {
    id: u64,
    path: PathBuf,
    duration: f64,
}

/// Что заиграло: откуда и сколько секунд. По нему карточка ведёт бегунок.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Playing {
    start: f64,
    length: f64,
}

impl Default for Inner {
    fn default() -> Self {
        Self { items: Vec::new(), options: Options::default(), running: false, next_id: 1, batch: None }
    }
}

/// Занят ли конвертер ffmpeg прямо сейчас: идёт пачка или проба файлов.
///
/// Спрашивает удаление программ — работающий exe Windows не удалит. Превью
/// обрезки сюда не попадает: оно короткое и флага не имеет, его отказ ловит
/// само удаление.
pub fn uses_tools(app: &AppHandle) -> bool {
    let Some(converter) = app.try_state::<Converter>() else {
        return false;
    };
    let inner = converter.lock();
    inner.running || inner.items.iter().any(|item| matches!(item.status, Status::Probing))
}

impl Converter {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        // Отравленный мьютекс не повод падать: данные внутри осмысленные
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn hush(&self) {
        // Отмена и остановка неделимы относительно запуска готового куска.
        let _turn = SOUND_TURN.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        self.listens.fetch_add(1, Ordering::SeqCst);
        preview::sound::stop();
    }

    fn shutdown_files(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.cancel.store(true, Ordering::SeqCst);
        let mut active = self.active.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(mut job) = active.take() {
            job.discard();
        }
        self.stopped.store(true, Ordering::SeqCst);
    }

    fn order(&self, order: Order) {
        if let Some(orders) = self.orders.get() {
            let _ = orders.send(order);
        }
    }
}

impl Inner {
    fn remove(&mut self, id: u64) {
        let trimmed = self.trim_subject().is_some_and(|item| item.id == id);
        self.items.retain(|item| item.id != id);
        if trimmed {
            // Поля принадлежали удалённому файлу, следующему они не подходят.
            self.options.from.clear();
            self.options.to.clear();
        }
    }

    fn runnable(&self) -> impl Iterator<Item = &Item> {
        self.items.iter().filter(|item| item.status == Status::Ready)
    }

    fn targets(&self) -> Vec<Target> {
        let each: Vec<Vec<Target>> = self.runnable().map(|item| item.targets.clone()).collect();
        args::batch_targets(&each)
    }

    /// Выбранная цель, если она подходит пачке, иначе первая подходящая.
    ///
    /// Выбор при этом не переписывается: выбравший GIF, добавив звук, увидит
    /// MP3, а вернувшись к видео — снова GIF.
    fn target(&self) -> Option<Target> {
        let targets = self.targets();
        self.options
            .target
            .filter(|target| targets.contains(target))
            .or_else(|| targets.first().copied())
    }

    /// Самая большая сторона нужного рода среди файлов пачки: `short` —
    /// короткая (потолок MP4), иначе длинная (GIF и картинки).
    fn largest(&self, short: bool) -> Option<u32> {
        self.runnable()
            .filter_map(|item| item.probe.as_ref()?.video.as_ref())
            .map(|video| if short { video.width.min(video.height) } else { video.width.max(video.height) })
            .max()
    }

    fn sizes(&self) -> Sizes {
        let choice = |offered: Vec<u32>, value: u32, fallback: u32| SizeChoice {
            chosen: args::chosen_size(value, &offered, fallback),
            offered,
        };
        let defaults = Options::default();
        Sizes {
            video: choice(
                args::offered_sizes(&args::VIDEO_SIZES, self.largest(true)),
                self.options.video_size,
                defaults.video_size,
            ),
            gif: choice(args::gif_sizes(self.largest(false)), self.options.gif_size, defaults.gif_size),
            image: choice(
                args::offered_sizes(&args::IMAGE_SIZES, self.largest(false)),
                self.options.image_size,
                defaults.image_size,
            ),
        }
    }

    /// Настройки, с которыми пачка пойдёт в работу: потолки приведены к ряду.
    fn working_options(&self) -> Options {
        let sizes = self.sizes();
        Options {
            video_size: sizes.video.chosen,
            gif_size: sizes.gif.chosen,
            image_size: sizes.image.chosen,
            ..self.options.clone()
        }
    }

    /// Один видео- или звуковой файл — единственный случай, когда обрезка
    /// что-то значит: у пачки нет общего «от–до».
    fn trim_subject(&self) -> Option<&Item> {
        let mut runnable = self.runnable();
        match (runnable.next(), runnable.next()) {
            (Some(item), None) if matches!(item.kind, Some(Kind::Video | Kind::Audio)) => Some(item),
            _ => None,
        }
    }

    fn subject(&self) -> Option<Subject> {
        let item = self.trim_subject()?;
        Some(Subject {
            id: item.id,
            path: item.path.clone(),
            duration: item.probe.as_ref()?.duration.filter(|length| *length > 0.0)?,
        })
    }

    fn trim(&self) -> Result<Option<Trim>, TrimError> {
        match self.trim_subject() {
            Some(item) => args::resolve_trim(
                &self.options.from,
                &self.options.to,
                item.probe.as_ref().and_then(|probe| probe.duration),
            ),
            None => Ok(None),
        }
    }
}

fn snapshot(app: &AppHandle) -> Option<View> {
    let converter = app.try_state::<Converter>()?;
    let (dir, custom) = crate::output::dir_for(app, Module::Converter);
    let inner = converter.lock();

    let items = inner
        .items
        .iter()
        .map(|item| ItemView {
            id: item.id,
            name: item.name.clone(),
            kind: item.kind,
            duration: item.probe.as_ref().and_then(|probe| probe.duration),
            status: item.status,
            progress: item.progress,
            reason: item.reason.clone(),
            output: item
                .output
                .as_ref()
                .and_then(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned()),
        })
        .collect();

    let targets = inner.targets();
    let subject = inner.trim_subject();
    let trim = TrimView {
        enabled: subject.is_some(),
        duration: subject.and_then(|item| item.probe.as_ref()).and_then(|probe| probe.duration),
        error: inner.trim().err(),
        from: args::parse_timecode(&inner.options.from).ok().flatten(),
        to: args::parse_timecode(&inner.options.to).ok().flatten(),
        kind: subject.and_then(|item| item.kind),
        subject: subject.map(|item| item.id),
    };
    let sizes = inner.sizes();

    let overall = inner.batch.map(|(done, total)| {
        let current = inner
            .items
            .iter()
            .find(|item| item.status == Status::Running)
            .and_then(|item| item.progress)
            .unwrap_or(0.0);
        if total == 0 { 1.0 } else { (done as f64 + current) / total as f64 }
    });

    Some(View {
        items,
        options: inner.options.clone(),
        mixed: targets.is_empty() && inner.runnable().next().is_some(),
        target: inner.target(),
        targets,
        sizes,
        trim,
        running: inner.running,
        overall,
        out_dir: dir.map(|dir| dir.to_string_lossy().into_owned()),
        out_dir_custom: custom,
    })
}

pub fn publish(app: &AppHandle) {
    if let Some(view) = snapshot(app) {
        let _ = app.emit(CONVERTER_EVENT, view);
    }
}

pub fn spawn(app: AppHandle) {
    let (orders, inbox) = mpsc::channel();
    if app.state::<Converter>().orders.set(orders).is_err() {
        eprintln!("[конвертер] поток уже поднят");
        return;
    }

    if let Err(error) = std::thread::Builder::new()
        .name("converter".to_owned())
        .spawn(move || work(app, inbox))
    {
        eprintln!("[конвертер] поток не поднялся: {error}");
    }
}

/// Файлы, брошенные на карточку из Проводника, — как и почему не через
/// `onDragDropEvent`, написано в `file_drop.rs`.
///
/// Зовётся главным потоком после сборки окна конвертера — при старте и при
/// включении модуля (`modules.rs`).
#[cfg(windows)]
pub fn accept_drops(app: &AppHandle) {
    let Some(window) = app.get_webview_window("converter") else {
        return;
    };
    let (ask, hover, drop) = (app.clone(), app.clone(), app.clone());
    let attached = crate::file_drop::attach(
        &window,
        move || !ask.state::<Converter>().lock().running,
        // Из оконной процедуры к Tauri не ходим: событие уходит своим
        // потоком, а процедура возвращается сразу
        move |over| {
            let app = hover.clone();
            std::thread::spawn(move || {
                let _ = app.emit_to("converter", DROP_EVENT, over);
            });
        },
        move |paths| {
            let converter = drop.state::<Converter>();
            if !converter.lock().running {
                converter.order(Order::Add(paths));
            }
        },
    );
    if let Err(error) = attached {
        eprintln!("[конвертер] перетаскивание не заработает: {error}");
    }
}

#[cfg(not(windows))]
pub fn accept_drops(_app: &AppHandle) {}

/// Снимает подложку перетаскивания — перед разборкой окна конвертера.
///
/// Главным потоком: подложка и её таймер принадлежат ему.
#[cfg(windows)]
pub fn refuse_drops() {
    crate::file_drop::detach();
}

#[cfg(not(windows))]
pub fn refuse_drops() {}

fn work(app: AppHandle, inbox: Receiver<Order>) {
    for order in inbox {
        if app.state::<Converter>().stopping.load(Ordering::SeqCst) {
            break;
        }
        match order {
            Order::Add(paths) => add(&app, paths),
            Order::Start { ids, target, options, trim } => run_batch(&app, ids, target, options, trim),
        }
    }
}

/// Ставит файлы в список и пробует каждый.
fn add(app: &AppHandle, paths: Vec<PathBuf>) {
    let converter = app.state::<Converter>();
    let ffmpeg = app.state::<Tools>().locate(Component::Ffmpeg);

    // Папки не разворачиваем: пачка — это то, что выбрали, а не всё
    // содержимое диска. Спрашиваем диск до замка: файл на уснувшем сетевом
    // диске держал бы замок, а за ним ждали бы синхронные команды карточки —
    // в главном потоке, вместе с приёмом IPC всех окон
    let paths: Vec<PathBuf> = paths.into_iter().filter(|path| path.is_file()).collect();
    let fresh: Vec<(u64, PathBuf)> = {
        let mut inner = converter.lock();
        let mut fresh = Vec::new();
        for path in paths {
            if inner.items.len() >= MAX_ITEMS {
                continue;
            }
            let queued = inner.items.iter().any(|item| {
                item.path == path && matches!(item.status, Status::Probing | Status::Ready)
            });
            if queued {
                continue;
            }

            let id = inner.next_id;
            inner.next_id += 1;
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            inner.items.push(Item {
                id,
                path: path.clone(),
                name,
                probe: None,
                kind: None,
                targets: Vec::new(),
                status: Status::Probing,
                progress: None,
                reason: None,
                output: None,
            });
            fresh.push((id, path));
        }
        fresh
    };
    publish(app);

    for (id, path) in fresh {
        // Модуль выключили посреди проб — дальше ffmpeg не зовём: у
        // выключенного модуля фоновой работы нет
        if !crate::modules::on(app, "converter") {
            // Неразобранные строки не висят «…» навсегда: удалению ffmpeg
            // они казались бы работой, а включённый заново модуль их уже не
            // разберёт — заказ ушёл
            converter.lock().items.retain(|item| !matches!(item.status, Status::Probing));
            publish(app);
            break;
        }
        let probed = ffmpeg.as_deref().map(|ffmpeg| probe_file(ffmpeg, &path));
        let mut inner = converter.lock();
        let Some(item) = inner.items.iter_mut().find(|item| item.id == id) else {
            continue;
        };
        match probed {
            Some((Some(probe), Some(kind))) => {
                item.targets = args::targets(kind, probe.audio.is_some());
                item.kind = Some(kind);
                item.probe = Some(probe);
                item.status = Status::Ready;
            }
            Some(_) => item.status = Status::Unsupported,
            None => {
                item.status = Status::Failed;
                item.reason = Some(tr!("converter.error.noFfmpeg"));
            }
        }
        drop(inner);
        publish(app);
    }
}

/// Что внутри файла.
fn probe_file(ffmpeg: &Path, path: &Path) -> (Option<Probe>, Option<Kind>) {
    let mut command = Command::new(ffmpeg);
    command.args(["-hide_banner", "-nostdin", "-i"]).arg(path);
    let Ok((_, lines)) = child::run(command, PROBE_LIMIT) else {
        return (None, None);
    };
    let texts: Vec<&str> = lines.iter().map(Line::text).collect();
    let Some(probe) = probe::parse(&texts) else {
        return (None, None);
    };

    let frames = if probe::needs_frame_count(&probe) {
        let mut command = Command::new(ffmpeg);
        command
            .args(["-hide_banner", "-nostdin", "-v", "error", "-i"])
            .arg(path)
            .args(["-map", "0:v:0", "-c", "copy", "-f", "null", "-", "-progress", "pipe:1"]);
        child::run(command, PROBE_LIMIT).ok().and_then(|(_, lines)| {
            let texts: Vec<&str> = lines.iter().map(Line::text).collect();
            probe::parse_frames(&texts)
        })
    } else {
        None
    };

    let kind = probe::classify(&probe, frames);
    (Some(probe), kind)
}

/// Чем кончилась одна конвертация.
enum Outcome {
    Cancelled,
    Failed(String),
}

fn run_batch(app: &AppHandle, chosen: Vec<u64>, target: Target, options: Options, trim: Option<Trim>) {
    let converter = app.state::<Converter>();

    let ids = {
        let mut inner = converter.lock();
        // Из нажатого — то, что ещё в списке и ждёт: строку могли убрать,
        // пока заказ стоял в очереди за пробами
        let ids: Vec<u64> = inner.runnable().map(|item| item.id).filter(|id| chosen.contains(id)).collect();
        if ids.is_empty() {
            inner.running = false;
            drop(inner);
            publish(app);
            return;
        }
        inner.batch = Some((0, ids.len()));
        ids
    };
    publish(app);

    let ffmpeg = app.state::<Tools>().locate(Component::Ffmpeg);
    let (dir, _) = crate::output::dir_for(app, Module::Converter);
    // Кодировщик нужен только видео; проба — один раз на запуск
    let encoder = match (&ffmpeg, target) {
        (Some(ffmpeg), Target::Mp4) => encoder_for(&converter, ffmpeg),
        _ => Encoder::X264,
    };

    for id in ids {
        if converter.cancel.load(Ordering::SeqCst) || converter.stopping.load(Ordering::SeqCst) {
            break;
        }
        let image = matches!(target, Target::Png | Target::Jpg);
        set_item(&converter, id, |item| {
            item.status = Status::Running;
            item.progress = (!image).then_some(0.0);
            item.reason = None;
        });
        publish(app);

        let outcome = match (&ffmpeg, &dir) {
            (Some(ffmpeg), Some(dir)) => {
                let job = Job { target, options: &options, trim, encoder };
                convert(app, ffmpeg, id, &job, dir)
            }
            (None, _) => Err(Outcome::Failed(tr!("converter.error.noFfmpeg"))),
            (_, None) => Err(Outcome::Failed(tr!("converter.error.noFolder"))),
        };

        set_item(&converter, id, |item| {
            item.progress = None;
            match outcome {
                Ok(output) => {
                    item.status = Status::Done;
                    item.output = Some(output);
                }
                Err(Outcome::Cancelled) => item.status = Status::Cancelled,
                Err(Outcome::Failed(reason)) => {
                    item.status = Status::Failed;
                    item.reason = Some(reason);
                }
            }
        });
        if let Some((done, _)) = converter.lock().batch.as_mut() {
            *done += 1;
        }
        publish(app);
    }

    {
        let mut inner = converter.lock();
        inner.running = false;
        inner.batch = None;
    }
    converter.cancel.store(false, Ordering::SeqCst);
    publish(app);
}

fn set_item(converter: &Converter, id: u64, change: impl FnOnce(&mut Item)) {
    if let Some(item) = converter.lock().items.iter_mut().find(|item| item.id == id) {
        change(item);
    }
}

/// Чем кодирует этот ffmpeg: проба NVENC одним кадром, ответ — до выхода.
fn encoder_for(converter: &Converter, ffmpeg: &Path) -> Encoder {
    let mut known = converter.encoder.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((path, encoder)) = known.as_ref() {
        if path == ffmpeg {
            return *encoder;
        }
    }

    let mut command = Command::new(ffmpeg);
    command.args(args::nvenc_probe());
    let encoder = match child::run(command, NVENC_PROBE_LIMIT) {
        Ok((Some(0), _)) => Encoder::Nvenc,
        _ => Encoder::X264,
    };
    eprintln!("[конвертер] видео кодирует {encoder:?}");
    *known = Some((ffmpeg.to_owned(), encoder));
    encoder
}

/// Что делаем с каждым файлом пачки.
struct Job<'a> {
    target: Target,
    options: &'a Options,
    trim: Option<Trim>,
    encoder: Encoder,
}

/// Один файл: от занятого имени до готового файла на нём.
fn convert(app: &AppHandle, ffmpeg: &Path, id: u64, job: &Job, dir: &Path) -> Result<PathBuf, Outcome> {
    let converter = app.state::<Converter>();
    let Job { target, options, trim, encoder } = *job;
    let (path, probe) = {
        let inner = converter.lock();
        let item = inner
            .items
            .iter()
            .find(|item| item.id == id)
            .ok_or_else(|| Outcome::Failed(tr!("converter.error.gone")))?;
        (item.path.clone(), item.probe.clone())
    };
    let probe = probe.ok_or_else(|| Outcome::Failed(tr!("converter.error.notProbed")))?;

    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "converted".to_owned());
    // Занятие путей и передача уборке неделимы относительно выхода.
    let mut active = converter.active.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if converter.stopping.load(Ordering::SeqCst) {
        return Err(Outcome::Cancelled);
    }
    let reserved = crate::output::reserve(dir, &stem, target.ext()).map_err(|error| {
        eprintln!("[конвертер] имя не заняли в {}: {error}", dir.display());
        Outcome::Failed(tr!("output.error.cannotWrite"))
    })?;
    let part = crate::output::reserve_part(&reserved).map_err(|error| {
        eprintln!("[конвертер] рабочий файл не заняли рядом с {}: {error}", reserved.display());
        let _ = std::fs::remove_file(&reserved);
        Outcome::Failed(tr!("output.error.cannotWrite"))
    })?;

    *active = Some(Active { part: part.clone(), reserved: reserved.clone(), process: None });
    drop(active);

    let length = if matches!(target, Target::Png | Target::Jpg) {
        None
    } else {
        args::segment(probe.duration, trim)
    };
    let run = |encoder| {
        let arguments = args::ffmpeg_args(target, options, &probe, trim, encoder, &path, &part);
        encode(app, ffmpeg, id, arguments, &part, length)
    };

    let mut outcome = run(encoder);
    // Видеокарта отказала на этом файле — например, кадр больше 4096 точек,
    // которых H.264 у NVENC не берёт. Файл не виноват: процессор справится
    if encoder == Encoder::Nvenc && matches!(outcome, Err(Outcome::Failed(_))) {
        eprintln!("[конвертер] NVENC отказал на {}, кодируем процессором", path.display());
        set_item(&converter, id, |item| item.progress = Some(0.0));
        publish(app);
        outcome = run(Encoder::X264);
    }
    let mut active = converter.active.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(mut owned) = active.take() else {
        return Err(Outcome::Cancelled);
    };
    if converter.stopping.load(Ordering::SeqCst) || converter.cancel.load(Ordering::SeqCst) {
        outcome = Err(Outcome::Cancelled);
    }
    if let Err(outcome) = outcome {
        owned.discard();
        return Err(outcome);
    }

    crate::output::place(&part, &reserved).map_err(|error| {
        eprintln!("[конвертер] готовый файл не встал на место: {error}");
        owned.discard();
        Outcome::Failed(tr!("output.error.cannotWrite"))
    })?;
    Ok(reserved)
}

/// Один проход ffmpeg с прогрессом. Недописанное за собой не убирает — это
/// дело `convert`, который знает и про занятое имя.
fn encode(
    app: &AppHandle,
    ffmpeg: &Path,
    id: u64,
    arguments: Vec<std::ffi::OsString>,
    part: &Path,
    length: Option<f64>,
) -> Result<(), Outcome> {
    let converter = app.state::<Converter>();
    let mut command = Command::new(ffmpeg);
    command.args(arguments);
    {
        let mut active = converter.active.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if converter.stopping.load(Ordering::SeqCst) || converter.cancel.load(Ordering::SeqCst) {
            return Err(Outcome::Cancelled);
        }
        let owned = active.as_mut().ok_or(Outcome::Cancelled)?;
        owned.stop();
        owned.process = Some(child::spawn(command).map_err(|error| {
            eprintln!("[конвертер] ffmpeg не запустился: {error}");
            Outcome::Failed(tr!("converter.error.noStart"))
        })?);
    }

    let mut shown = Instant::now();
    let mut complaints: Vec<String> = Vec::new();

    let code = loop {
        if converter.stopping.load(Ordering::SeqCst) || converter.cancel.load(Ordering::SeqCst) {
            return Err(Outcome::Cancelled);
        }
        let next = {
            let mut active = converter.active.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let owned = active.as_mut().ok_or(Outcome::Cancelled)?;
            let process = owned.process.as_mut().ok_or(Outcome::Cancelled)?;
            match process.next(Duration::from_millis(200)) {
                Next::Closed => match process.try_wait() {
                    Ok(Some(status)) => {
                        // Процесс уже завершился: Drop здесь больше не ждёт.
                        owned.process.take();
                        break status.code();
                    }
                    Ok(None) => Next::Idle,
                    Err(error) => {
                        eprintln!("[конвертер] конец ffmpeg не проверился: {error}");
                        return Err(Outcome::Failed(tr!("converter.error.failed")));
                    }
                },
                next => next,
            }
        };
        match next {
            Next::Line(Line::Out(text)) => {
                if let (Some(Some(done)), Some(length)) = (args::progress_seconds(&text), length) {
                    set_item(&converter, id, |item| item.progress = Some(args::fraction(done, length)));
                    if shown.elapsed() >= PUBLISH_EVERY {
                        shown = Instant::now();
                        publish(app);
                    }
                }
            }
            Next::Line(Line::Err(text)) => {
                if !text.trim().is_empty() {
                    complaints.push(text);
                    if complaints.len() > 3 {
                        complaints.remove(0);
                    }
                }
            }
            Next::Idle => std::thread::sleep(Duration::from_millis(10)),
            Next::Closed => unreachable!(),
        }
    };

    if converter.cancel.load(Ordering::SeqCst) {
        return Err(Outcome::Cancelled);
    }
    if code != Some(0) || !part.is_file() {
        eprintln!("[конвертер] ffmpeg отказал ({code:?}): {}", complaints.join(" | "));
        let reason = complaints.pop().unwrap_or_else(|| tr!("converter.error.failed"));
        return Err(Outcome::Failed(reason));
    }
    Ok(())
}

/// Стирает недописанное и занятое за ним имя.
///
/// Убитый ffmpeg отпускает файл не мгновенно, поэтому несколько попыток с
/// паузой: брошенный `.part` в папке вывода выглядел бы готовым файлом.
fn discard(paths: &[&Path]) {
    let mut pending = paths.to_vec();
    for attempt in 0..10 {
        pending.retain(|path| path.exists() && std::fs::remove_file(path).is_err());
        if pending.is_empty() {
            return;
        }
        if attempt < 9 {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    for path in pending {
        eprintln!("[конвертер] недописанный файл не удалился: {}", path.display());
    }
}

#[tauri::command]
pub fn converter_state(app: AppHandle) -> Option<View> {
    snapshot(&app)
}

/// Выбор файлов окном Windows поверх карточки.
#[tauri::command]
pub fn converter_pick(app: AppHandle, window: WebviewWindow) {
    if app.state::<Converter>().lock().running {
        return;
    }
    let owner = window.hwnd().map(|hwnd| hwnd.0 as isize).unwrap_or(0);
    crate::picker::pick(owner, crate::picker::Pick::Files, None, move |chosen| {
        if !chosen.is_empty() {
            app.state::<Converter>().order(Order::Add(chosen));
        }
    });
}

#[tauri::command]
pub fn converter_remove(app: AppHandle, id: u64) {
    // Звучать файлу, которого в списке уже нет, незачем
    app.state::<Converter>().hush();
    {
        let converter = app.state::<Converter>();
        let mut inner = converter.lock();
        if inner.running {
            return;
        }
        inner.remove(id);
    }
    publish(&app);
}

#[tauri::command]
pub fn converter_clear(app: AppHandle) {
    app.state::<Converter>().hush();
    {
        let converter = app.state::<Converter>();
        let mut inner = converter.lock();
        if inner.running {
            return;
        }
        inner.items.clear();
        // Обрезка относилась к ушедшему файлу
        inner.options.from.clear();
        inner.options.to.clear();
    }
    publish(&app);
}

#[tauri::command]
pub fn converter_set(app: AppHandle, options: Options) {
    app.state::<Converter>().lock().options = options.sane();
    publish(&app);
}

#[tauri::command]
pub fn converter_start(app: AppHandle) -> Result<(), String> {
    app.state::<Converter>().hush();
    if !app.state::<Tools>().ready(Module::Converter) {
        return Err("notReady".to_owned());
    }
    let converter = app.state::<Converter>();
    if converter.stopping.load(Ordering::SeqCst) {
        return Err("busy".to_owned());
    }
    let order;
    {
        let mut inner = converter.lock();
        if inner.running {
            return Err("busy".to_owned());
        }
        if inner.runnable().next().is_none() {
            return Err("empty".to_owned());
        }
        if inner.target().is_none() {
            return Err("noTarget".to_owned());
        }
        if inner.trim().is_err() {
            return Err("badTrim".to_owned());
        }
        inner.running = true;
        order = Order::Start {
            ids: inner.runnable().map(|item| item.id).collect(),
            target: inner.target().expect("цель проверена выше"),
            options: inner.working_options(),
            trim: inner.trim().ok().flatten(),
        };
    }
    converter.cancel.store(false, Ordering::SeqCst);
    converter.order(order);
    publish(&app);
    Ok(())
}

#[tauri::command]
pub fn converter_cancel(app: AppHandle) {
    app.state::<Converter>().cancel.store(true, Ordering::SeqCst);
}

#[tauri::command]
pub fn converter_reveal(app: AppHandle, id: u64) {
    let output = app
        .state::<Converter>()
        .lock()
        .items
        .iter()
        .find(|item| item.id == id)
        .and_then(|item| item.output.clone());
    if let Some(output) = output {
        crate::links::reveal(&output);
    }
}

/// Всё, что нужно любому превью: ffmpeg, файл с обрезкой и папка под
/// временные файлы. ffmpeg пишет кадр и звук в файл, а не в трубу: построчное
/// чтение вывода двоичного не переживёт, а свой читатель ради двух случаев
/// дороже временного файла.
fn preview_setup(app: &AppHandle) -> Result<(PathBuf, Subject, PathBuf), String> {
    // Окно уже разбирают: ни кадр, ни волна, ни звук никому не нужны
    if !crate::modules::on(app, "converter") {
        return Err("off".to_owned());
    }
    let tools = app.state::<Tools>();
    let ffmpeg = tools.locate(Component::Ffmpeg).ok_or("noFfmpeg")?;
    let subject = app.state::<Converter>().lock().subject().ok_or("noSubject")?;
    let dir = tools.work_dir().ok_or("noDir")?.join("preview");
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok((ffmpeg, subject, dir))
}

/// Гоняет ffmpeg, который пишет один файл, и отдаёт его байты. Файл
/// стирается сразу: превью живёт в памяти, а не на диске.
fn render(ffmpeg: &Path, args: Vec<std::ffi::OsString>, output: &Path) -> Result<Vec<u8>, String> {
    let mut command = Command::new(ffmpeg);
    command.args(args);
    let outcome = child::run(command, PREVIEW_LIMIT);
    let bytes = std::fs::read(output);
    let _ = std::fs::remove_file(output);
    match (outcome, bytes) {
        (Ok((Some(0), _)), Ok(bytes)) if !bytes.is_empty() => Ok(bytes),
        (Ok((code, lines)), _) => {
            let complaint = lines.last().map(Line::text).unwrap_or_default().to_owned();
            eprintln!("[конвертер] превью не вышло ({code:?}): {complaint}");
            Err("failed".to_owned())
        }
        (Err(error), _) => Err(error),
    }
}

/// Где на экране ручка, над которой встанет кадр, — в точках webview.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
pub struct Handle {
    x: f64,
    top: f64,
    bottom: f64,
}

fn frame(app: &AppHandle, at: f64, width: u32, height: u32) -> Result<Vec<u8>, String> {
    if !at.is_finite() {
        return Err("badTime".to_owned());
    }
    let (ffmpeg, subject, dir) = preview_setup(app)?;
    // У самого конца кадра может не оказаться: последний стоит чуть раньше
    // длины, и поиск ровно в неё не находит ничего
    let at = at.clamp(0.0, (subject.duration - 0.1).max(0.0));
    let number = app.state::<Converter>().previews.fetch_add(1, Ordering::SeqCst);
    let output = dir.join(format!("frame-{number}.bmp"));
    render(&ffmpeg, preview::frame_args(&subject.path, at, width, height, &output), &output)
}

fn wave(app: &AppHandle) -> Result<Vec<f32>, String> {
    let (ffmpeg, subject, _) = preview_setup(app)?;
    let converter = app.state::<Converter>();
    if let Some((id, bars)) = converter.wave.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref() {
        if *id == subject.id {
            return Ok(bars.clone());
        }
    }

    let mut command = Command::new(&ffmpeg);
    command.args(preview::wave_args(&subject.path, subject.duration));
    let (code, lines) = child::run(command, WAVE_LIMIT)?;
    // В какой из потоков `ametadata` пишет `file=-`, у сборок бывает по-разному:
    // разбор ищет свои строки в обоих
    let bars = preview::parse_wave(lines.iter().map(Line::text));
    if code != Some(0) || bars.is_empty() {
        eprintln!("[конвертер] волна не посчиталась ({code:?})");
        return Err("failed".to_owned());
    }

    *converter.wave.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((subject.id, bars.clone()));
    Ok(bars)
}

fn listen(app: &AppHandle, from: f64, until: f64, tail: bool) -> Result<Playing, String> {
    let ticket = app.state::<Converter>().listens.fetch_add(1, Ordering::SeqCst) + 1;
    let (ffmpeg, subject, dir) = preview_setup(app)?;
    let (start, length) =
        preview::listen_span(from, until, subject.duration, tail).ok_or("badSpan")?;
    let output = dir.join(format!("clip-{ticket}.wav"));
    let wav = render(&ffmpeg, preview::clip_args(&subject.path, start, length, &output), &output)?;

    // Пока резали этот кусок, попросили другой — этот уже не нужен. Проверка
    // и запуск под одним замком с остановкой: иначе «Стоп», пришедший между
    // ними, остановил бы тишину, а запоздавший кусок заиграл бы после него
    play_current_sound(&app.state::<Converter>().listens, ticket, || preview::sound::play(wav))?;
    Ok(Playing { start, length })
}

/// Кадр видео в этом месте — своим окном над ручкой (`popup.rs`).
///
/// `async`, а не обычная: ffmpeg идёт до секунды, и в главном потоке это
/// стояние всего приложения. Само ожидание — в пуле
/// блокирующих задач, а окно кадра показывает главный поток.
#[tauri::command]
pub async fn converter_frame(app: AppHandle, window: WebviewWindow, at: f64, handle: Handle) -> Result<(), String> {
    let scale = window.scale_factor().map_err(|error| error.to_string())?;
    let origin = window.inner_position().map_err(|error| error.to_string())?;
    let (box_width, box_height) = preview::FRAME_BOX;
    let (width, height) = ((box_width * scale).round() as u32, (box_height * scale).round() as u32);

    let ticket = app.state::<Converter>().frames.fetch_add(1, Ordering::SeqCst) + 1;
    let task = app.clone();
    let image = tauri::async_runtime::spawn_blocking(move || frame(&task, at, width, height))
        .await
        .map_err(|error| error.to_string())??;
    if app.state::<Converter>().frames.load(Ordering::SeqCst) != ticket || !crate::modules::on(&app, "converter") {
        return Err("superseded".to_owned());
    }

    #[cfg(windows)]
    {
        let anchor = popup::Anchor {
            x: origin.x + (handle.x * scale).round() as i32,
            top: origin.y + (handle.top * scale).round() as i32,
            bottom: origin.y + (handle.bottom * scale).round() as i32,
        };
        let main = app.clone();
        app.run_on_main_thread(move || {
            // Пока callback ждал очередь, модуль могли выключить и скрыть кадр.
            // Проверка и показ идут одним шагом главного потока вместе с hide
            show_current_frame(
                &main.state::<Converter>().frames,
                ticket,
                crate::modules::on(&main, "converter"),
                || popup::show(image, anchor, scale),
            );
        }).map_err(|error| error.to_string())?;
    }
    #[cfg(not(windows))]
    let _ = (image, origin, handle);
    Ok(())
}

#[cfg(any(windows, test))]
fn show_current_frame(frames: &AtomicU64, ticket: u64, enabled: bool, show: impl FnOnce()) {
    if frames.load(Ordering::SeqCst) == ticket && enabled {
        show();
    }
}

/// Убрать кадр: мышь ушла с полосы, ручку взяли снова или файл сменился.
#[tauri::command]
pub fn converter_frame_hide(app: AppHandle) {
    // И тот кадр, что ещё вынимается, показывать уже не надо
    app.state::<Converter>().frames.fetch_add(1, Ordering::SeqCst);
    #[cfg(windows)]
    let _ = app.run_on_main_thread(popup::hide);
    #[cfg(not(windows))]
    let _ = app;
}

/// Волна звука для полосы обрезки: `preview::WAVE_BARS` столбиков от нуля
/// до единицы. Считается один раз на файл.
#[tauri::command]
pub async fn converter_wave(app: AppHandle) -> Result<Vec<f32>, String> {
    tauri::async_runtime::spawn_blocking(move || wave(&app)).await.map_err(|error| error.to_string())?
}

/// Послушать с места обрезки: от левой ручки вперёд или конец перед правой.
/// Отвечает, когда звук уже пошёл, — с этого мига карточка и ведёт бегунок.
#[tauri::command]
pub async fn converter_listen(app: AppHandle, from: f64, until: f64, tail: bool) -> Result<Playing, String> {
    tauri::async_runtime::spawn_blocking(move || listen(&app, from, until, tail))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn converter_hush(app: AppHandle) {
    app.state::<Converter>().hush();
}

fn play_current_sound(listens: &AtomicU64, ticket: u64, play: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    let _turn = SOUND_TURN.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if listens.load(Ordering::SeqCst) != ticket {
        return Err("superseded".to_owned());
    }
    play()
}

/// Проверка номера прослушивания и запуск звука — одним шагом против «Стоп».
static SOUND_TURN: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_removes_only_the_active_pair_and_cannot_be_undone_by_batch_cancel_reset() {
        let dir = std::env::temp_dir().join(format!("windgets-converter-shutdown-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&dir).unwrap();
        let reserved = crate::output::reserve(&dir, "clip", "mp4").unwrap();
        let part = crate::output::reserve_part(&reserved).unwrap();
        std::fs::write(&part, b"unfinished").unwrap();
        let finished = dir.join("finished.mp4");
        std::fs::write(&finished, b"finished").unwrap();
        let converter = Converter::default();
        *converter.active.lock().unwrap() = Some(Active { part: part.clone(), reserved: reserved.clone(), process: None });
        converter.shutdown_files();
        converter.cancel.store(false, Ordering::SeqCst);
        converter.shutdown_files();
        assert!(converter.stopping.load(Ordering::SeqCst));
        assert!(converter.stopped.load(Ordering::SeqCst));
        assert!(converter.active.lock().unwrap().is_none());
        assert!(!part.exists() && !reserved.exists());
        assert_eq!(std::fs::read(&finished).unwrap(), b"finished");
        std::fs::remove_file(finished).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    /// Фоновая нарезка закончилась после остановки: до звукового API не доходит.
    #[test]
    fn hush_cancels_a_clip_still_rendering_in_the_background() {
        let converter = Converter::default();
        let ticket = converter.listens.fetch_add(1, Ordering::SeqCst) + 1;
        let played = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let (ready, wait) = mpsc::channel();
            let converter = &converter;
            let played = &played;
            let pending = scope.spawn(move || {
                wait.recv().unwrap();
                play_current_sound(&converter.listens, ticket, || {
                    played.store(true, Ordering::SeqCst);
                    Ok(())
                })
            });
            converter.hush();
            ready.send(()).unwrap();
            assert_eq!(pending.join().unwrap(), Err("superseded".to_owned()));
        });
        assert!(!played.load(Ordering::SeqCst));
        let fresh = converter.listens.fetch_add(1, Ordering::SeqCst) + 1;
        assert!(play_current_sound(&converter.listens, fresh, || Ok(())).is_ok());
    }

    #[test]
    fn removing_the_trim_subject_resets_its_range_only() {
        let mut inner = Inner::default();
        inner.items.push(item(1, Kind::Audio, true, Status::Ready));
        inner.items.push(item(2, Kind::Audio, true, Status::Done));
        inner.options.from = "10".to_owned();
        inner.options.to = "20".to_owned();
        inner.remove(2);
        assert_eq!((&*inner.options.from, &*inner.options.to), ("10", "20"));
        inner.remove(1);
        inner.items.push(item(3, Kind::Audio, true, Status::Ready));
        assert!(inner.options.from.is_empty() && inner.options.to.is_empty());
        assert_eq!(inner.trim(), Ok(None));
    }


    /// Имитируем очередь главного потока: после постановки показа успели
    /// скрыть кадр, попросить другой или выключить модуль без смены билета.
    #[test]
    fn a_queued_frame_checks_its_ticket_and_module_at_display_time() {
        use std::cell::Cell;

        for (current, enabled, expected) in [(1, true, true), (2, true, false), (1, false, false)] {
            let frames = AtomicU64::new(1);
            let module = Cell::new(true);
            let shown = Cell::new(false);
            let queued = || show_current_frame(&frames, 1, module.get(), || shown.set(true));
            frames.store(current, Ordering::SeqCst);
            module.set(enabled);
            queued();
            assert_eq!(shown.get(), expected);
        }
    }

    fn item(id: u64, kind: Kind, has_audio: bool, status: Status) -> Item {
        Item {
            id,
            path: PathBuf::from(format!("C:\\{id}")),
            name: id.to_string(),
            probe: Some(Probe { format: "x".to_owned(), duration: Some(60.0), video: None, audio: None }),
            kind: Some(kind),
            targets: args::targets(kind, has_audio),
            status,
            progress: None,
            reason: None,
            output: None,
        }
    }

    /// Готовые и упавшие файлы в пачку не входят: цели и обрезку решают
    /// только те, что ещё ждут.
    #[test]
    fn only_waiting_files_decide_targets_and_trim() {
        let mut inner = Inner::default();
        inner.items.push(item(1, Kind::Image, false, Status::Done));
        inner.items.push(item(2, Kind::Video, true, Status::Ready));
        assert_eq!(inner.targets(), vec![Target::Mp4, Target::Gif, Target::Mp3]);
        assert!(inner.trim_subject().is_some(), "один видеофайл — обрезка есть");

        inner.items.push(item(3, Kind::Video, false, Status::Ready));
        assert_eq!(inner.targets(), vec![Target::Mp4, Target::Gif]);
        assert!(inner.trim_subject().is_none(), "два файла — обрезки нет");

        inner.options.from = "5:00".to_owned();
        assert_eq!(inner.trim(), Ok(None), "без обрезки поля не проверяются");
    }

    /// Выбор, которого пачка не допускает, не требует нового выбора: звук
    /// сразу идёт в MP3, а прежний выбор возвращается вместе с видео.
    #[test]
    fn a_target_that_does_not_fit_falls_back_without_being_forgotten() {
        let mut inner = Inner::default();
        assert_eq!(inner.target(), None, "пустой пачке превращаться не во что");

        inner.items.push(item(1, Kind::Audio, true, Status::Ready));
        assert_eq!(inner.target(), Some(Target::Mp3), "звуку выбирать не из чего");

        inner.options.target = Some(Target::Gif);
        assert_eq!(inner.target(), Some(Target::Mp3));

        inner.items.clear();
        inner.items.push(item(2, Kind::Video, true, Status::Ready));
        assert_eq!(inner.target(), Some(Target::Gif), "выбор пережил звук");
    }

    /// Предлагаемые потолки считаются по самому большому файлу пачки: его
    /// ещё есть что уменьшать.
    #[test]
    fn the_largest_file_decides_the_sizes() {
        let sized = |id, width, height| {
            let mut item = item(id, Kind::Video, true, Status::Ready);
            item.probe.as_mut().unwrap().video = Some(probe::Video {
                codec: "h264".to_owned(),
                pix_fmt: None,
                width,
                height,
                fps: None,
                stream: None,
            });
            item
        };
        let mut inner = Inner::default();
        inner.items.push(sized(1, 1280, 720));
        inner.items.push(sized(2, 1080, 1920));
        assert_eq!(inner.largest(true), Some(1080), "короткая сторона вертикального ролика");
        assert_eq!(inner.largest(false), Some(1920));

        // Готовые файлы в пачку уже не входят
        inner.items[1].status = Status::Done;
        assert_eq!(inner.largest(true), Some(720));

        // 1080p у ролика 720p — «как есть», и в работу идёт тоже он
        inner.options.video_size = 1080;
        assert_eq!(inner.sizes().video.offered, vec![0, 480]);
        assert_eq!(inner.working_options().video_size, 0);
        assert_eq!(inner.options.video_size, 1080, "сам выбор не переписан");
    }

    #[test]
    fn a_bad_trim_is_reported_for_a_single_file() {
        let mut inner = Inner::default();
        inner.items.push(item(1, Kind::Audio, true, Status::Ready));
        inner.options.from = "2:00".to_owned();
        assert_eq!(inner.trim(), Err(TrimError::BadFrom));
    }

    /// Живая проверка на ffmpeg из PATH — том, что стоит у человека: проба
    /// настоящих файлов и каждая строка конвертации до готового файла.
    #[test]
    #[ignore = "нужен ffmpeg в PATH: cargo test -- --ignored real_files"]
    fn real_files_are_probed_and_converted() {
        let ffmpeg = crate::tools::system_ffmpeg().expect("ffmpeg с x264 и LAME в PATH");
        let dir = std::env::temp_dir().join(format!("hw-converter-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let make = |args: &[&str], name: &str| {
            let mut command = Command::new(&ffmpeg);
            command.args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).arg(dir.join(name));
            let (code, _) = child::run(command, Duration::from_secs(60)).unwrap();
            assert_eq!(code, Some(0), "образец {name} не собрался");
            dir.join(name)
        };
        let video = make(
            &["-f", "lavfi", "-i", "testsrc2=s=640x360:r=25:d=3", "-f", "lavfi", "-i", "sine=d=3",
              "-shortest", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"],
            "clip with space.mp4",
        );
        let alpha = make(&["-f", "lavfi", "-i", "color=c=red@0.5:s=300x200,format=rgba", "-frames:v", "1"], "alpha.png");
        let gif = make(&["-f", "lavfi", "-i", "testsrc2=s=160x120:r=10:d=1"], "anim.gif");

        let (probe, kind) = probe_file(&ffmpeg, &video);
        let probe = probe.expect("видео разобралось");
        assert_eq!(kind, Some(Kind::Video));
        assert!(probe.audio.is_some());
        assert!((probe.duration.unwrap() - 3.0).abs() < 0.2);

        let (alpha_probe, alpha_kind) = probe_file(&ffmpeg, &alpha);
        assert_eq!(alpha_kind, Some(Kind::Image));
        assert_eq!(probe_file(&ffmpeg, &gif).1, Some(Kind::Video), "десять кадров — ролик");

        // Видеокарта — если проба её нашла; процессор — всегда
        let found = encoder_for(&Converter::default(), &ffmpeg);
        eprintln!("кодировщик на этой машине: {found:?}");

        let trim = Some(Trim { from: 0.5, length: Some(1.5) });
        let jobs = [
            (Target::Mp4, Encoder::X264, &probe, trim, &video),
            (Target::Mp4, found, &probe, trim, &video),
            (Target::Gif, Encoder::X264, &probe, None, &video),
            (Target::Mp3, Encoder::X264, &probe, trim, &video),
            (Target::Jpg, Encoder::X264, alpha_probe.as_ref().unwrap(), None, &alpha),
            (Target::Png, Encoder::X264, alpha_probe.as_ref().unwrap(), None, &alpha),
        ];
        for (target, encoder, probe, trim, input) in jobs {
            let output = dir.join(format!("out.{}", target.ext()));
            let options = Options { image_size: 640, video_size: 720, ..Options::default() };
            let mut command = Command::new(&ffmpeg);
            command.args(args::ffmpeg_args(target, &options, probe, trim, encoder, input, &output));
            let (code, lines) = child::run(command, Duration::from_secs(120)).unwrap();
            let complaints: Vec<&str> = lines.iter().filter(|line| matches!(line, Line::Err(_))).map(Line::text).collect();
            assert_eq!(code, Some(0), "{target:?} {encoder:?}: {complaints:?}");
            assert!(output.metadata().unwrap().len() > 0, "{target:?}: пустой файл");
            assert!(
                lines.iter().any(|line| line.text() == "progress=end"),
                "{target:?}: нет конца прогресса"
            );
        }

        let (cut, _) = probe_file(&ffmpeg, &dir.join("out.mp4"));
        assert!((cut.unwrap().duration.unwrap() - 1.5).abs() < 0.2, "обрезка не сработала");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_view_arrives_in_camel_case() {
        let view = View {
            items: Vec::new(),
            options: Options::default(),
            targets: vec![Target::Mp4],
            target: Some(Target::Mp4),
            sizes: Sizes {
                video: SizeChoice { offered: vec![0], chosen: 0 },
                gif: SizeChoice { offered: vec![320], chosen: 320 },
                image: SizeChoice { offered: vec![0], chosen: 0 },
            },
            mixed: false,
            trim: TrimView {
                enabled: true,
                duration: Some(1.0),
                error: Some(TrimError::EmptyRange),
                from: Some(0.5),
                to: None,
                kind: Some(Kind::Audio),
                subject: Some(1),
            },
            running: false,
            overall: None,
            out_dir: None,
            out_dir_custom: false,
        };
        let json = serde_json::to_string(&view).unwrap();
        for key in ["outDirCustom", "videoSize", "gifFps", "imageSize", "\"emptyRange\"", "\"mp4\"", "\"sizes\""] {
            assert!(json.contains(key), "{key} нет в {json}");
        }
    }
}
