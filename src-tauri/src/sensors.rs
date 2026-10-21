//! Сайдкар телеметрии: дочерний процесс, читающий железо.
//!
//! Заменил внешний LibreHardwareMonitor, который приходилось запускать руками
//! от администратора. Формат провода у сайдкара тот же, что был у веб-сервера
//! LHM, поэтому разбор дерева, единицы и селекторы виджетов остались как есть —
//! см. [`crate::lhm`].
//!
//! Разговор простой: пишем `poll` в stdin, читаем строку из stdout. Такт задаёт
//! эта сторона, потому что шаг опроса живёт в настройках приложения.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use crate::lhm::{flatten, Flat, Node};

/// Имя бинаря сайдкара. В сборку он едет ресурсом, в разработке лежит рядом с
/// исходником на C#.
const SIDECAR: &str = "windgets-sensors.exe";

/// Путь сайдкара в дереве исходников — на случай запуска из `cargo`/`tauri dev`,
/// где каталога ресурсов ещё нет.
const IN_TREE: &str = "../src-sensors/bin/Release/windgets-sensors.exe";

/// Окна у консольного процесса быть не должно: иначе на каждом запуске
/// приложения мигал бы чёрный прямоугольник поверх обоев.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Сколько ждём ответа на `poll`.
///
/// ВАЖНО: **сайдкар умеет не отвечать вовсе.** Умерев, он закрывает поток, и
/// это приходит концом строки, — а вот застряв внутри `computer.Accept`, он не
/// шлёт ничего и поток не закрывает. Так бывает: LHM ходит в шину, в NVAPI и в
/// драйвер, и любой из них умеет не вернуться. Без срока опрос вставал бы там
/// навсегда, окна вечно показывали бы последний снимок, и «источник
/// недоступен» не появилось бы никогда — то есть ровно то состояние, которого
/// не должно быть по правилам проекта.
///
/// Заметно больше самого долгого шага опроса (пять секунд): срок отмеряет
/// «сайдкар не отвечает», а не «сайдкар не торопится».
const ANSWER_TIMEOUT: Duration = Duration::from_secs(10);

/// Срок первого ответа. До чтения stdin сайдкар открывает всё железо
/// (`computer.Open()`): SMART каждого диска, шины, драйвер. На машине с
/// десятком накопителей или спящим USB-диском это дольше обычного такта, и с
/// общим сроком сайдкар убивался бы на середине открытия — каждый раз заново.
const FIRST_ANSWER_TIMEOUT: Duration = Duration::from_secs(30);

/// Сколько ждём, пока убитый сайдкар уйдёт. Застрявший в драйвере ядра он
/// может не уйти вовсе. Одна попытка ограничена сроком, но Child остаётся
/// у опроса до подтверждённого выхода: установщику нельзя подтверждать
/// освобождение драйвера только на основании отправленного kill.
const EXIT_WAIT: Duration = Duration::from_secs(2);

/// Живой сайдкар со своими потоками ввода-вывода.
pub struct Sensors {
    child: Child,
    stdin: ChildStdin,
    /// Ответы от потока-читателя. Оборванный канал означает, что сайдкар
    /// закрыл поток вывода, то есть кончился.
    lines: Receiver<String>,
    /// Ответа ещё не было: срок у него свой ([`FIRST_ANSWER_TIMEOUT`]).
    first: bool,
    stopped: bool,
    /// Объект задания с `KILL_ON_JOB_CLOSE`: сайдкар уходит вместе с
    /// приложением, даже зависший — тогда закрытый stdin он не прочтёт, а поток
    /// опроса на выходе не доживает до `Drop`.
    #[cfg(windows)]
    _job: Option<crate::child::job::Job>,
}

impl Sensors {
    /// Поднимает сайдкар. Ошибка означает, что бинаря нет или он не запустился.
    pub fn start(exe: &Path) -> Result<Self, String> {
        let mut command = Command::new(exe);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // stderr оставляем родителю: жалобы сайдкара видно в логе приложения
            .stderr(Stdio::inherit());

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = command
            .spawn()
            .map_err(|error| tr!("source.sidecarNoStart", error = error))?;

        // Обе трубы запрошены через Stdio::piped: после удачного spawn они
        // обязаны быть. Нарушение этого условия — ошибка самого запуска;
        // опрос тогда не подтвердит остановку неучтённого процесса.
        let stdin = child.stdin.take().expect("сайдкар получил stdin");
        let stdout = child.stdout.take().expect("сайдкар получил stdout");

        // Своих потомков сайдкар не порождает, так что спящий запуск, как у
        // программ конвертера, ему не нужен
        #[cfg(windows)]
        let job = crate::child::job::Job::adopt(&child)
            .map_err(|error| eprintln!("сайдкар без объекта задания: {error}"))
            .ok();

        Ok(Sensors {
            child,
            stdin,
            lines: read_lines(stdout),
            first: true,
            stopped: false,
            #[cfg(windows)]
            _job: job,
        })
    }

