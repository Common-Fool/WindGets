//! Счётчик кадров игры: FPS, 1 % low, время кадра, упор в видеокарту.
//!
//! Датчика FPS нет ни у LibreHardwareMonitor, ни у драйвера, ни у Windows:
//! кадры видны только в ETW (события Present у DXGI, D3D9 и Vulkan). Разбирает
//! их PresentMon от Intel — докачиваемая программа (`tools-components.md`),
//! которая печатает строку CSV на каждый кадр каждого процесса. В игру ничего
//! не внедряется, античиту смотреть не на что. ETW требует прав
//! администратора — они у приложения и так есть.
//!
//! PresentMon видит всех, кто выводит кадры: браузер, плеер, наш же WebView2.
//! Поэтому цель одна — **игра на переднем плане**. Щелчок по виджету уводит
//! передний план к нам, и цель при этом не теряется: держится последняя,
//! пока на передний план не выйдет другой процесс с кадрами. Свои окна,
//! оболочка, браузеры и мессенджеры целью не бывают вовсе ([`IGNORED`]).
//!
//! Работает по правилу звука: PresentMon запущен, только пока окно виджета
//! видно. Спрятали — процесс гасится и сеанс ETW снимается.

mod frames;
mod stats;

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::child::{self, Line, Next, Spawned};
use crate::tools::catalog::Component;
use crate::tools::Tools;
use frames::{Columns, Frame};
use stats::{Summary, Tracker};

/// Метка окна виджета.
const WIDGET: &str = "fps";

/// Событие, которым снимок расходится по окнам.
const FPS_EVENT: &str = "fps";

/// Имя сеанса ETW. Своё, а не умолчание «PresentMon»: иначе запуск снимал бы
/// сеанс у PresentMon, CapFrameX или FrameView, если человек держит их рядом.
const SESSION: &str = "WindgetsFps";

/// Шаг счёта и рассылки: четверть секунды. Раз в секунду число заметно
/// отставало от игры (слово пользователя).
const TICK: Duration = Duration::from_millis(1000 / stats::TICKS_PER_SECOND as u64);

/// Сколько процесс считается «рисующим» после последнего кадра.
const FRESH: Duration = Duration::from_secs(2);

/// Пауза перед новой попыткой, если PresentMon упал или не поднялся.
const RETRY: Duration = Duration::from_secs(10);

/// Сколько цель может молчать, прежде чем её забыть: игру закрыли или
/// свернули надолго. Карточка возвращается к «Ждём игру», а не держит имя и
/// low-показатели закрытой игры (живая проверка).
const LOST: Duration = Duration::from_secs(5);

/// Кто целью не бывает никогда: наши окна, оболочка Windows, браузеры и
/// прочий интерфейс, который рисует кадры через DirectX, но игрой не является.
/// Эти же имена уходят в PresentMon ключами `--exclude`, чтобы их кадры не
/// гонять по трубе вовсе.
const IGNORED: &[&str] = &[
    "windgets.exe",
    "msedgewebview2.exe",
    "dwm.exe",
    "explorer.exe",
    "ShellExperienceHost.exe",
    "StartMenuExperienceHost.exe",
    "SearchHost.exe",
    "TextInputHost.exe",
    "ApplicationFrameHost.exe",
    "msedge.exe",
    "chrome.exe",
    "firefox.exe",
    "opera.exe",
    "brave.exe",
    "vivaldi.exe",
    "zen.exe",
    "Discord.exe",
    "Telegram.exe",
    "Spotify.exe",
    "steamwebhelper.exe",
    "EpicGamesLauncher.exe",
    "obs64.exe",
];

fn ignored(name: &str) -> bool {
    IGNORED.iter().any(|ignored| ignored.eq_ignore_ascii_case(name))
}

