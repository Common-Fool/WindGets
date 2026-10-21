//! Чужие программы конвертера и загрузчика: ffmpeg, yt-dlp, tar.
//!
//! Запускаются одинаково: без консольного окна, с пониженным приоритетом и в
//! объекте задания. Их вывод — оба потока, stdout и stderr, — приходит строками
//! в один канал, помеченный, откуда строка: yt-dlp пишет прогресс в один поток,
//! а шаги обработки в другой, и читать только один значило бы терять половину.
//!
//! Ждать процесс из потока команды нельзя (синхронная команда Tauri выполняется
//! в главном потоке — и ожидание в нём держит всё приложение), поэтому всё здесь зовётся из рабочих
//! потоков виджетов.

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

#[cfg(windows)]
pub(crate) mod job;

/// Окна у консольной программы быть не должно: иначе на каждом запуске поверх
/// обоев моргал бы чёрный прямоугольник.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Кодирование видео съедает все ядра, а рядом крутятся сайдкар телеметрии и
/// сам рабочий стол. Пониженный приоритет отдаёт им процессор первыми: видео
/// кодируется чуть дольше, зато графики не замирают.
#[cfg(windows)]
const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;

/// Строка вывода и поток, откуда она пришла.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Out(String),
    Err(String),
}

impl Line {
    pub fn text(&self) -> &str {
        match self {
            Line::Out(text) | Line::Err(text) => text,
        }
    }
}

/// Что принесло ожидание следующей строки.
pub enum Next {
    Line(Line),
    /// Срок вышел, а процесс ещё работает: самое время проверить отмену.
    Idle,
    /// Оба потока закрыты — процесс кончился.
    Closed,
}

/// Работающая программа со своим объектом задания.
pub struct Spawned {
    child: Child,
    #[cfg(windows)]
    job: Option<job::Job>,
    lines: Receiver<Line>,
}

/// Запускает подготовленную команду.
///
/// Потоки ввода-вывода, окно и приоритет выставляются здесь, а не у зовущего:
/// так ни один запуск не забудет ни `CREATE_NO_WINDOW`, ни задания.
pub fn spawn(mut command: Command) -> Result<Spawned, String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS | job::CREATE_SUSPENDED);
    }

    let mut child = command.spawn().map_err(|error| error.to_string())?;

    // Без задания отмена погасит только сам процесс, а не его потомков, — это
    // хуже, но не повод отказываться от работы
    #[cfg(windows)]
    let job = match job::Job::adopt(&child) {
        Ok(job) => Some(job),
        Err(error) => {
            eprintln!("объект задания не завёлся: {error}");
            None
        }
    };

    // Процесс спит с самого запуска: будим его, когда задание уже на месте
    #[cfg(windows)]
    if !job::resume(&child) {
        let _ = child.kill();
        let _ = child.wait();
        return Err("процесс не проснулся после запуска".to_owned());
    }

    let (sender, lines) = mpsc::channel();
    if let Some(stdout) = child.stdout.take() {
        pump(stdout, sender.clone(), Line::Out);
    }
    if let Some(stderr) = child.stderr.take() {
        pump(stderr, sender, Line::Err);
    }

    Ok(Spawned {
        child,
        #[cfg(windows)]
        job,
        lines,
    })
}

/// Читает поток строками в своём потоке ОС: только так у блокирующего чтения
/// появляется срок. Кончается сам, когда программа закрывает поток.
///
/// Байты, а не `read_line`: имя файла с кириллицей в чужой кодировке или
/// оборванный посреди буквы вывод не должны рвать чтение целиком.
fn pump<R: Read + Send + 'static>(stream: R, sender: Sender<Line>, wrap: fn(String) -> Line) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        let mut buffer = Vec::new();

        loop {
            buffer.clear();
            match reader.read_until(b'\n', &mut buffer) {
                Ok(0) | Err(_) => return,
                Ok(_) => {
                    let text = String::from_utf8_lossy(&buffer)
                        .trim_end_matches(['\r', '\n'])
                        .to_owned();
                    // Некому слушать — значит, `Spawned` уже выбросили
                    if sender.send(wrap(text)).is_err() {
                        return;
                    }
                }
            }
        }
    });
}

impl Spawned {
    /// Следующая строка, но не дольше `wait`.
    pub fn next(&self, wait: Duration) -> Next {
        match self.lines.recv_timeout(wait) {
            Ok(line) => Next::Line(line),
            Err(RecvTimeoutError::Timeout) => Next::Idle,
            Err(RecvTimeoutError::Disconnected) => Next::Closed,
        }
    }

    /// Гасит программу вместе со всеми её потомками.
    pub fn kill(&mut self) {
        #[cfg(windows)]
        if let Some(job) = &self.job {
            job.terminate();
        }
        let _ = self.child.kill();
    }

    /// Проверяет конец без ожидания: выход приложения не должен ждать процесс.
    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }

    /// Ждёт конца и отдаёт код выхода. `None` — убита или код не узнать.
    pub fn wait(&mut self) -> Option<i32> {
        self.child.wait().ok().and_then(|status| status.code())
    }
}

impl Drop for Spawned {
    fn drop(&mut self) {
        // Уже кончившуюся программу это не трогает, а брошенную посреди работы
        // добивает: иначе ffmpeg дописывал бы файл, который уже никому не нужен
        self.kill();
        let _ = self.child.wait();
    }
}

/// Читает всю программу до конца и отдаёт её строки с кодом выхода.
///
/// Для коротких вызовов: `ffmpeg -i` на один файл, `tar -tf`. Длинную работу с
/// прогрессом и отменой так не водят — там нужен [`Spawned::next`].
pub fn run(command: Command, limit: Duration) -> Result<(Option<i32>, Vec<Line>), String> {
    let mut spawned = spawn(command)?;
    let started = std::time::Instant::now();
    let mut lines = Vec::new();

    loop {
        let left = limit.saturating_sub(started.elapsed());
        if left.is_zero() {
            spawned.kill();
            return Err("timed out".to_owned());
        }
        match spawned.next(left.min(Duration::from_millis(200))) {
            Next::Line(line) => lines.push(line),
            Next::Idle => {}
            Next::Closed => break,
        }
    }

    Ok((spawned.wait(), lines))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// Программа стартует спящей и будится уже в задании: проснуться она
    /// обязана, иначе ffmpeg и yt-dlp стояли бы вечно.
    #[test]
    fn a_program_wakes_up_inside_its_job() {
        let mut command = Command::new(crate::autostart::system_exe("System32\\cmd.exe"));
        command.args(["/C", "echo", "woken"]);
        let (code, lines) = run(command, Duration::from_secs(10)).expect("программа отработала");
        assert_eq!(code, Some(0));
        assert!(lines.iter().any(|line| line.text().trim() == "woken"), "{lines:?}");
    }
}