    /// Просит снимок и разбирает ответ.
    ///
    /// Ответа ждём не дольше [`ANSWER_TIMEOUT`]. Просрочка — такой же отказ,
    /// как оборванный разговор: наверх уходит `Err`, а опрос от него избавится
    /// от процесса целиком и поднимет новый (см. [`crate::poller`]).
    ///
    /// Опоздавший ответ никого не путает именно поэтому: вместе с `Sensors`
    /// уходит и приёмник, так что второй раз его уже некому прочесть.
    pub fn poll(&mut self) -> Result<Flat, String> {
        if crate::poller::holding() {
            return Err(tr!("source.sidecarDown"));
        }
        self.stdin
            .write_all(b"poll\n")
            .and_then(|()| self.stdin.flush())
            .map_err(|error| tr!("source.sidecarDeaf", error = error))?;

        let wait = if self.first { FIRST_ANSWER_TIMEOUT } else { ANSWER_TIMEOUT };
        let line = answer(&self.lines, wait, crate::poller::holding)?;
        self.first = false;

        let root: Node = serde_json::from_str(&line)
            .map_err(|error| tr!("source.sidecarGarbled", error = error))?;
        Ok(flatten(&root))
    }

    /// Только подтверждённый выход освобождает драйвер для установщика.
    /// Не ушёл — опрос сохраняет Child и повторяет остановку, а не выдаёт
    /// отсутствие владельца процесса за отсутствие самого процесса.
    pub fn stop(&mut self) -> bool {
        if !self.stopped {
            let _ = self.child.kill();
            self.stopped = wait_for_exit(|| self.child.try_wait().map(|status| status.is_some()), EXIT_WAIT);
        }
        self.stopped
    }
}

/// Уносит чтение в свой поток: только так у блокирующего `read_line` появляется
/// срок. Поток кончается сам, когда сайдкар закрывает поток вывода, — а его
/// закрывает и обычный выход, и `kill` из [`Drop`].
///
/// Ждать этот поток не надо и негде: он никаких данных не держит, кроме своей
/// половины трубы.
fn read_lines(stdout: ChildStdout) -> Receiver<String> {
    let (sender, receiver) = mpsc::channel();

    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);

        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                // Конец потока — сайдкара больше нет
                Ok(0) | Err(_) => return,
                Ok(_) => {
                    // Некому слушать — значит, `Sensors` уже выбросили
                    if sender.send(line).is_err() {
                        return;
                    }
                }
            }
        }
    });

    receiver
}