/// Что сейчас с карточкой.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    /// Окно не видно — ничего не меряем.
    #[default]
    Idle,
    /// PresentMon не скачан.
    NoTool,
    /// Меряем, но игры на переднем плане ещё не было.
    Waiting,
    /// Есть цель.
    Live,
    /// PresentMon не поднялся или упал; через [`RETRY`] пробуем снова.
    Failed,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub status: Status,
    /// Заголовок окна игры, без него — имя файла без `.exe`.
    pub app: Option<String>,
    /// Способ вывода последнего кадра, как его называет PresentMon.
    pub mode: Option<String>,
    #[serde(flatten)]
    pub summary: Summary,
    /// FPS по секундам, от старых к новым.
    pub history: Vec<f32>,
    /// Последняя строка ошибки PresentMon — в подсказку и журнал.
    pub problem: Option<String>,
}

#[derive(Default)]
pub struct Fps(Mutex<View>);

impl Fps {
    fn locked(&self) -> MutexGuard<'_, View> {
        self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Запущен ли PresentMon: удалить его exe в это время нельзя.
static RUNNING: AtomicBool = AtomicBool::new(false);

pub fn uses_tools() -> bool {
    RUNNING.load(Ordering::Relaxed)
}

#[tauri::command]
pub fn fps_state(state: State<'_, Fps>) -> View {
    state.locked().clone()
}

pub fn spawn(app: AppHandle) {
    if let Err(error) = std::thread::Builder::new()
        .name("fps".to_owned())
        .spawn(move || work(app))
    {
        eprintln!("[fps] поток не поднялся: {error}");
    }
}

fn watched(app: &AppHandle) -> bool {
    app.get_webview_window(WIDGET)
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false)
}

fn publish(app: &AppHandle, next: View) {
    let state = app.state::<Fps>();
    {
        let mut stored = state.locked();
        if *stored == next {
            return;
        }
        *stored = next.clone();
    }
    let _ = app.emit(FPS_EVENT, next);
}

/// Процесс, чьё окно на переднем плане, и заголовок этого окна.
///
/// Заголовок — имя игры для людей: «S.T.A.L.K.E.R. 2: Heart of Chornobyl», а
/// не `Stalker2-Win64-Shipping.exe`, как зовут файл движки вроде Unreal.
#[cfg(windows)]
fn foreground() -> Option<(u32, String)> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId};
    // SAFETY: вызовы только читают; пустое окно даёт PID ноль, буфер свой
    unsafe {
        let window = GetForegroundWindow();
        let mut pid = 0u32;
        GetWindowThreadProcessId(window, Some(&mut pid));
        let mut buffer = [0u16; 256];
        let length = GetWindowTextW(window, &mut buffer).max(0) as usize;
        let title = String::from_utf16_lossy(&buffer[..length.min(buffer.len())]).trim().to_owned();
        (pid != 0).then_some((pid, title))
    }
}

#[cfg(not(windows))]
fn foreground() -> Option<(u32, String)> {
    None
}

/// Кто рисовал кадры недавно: имя и время последнего кадра.
struct Seen {
    name: String,
    last: Instant,
}

/// Выбор цели.
///
/// Процесс на переднем плане становится целью, если он рисует кадры и не из
/// [`IGNORED`]. Иначе остаётся прежняя цель: передний план мог уйти к виджету,
/// в Discord или на рабочий стол, а игра — та же.
fn choose(current: Option<u32>, front: Option<u32>, seen: &HashMap<u32, Seen>, now: Instant, own: u32) -> Option<u32> {
    let candidate = front.filter(|pid| *pid != own).and_then(|pid| {
        let entry = seen.get(&pid)?;
        (now.duration_since(entry.last) <= FRESH && !ignored(&entry.name)).then_some(pid)
    });
    candidate.or(current)
}

fn shorten(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((head, tail)) if !head.is_empty() && tail.eq_ignore_ascii_case("exe") => head.to_owned(),
        _ => name.to_owned(),
    }
}

