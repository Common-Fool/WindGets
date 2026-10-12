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

/// Живой сайдкар со своими потоками ввода-вывода.
pub struct Sensors {
    child: Child,
    stdin: ChildStdin,
    /// Ответы от потока-читателя. Оборванный канал означает, что сайдкар
    /// закрыл поток вывода, то есть кончился.
    lines: Receiver<String>,
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

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| tr!("source.sidecarNoPipe", pipe = "stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| tr!("source.sidecarNoPipe", pipe = "stdout"))?;

        Ok(Sensors {
            child,
            stdin,
            lines: read_lines(stdout),
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
        self.stdin
            .write_all(b"poll\n")
            .and_then(|()| self.stdin.flush())
            .map_err(|error| tr!("source.sidecarDeaf", error = error))?;

        let line = answer(&self.lines, ANSWER_TIMEOUT)?;

        let root: Node = serde_json::from_str(&line)
            .map_err(|error| tr!("source.sidecarGarbled", error = error))?;
        Ok(flatten(&root))
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
fn answer(lines: &Receiver<String>, wait: Duration) -> Result<String, String> {
    lines.recv_timeout(wait).map_err(|error| match error {
        RecvTimeoutError::Timeout => {
            tr!("source.sidecarSilent", seconds = wait.as_secs())
        }
        RecvTimeoutError::Disconnected => tr!("source.sidecarClosed"),
    })
}

impl Drop for Sensors {
    fn drop(&mut self) {
        // Закрытый stdin сайдкар понимает как «приложение ушло» и выходит сам,
        // но ждать его тут некогда — добиваем
        let _ = self.child.kill();
        let _ = self.child.wait();
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

        assert_eq!(answer(&receiver, SOON).as_deref(), Ok("{}\n"));
    }

    /// Молчащий сайдкар — это отказ, а не повод ждать вечно. Ровно из-за
    /// отсутствия этого срока опрос однажды вставал бы навсегда, а виджеты
    /// показывали бы последний снимок как живой.
    #[test]
    fn silence_ends_as_a_refusal() {
        // Отправитель жив — канал не оборван, ответа просто нет
        let (_sender, receiver) = mpsc::channel::<String>();

        let refusal = answer(&receiver, SOON).expect_err("молчание — отказ");
        assert!(refusal.contains("не ответил"), "{refusal}");
    }

    /// Умерший сайдкар отличается от молчащего: разговор оборван, и причина
    /// в логе должна быть своя.
    #[test]
    fn a_dead_sidecar_is_told_apart_from_a_silent_one() {
        let (sender, receiver) = mpsc::channel::<String>();
        drop(sender);

        let refusal = answer(&receiver, SOON).expect_err("оборванный канал — отказ");
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
}