/// Ответ сайдкара или причина, по которой его нет.
///
/// Отдельной функцией ради теста: обе беды — молчание и оборванный канал —
/// проверяются на голом канале, без живого процесса.
fn answer(lines: &Receiver<String>, wait: Duration, mut held: impl FnMut() -> bool) -> Result<String, String> {
    let until = std::time::Instant::now() + wait;
    loop {
        // Первый ответ ждём дольше обычного, но установщик не должен ждать
        // окончания computer.Open: удержание слышно не реже раза в 100 мс.
        if held() {
            return Err(tr!("source.sidecarDown"));
        }
        let left = until.saturating_duration_since(std::time::Instant::now());
        match lines.recv_timeout(left.min(Duration::from_millis(100))) {
            Ok(line) => return Ok(line),
            Err(RecvTimeoutError::Disconnected) => return Err(tr!("source.sidecarClosed")),
            Err(RecvTimeoutError::Timeout) if std::time::Instant::now() >= until => {
                return Err(tr!("source.sidecarSilent", seconds = wait.as_secs()));
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

fn wait_for_exit(mut exited: impl FnMut() -> std::io::Result<bool>, wait: Duration) -> bool {
    let until = std::time::Instant::now() + wait;
    loop {
        match exited() {
            Ok(true) => return true,
            Err(error) => {
                eprintln!("выход сайдкара не подтвердился: {error}");
                return false;
            }
            Ok(false) => {}
        }
        let left = until.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            eprintln!("сайдкар не ушёл за {} с после kill", wait.as_secs());
            return false;
        }
        std::thread::sleep(left.min(Duration::from_millis(50)));
    }
}

impl Drop for Sensors {
    fn drop(&mut self) {
        // На обычных путях опрос уже дождался stop. При панике остаётся
        // последняя попытка; разрешения установщику сам Drop не выдаёт.
        self.stop();
    }
}

/// Где искать бинарь сайдкара.
///
/// Сперва каталог ресурсов установленного приложения, потом дерево исходников:
/// в разработке ресурсов нет вовсе, а пересобирать установщик ради каждой
/// правки на C# незачем.
pub fn locate(resource_dir: Option<PathBuf>) -> Option<PathBuf> {
    let candidates = [resource_dir.map(|dir| dir.join(SIDECAR)), Some(IN_TREE.into())];

    candidates.into_iter().flatten().find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Срок для тестов: ждать настоящие десять секунд тут незачем.
    const SOON: Duration = Duration::from_millis(50);

    #[test]
    fn an_answer_comes_back_as_is() {
        let (sender, receiver) = mpsc::channel();
        sender.send("{}\n".to_owned()).expect("ответ уходит");

        assert_eq!(answer(&receiver, SOON, || false).as_deref(), Ok("{}\n"));
    }

    /// Молчащий сайдкар — это отказ, а не повод ждать вечно. Ровно из-за
    /// отсутствия этого срока опрос однажды вставал бы навсегда, а виджеты
    /// показывали бы последний снимок как живой.
    #[test]
    fn silence_ends_as_a_refusal() {
        // Отправитель жив — канал не оборван, ответа просто нет
        let (_sender, receiver) = mpsc::channel::<String>();

        let refusal = answer(&receiver, SOON, || false).expect_err("молчание — отказ");
        assert!(refusal.contains("не ответил"), "{refusal}");
    }

    /// Умерший сайдкар отличается от молчащего: разговор оборван, и причина
    /// в логе должна быть своя.
    #[test]
    fn a_dead_sidecar_is_told_apart_from_a_silent_one() {
        let (sender, receiver) = mpsc::channel::<String>();
        drop(sender);

        let refusal = answer(&receiver, SOON, || false).expect_err("оборванный канал — отказ");
        assert!(refusal.contains("закрылся"), "{refusal}");
    }

    /// Живой круг: поднять сайдкар, спросить снимок, получить показания.
    ///
    /// Без прав администратора часть железа не читается вовсе (накопители,
    /// плата, Tctl/Tdie), поэтому тест проверяет только сам разговор — а полноту
    /// выхлопа держат тесты в `poller`.
    #[test]
    #[ignore = "нужен собранный сайдкар: npm run sidecar"]
    fn talks_to_the_sidecar() {
        let exe = locate(None).expect("сайдкар собран");
        let mut sensors = Sensors::start(&exe).expect("сайдкар запускается");

        let first = sensors.poll().expect("первый снимок приходит").readings;
        assert!(
            first.len() > 50,
            "датчиков подозрительно мало: {}",
            first.len()
        );

        // Второй заход по тому же процессу: сайдкар обязан пережить свой ответ
        let second = sensors.poll().expect("второй снимок приходит").readings;
        assert_eq!(
            first.len(),
            second.len(),
            "между снимками изменился состав датчиков"
        );
    }

    #[test]
    fn the_first_answer_can_be_interrupted_by_hold() {
        let (_sender, receiver) = mpsc::channel::<String>();
        let asked = std::time::Instant::now();
        let mut checks = 0;
        let outcome = answer(&receiver, FIRST_ANSWER_TIMEOUT, || {
            checks += 1;
            checks > 1
        });
        assert!(outcome.is_err());
        assert!(checks > 1, "удержание проверяется во время ожидания");
        assert!(asked.elapsed() < Duration::from_secs(2), "не ждём первого ответа целиком");
    }

    #[test]
    fn a_lost_exit_status_does_not_confirm_a_stopped_process() {
        assert!(!wait_for_exit(|| Err(std::io::ErrorKind::PermissionDenied.into()), Duration::ZERO));
        assert!(!wait_for_exit(|| Ok(false), Duration::ZERO));
        assert!(wait_for_exit(|| Ok(true), Duration::ZERO));
    }
}