fn start(exe: &Path) -> Result<Spawned, String> {
    let mut command = Command::new(exe);
    command.args([
        "--output_stdout",
        "--no_console_stats",
        "--v2_metrics",
        "--no_track_input",
        "--session_name",
        SESSION,
        // Упавший или убитый прошлый запуск оставляет сеанс ETW живым
        "--stop_existing_session",
    ]);
    for name in IGNORED {
        command.args(["--exclude", name]);
    }
    child::spawn_listener(command)
}

/// Гасит PresentMon и снимает его сеанс ETW.
///
/// ВАЖНО: **убитый процесс сеанс не снимает** — тот живёт в ядре, пока его не
/// остановят. Мирно остановить консольную программу без окна нечем, поэтому
/// сеанс снимает `logman`.
fn stop(running: Spawned) {
    drop(running);
    RUNNING.store(false, Ordering::Relaxed);
    close_session();
}

/// Снимает сеанс на выходе и перезапуске, если PresentMon работает.
///
/// PresentMon на выходе убивает объект задания, а убитый сеанс не снимает:
/// без этого сеанс собирал бы события в ядре до следующего показа счётчика
/// или до перезагрузки.
pub fn shutdown() {
    if RUNNING.swap(false, Ordering::Relaxed) {
        close_session();
    }
}

fn close_session() {
    #[cfg(windows)]
    {
        let mut command = Command::new(crate::autostart::system_exe("System32\\logman.exe"));
        command.args(["stop", SESSION, "-ets"]);
        let _ = child::run(command, Duration::from_secs(10));
    }
}

/// Работающий PresentMon и то, что о нём известно.
struct Run {
    spawned: Spawned,
    columns: Option<Columns>,
    problem: Option<String>,
}

/// Состояние счёта, общее для тактов.
#[derive(Default)]
struct Count {
    target: Option<u32>,
    /// Основная цепочка вывода цели — та, что дала больше кадров.
    chain: Option<String>,
    /// Кадры текущего такта. Цель выбирается в конце такта, и без этой
    /// копилки первая пачка новой игры терялась бы целиком (замечание Argon).
    pending: Vec<Frame>,
    name: Option<String>,
    mode: Option<String>,
    seen: HashMap<u32, Seen>,
    tracker: Tracker,
}

impl Count {
    fn take(&mut self, frame: Frame, now: Instant) {
        self.seen.insert(frame.pid, Seen { name: frame.app.clone(), last: now });
        self.pending.push(frame);
    }

    /// Кадры такта отдаются счёту цели — только основной цепочки.
    ///
    /// ВАЖНО: **цепочки не смешиваются.** Время кадра PresentMon считает по
    /// каждой отдельно, и сцена на 100 FPS со вспомогательным окном на 50
    /// давали бы в сумме 75 (замечание Sol). Основная — у которой больше
    /// кадров за такт; прежняя держится, пока она вообще рисует.
    fn feed(&mut self) {
        let pending = std::mem::take(&mut self.pending);
        let Some(target) = self.target else { return };
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for frame in pending.iter().filter(|frame| frame.pid == target) {
            *counts.entry(frame.chain.as_str()).or_default() += 1;
        }
        let keeps = self.chain.as_deref().is_some_and(|chain| counts.contains_key(chain));
        if !keeps {
            if let Some((chain, _)) = counts.iter().max_by_key(|(chain, count)| (**count, std::cmp::Reverse(**chain))) {
                self.chain = Some((*chain).to_owned());
            }
        }
        for frame in pending.iter().filter(|frame| frame.pid == target && Some(&frame.chain) == self.chain.as_ref()) {
            self.tracker.push(frame);
            if frame.mode.is_some() {
                self.mode = frame.mode.clone();
            }
        }
    }

    fn tick(&mut self, now: Instant) -> View {
        let own = std::process::id();
        let front = foreground();
        let alive = self.target.filter(|pid| {
            self.seen.get(pid).is_some_and(|seen| now.duration_since(seen.last) <= LOST)
        });
        let next = choose(alive, front.as_ref().map(|(pid, _)| *pid), &self.seen, now, own);
        if next != self.target {
            self.target = next;
            self.tracker = Tracker::default();
            self.chain = None;
            self.mode = None;
            self.name = next.and_then(|pid| self.seen.get(&pid)).map(|seen| shorten(&seen.name));
        }
        // Имя — заголовок окна игры, пока она впереди; без заголовка остаётся
        // имя файла
        if let Some((pid, title)) = &front {
            if Some(*pid) == self.target && !title.is_empty() {
                self.name = Some(title.clone());
            }
        }
        // Давно молчащие — уже не кандидаты, а память не бесконечна
        self.seen
            .retain(|pid, seen| Some(*pid) == self.target || now.duration_since(seen.last) <= FRESH * 5);

        self.feed();
        if self.target.is_none() {
            return View { status: Status::Waiting, ..View::default() };
        }
        let summary = self.tracker.close_tick();
        View {
            status: Status::Live,
            app: self.name.clone(),
            mode: self.mode.clone(),
            summary,
            history: self.tracker.history(),
            problem: None,
        }
    }
}

fn work(app: AppHandle) {
    let mut running: Option<Run> = None;
    let mut count = Count::default();
    let mut retry_at: Option<Instant> = None;
    let mut failed: Option<String> = None;

    loop {
        if !watched(&app) {
            if let Some(run) = running.take() {
                stop(run.spawned);
            }
            // Спрятанное окно меряет заново: старая история за часы до показа
            // выдавала бы себя за нынешнюю
            count = Count::default();
            // Спрятали и показали — пробуем сразу, а не дожидаемся старого срока
            retry_at = None;
            failed = None;
            publish(&app, View::default());
            std::thread::sleep(Duration::from_millis(500));
            continue;
        }

        let Some(exe) = app.state::<Tools>().locate(Component::PresentMon) else {
            if let Some(run) = running.take() {
                stop(run.spawned);
            }
            count = Count::default();
            retry_at = None;
            failed = None;
            publish(&app, View { status: Status::NoTool, ..View::default() });
            std::thread::sleep(Duration::from_secs(1));
            continue;
        };

        if running.is_none() {
            if retry_at.is_some_and(|at| Instant::now() < at) {
                publish(&app, View { status: Status::Failed, problem: failed.clone(), ..View::default() });
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }
            match start(&exe) {
                Ok(spawned) => {
                    RUNNING.store(true, Ordering::Relaxed);
                    running = Some(Run { spawned, columns: None, problem: None });
                    retry_at = None;
                    failed = None;
                }
                Err(error) => {
                    eprintln!("[fps] PresentMon не запустился: {error}");
                    failed = Some(error);
                    retry_at = Some(Instant::now() + RETRY);
                    continue;
                }
            }
        }

        let Some(run) = running.as_mut() else { continue };
        let deadline = Instant::now() + TICK;
        let mut closed = false;

        while let Some(left) = deadline.checked_duration_since(Instant::now()).filter(|left| !left.is_zero()) {
            match run.spawned.next(left) {
                Next::Line(Line::Out(text)) => match &run.columns {
                    Some(columns) => {
                        if let Some(frame) = columns.parse(&text) {
                            count.take(frame, Instant::now());
                        }
                    }
                    None if text.starts_with("Application,") => {
                        run.columns = Columns::from_header(&text);
                        if run.columns.is_none() {
                            eprintln!("[fps] заголовок PresentMon не разобран: {text}");
                        }
                    }
                    None => {}
                },
                Next::Line(Line::Err(text)) => {
                    let text = text.trim();
                    if !text.is_empty() {
                        run.problem = Some(text.to_owned());
                    }
                }
                Next::Idle => break,
                Next::Closed => {
                    closed = true;
                    break;
                }
            }
        }

        if closed {
            let problem = running.take().and_then(|run| {
                let problem = run.problem.clone();
                stop(run.spawned);
                problem
            });
            eprintln!("[fps] PresentMon завершился: {}", problem.as_deref().unwrap_or("без объяснений"));
            failed = problem;
            retry_at = Some(Instant::now() + RETRY);
            count = Count::default();
            continue;
        }

        publish(&app, count.tick(Instant::now()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(entries: &[(u32, &str, u64)], now: Instant) -> HashMap<u32, Seen> {
        entries
            .iter()
            .map(|(pid, name, ago)| (*pid, Seen { name: (*name).to_owned(), last: now - Duration::from_millis(*ago) }))
            .collect()
    }

    #[test]
    fn the_foreground_game_becomes_the_target() {
        let now = Instant::now();
        let map = seen(&[(10, "game.exe", 100)], now);
        assert_eq!(choose(None, Some(10), &map, now, 1), Some(10));
    }

    #[test]
    fn clicking_the_widget_keeps_the_game() {
        let now = Instant::now();
        let map = seen(&[(10, "game.exe", 100)], now);
        // Передний план ушёл к нам — цель та же
        assert_eq!(choose(Some(10), Some(1), &map, now, 1), Some(10));
        // И к процессу без кадров — тоже
        assert_eq!(choose(Some(10), Some(77), &map, now, 1), Some(10));
    }

    #[test]
    fn browsers_and_the_shell_never_win() {
        let now = Instant::now();
        let map = seen(&[(10, "game.exe", 100), (20, "chrome.exe", 10), (30, "Explorer.EXE", 10)], now);
        assert_eq!(choose(Some(10), Some(20), &map, now, 1), Some(10));
        assert_eq!(choose(Some(10), Some(30), &map, now, 1), Some(10));
    }

    #[test]
    fn a_stale_window_does_not_steal_the_target() {
        let now = Instant::now();
        let map = seen(&[(10, "game.exe", 100), (40, "old.exe", 10_000)], now);
        assert_eq!(choose(Some(10), Some(40), &map, now, 1), Some(10));
    }

    #[test]
    fn another_game_in_front_takes_over() {
        let now = Instant::now();
        let map = seen(&[(10, "game.exe", 100), (50, "other.exe", 50)], now);
        assert_eq!(choose(Some(10), Some(50), &map, now, 1), Some(50));
    }

    fn frame(pid: u32, chain: &str, ms: f32) -> Frame {
        Frame {
            app: "game.exe".to_owned(),
            pid,
            chain: chain.to_owned(),
            ms,
            displayed: true,
            gpu_busy: None,
            latency: None,
            mode: None,
        }
    }

    /// Первая пачка новой цели идёт в счёт, а не теряется до следующей.
    #[test]
    fn the_first_batch_of_a_new_target_counts() {
        let mut count = Count { target: None, ..Count::default() };
        let now = Instant::now();
        for _ in 0..50 {
            count.take(frame(10, "0x1", 10.0), now);
        }
        count.target = Some(10);
        count.feed();
        assert!(count.tracker.close_tick().fps.is_some());
    }

    /// Вспомогательная цепочка не смешивается с основной.
    #[test]
    fn a_second_swap_chain_does_not_blend_in() {
        let mut count = Count { target: Some(10), ..Count::default() };
        let now = Instant::now();
        for _ in 0..50 {
            count.take(frame(10, "0xA", 10.0), now);
            count.take(frame(10, "0xA", 10.0), now);
            count.take(frame(10, "0xB", 20.0), now);
        }
        count.feed();
        let fps = count.tracker.close_tick().fps.unwrap();
        assert!((fps - 100.0).abs() < 0.1, "{fps}");
    }

    #[test]
    fn names_lose_the_extension() {
        assert_eq!(shorten("Cyberpunk2077.exe"), "Cyberpunk2077");
        assert_eq!(shorten("game"), "game");
    }

    #[test]
    fn the_view_arrives_flat_in_camel_case() {
        let view = View { status: Status::NoTool, ..View::default() };
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(json["status"], "noTool");
        assert!(json.get("low1").is_some(), "{json}");
        assert!(json.get("frameMs").is_some(), "{json}");
    }
}
