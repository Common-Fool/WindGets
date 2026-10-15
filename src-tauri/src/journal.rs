//! Журнал ошибок: stderr приложения — в файл с временем у каждой строки.
//!
//! В собранном приложении консоли нет (`windows_subsystem = "windows"`), и
//! больше сотни `eprintln!` по всему коду уходили в никуда: по жалобе
//! человека нечего было бы открыть. Переписывать их на свой макрос не нужно —
//! стандартная библиотека на Windows спрашивает дескриптор stderr
//! (`GetStdHandle`) при каждой записи. Достаточно один раз подменить его
//! трубой: поток журнала читает её построчно, ставит местное время и пишет в
//! файл. Туда же попадают паники Rust и всё, что пишет сайдкар датчиков, — он
//! наследует stderr (`sensors.rs`).
//!
//! Только в релизе. В разработке вывод остаётся в терминале запуска, где его
//! и читают.
//!
//! Файл — `app_log_dir()`, то есть `%LOCALAPPDATA%\<id>\logs`: на машине, а
//! не в перемещаемом профиле, и под галочку «Удалить данные» установщика он
//! попадает вместе с остальным. Никуда сам не уходит: телеметрии в приложении
//! нет и не будет — журнал человек прикладывает к жалобе сам, кнопкой
//! «Открыть журнал» в разделе «Данные».

// В отладочной сборке журнал не ведётся, и всё, кроме `dir` и тестов, в ней
// не вызывается — без этого каждая сборка разработки ругалась бы на мёртвый код
#![cfg_attr(debug_assertions, allow(dead_code))]

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

/// Нынешний журнал.
const FILE: &str = "windgets.log";

/// Прошлый: один, без цепочки — для жалобы хватает последних двух мегабайт.
const OLD: &str = "windgets.old.log";

/// Больше этого журнал переезжает в `OLD`. Мегабайт — это тысячи строк, то
/// есть недели обычной работы; без предела файл рос бы всю жизнь приложения.
const LIMIT: u64 = 1024 * 1024;

/// Размер сверяем не на каждой строке: раз в сотню хватает, а стоит это
/// обращения к диску.
const CHECK_EVERY: u32 = 100;

/// Каталог журнала. Его же открывает кнопка в разделе «Данные».
pub fn dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_log_dir().ok()
}

/// Строка журнала: время, два пробела, текст.
fn line(stamp: &str, text: &str) -> String {
    format!("{stamp}  {text}\n")
}

/// Время в виде `ГГГГ-ММ-ДД ЧЧ:ММ:СС` — сортируется как строка.
fn stamp(year: u16, month: u16, day: u16, hour: u16, minute: u16, second: u16) -> String {
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
}

/// Нынешнее местное время: в жалобе человек называет его по своим часам.
#[cfg(windows)]
fn now() -> String {
    // SAFETY: GetLocalTime только заполняет структуру и не может отказать
    let time = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    stamp(time.wYear, time.wMonth, time.wDay, time.wHour, time.wMinute, time.wSecond)
}

#[cfg(not(windows))]
fn now() -> String {
    String::new()
}

/// Перекладывает разросшийся журнал в `OLD`. Прежний `OLD` пропадает.
fn rotate(dir: &Path) -> io::Result<()> {
    let path = dir.join(FILE);
    match fs::metadata(&path) {
        Ok(meta) if meta.len() > LIMIT => {
            let old = dir.join(OLD);
            let _ = fs::remove_file(&old);
            fs::rename(&path, &old)
        }
        _ => Ok(()),
    }
}

/// Открывает журнал на дописывание, переложив разросшийся.
fn open(dir: &Path) -> io::Result<File> {
    fs::create_dir_all(dir)?;
    rotate(dir)?;
    OpenOptions::new().create(true).append(true).open(dir.join(FILE))
}

/// Читает трубу построчно до её закрытия и пишет строки в журнал.
///
/// Файл сбрасывается после каждой строки: падение приложения не должно
/// унести с собой ровно ту строку, ради которой журнал и открывают.
/// Разросшийся файл закрывается и перекладывается: открытый Windows
/// переименовать не даст.
fn pump(source: impl Read, dir: &Path, clock: impl Fn() -> String) {
    let mut reader = BufReader::new(source);
    let mut file = open(dir).ok();
    let mut buffer = Vec::new();
    let mut written = 0u32;

    loop {
        buffer.clear();
        match reader.read_until(b'\n', &mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let text = String::from_utf8_lossy(&buffer);
        let text = text.trim_end_matches(['\r', '\n']);

        if let Some(out) = file.as_mut() {
            let _ = out.write_all(line(&clock(), text).as_bytes());
            let _ = out.flush();
        }

        written += 1;
        if written.is_multiple_of(CHECK_EVERY) {
            let grown = file
                .as_ref()
                .and_then(|out| out.metadata().ok())
                .is_some_and(|meta| meta.len() > LIMIT);
            if grown {
                // Открытый файл Windows переименовать не даст — сперва закрыть
                drop(file.take());
                file = open(dir).ok();
            }
        }
    }
}

/// Подменяет stderr трубой в журнал. Звать первым делом в `setup`.
///
/// Не вышло — молча: сказать об этом некуда, а приложение без журнала
/// работает так же, как работало до него.
#[cfg(all(windows, not(debug_assertions)))]
pub fn start(app: &AppHandle) {
    use std::os::windows::io::IntoRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Console::{SetStdHandle, STD_ERROR_HANDLE};

    let Some(dir) = dir(app) else {
        return;
    };
    let Ok((reader, writer)) = io::pipe() else {
        return;
    };
    // Пишущий конец живёт до выхода: им отныне и будет stderr процесса
    let handle = writer.into_raw_handle();
    // SAFETY: дескриптор только что получен от `io::pipe` и больше никому не
    // принадлежит; закрывать его не будет никто — он нужен до выхода
    if unsafe { SetStdHandle(STD_ERROR_HANDLE, HANDLE(handle)) }.is_err() {
        return;
    }

    let spawned = std::thread::Builder::new()
        .name("journal".into())
        .spawn(move || pump(reader, &dir, now));
    if spawned.is_ok() {
        eprintln!("— запуск Windgets {} —", app.package_info().version);
    }
}

/// В разработке журнал не ведётся: вывод читают в терминале запуска.
#[cfg(not(all(windows, not(debug_assertions))))]
pub fn start(_app: &AppHandle) {}

/// Сколько знаков отказа окна кладём в журнал: стек React бывает длинным, а
/// петля отказов не должна раздуть файл до переезда в `.old` за минуту.
const CRASH_LIMIT: usize = 4000;

/// Отказ интерфейса окна (граница `Guard` во фронте): в собранном приложении
/// консоли webview никто не видит, и без этого причина пустого окна пропала бы.
#[tauri::command]
pub fn report_crash(window: tauri::WebviewWindow, message: String, stack: String) {
    let clip = |text: &str| text.chars().take(CRASH_LIMIT).collect::<String>();
    eprintln!("[окно {}] интерфейс упал: {}
{}", window.label(), clip(&message), clip(&stack));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Каталог под тест: свой у каждого, чистый перед началом.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("windgets-tests").join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("каталог под тест создаётся");
        dir
    }

    #[test]
    fn a_line_carries_its_time_and_text() {
        assert_eq!(stamp(2026, 10, 7, 9, 5, 3), "2026-10-07 09:05:03");
        assert_eq!(line("2026-10-07 09:05:03", "сайдкар упал"), "2026-10-07 09:05:03  сайдкар упал\n");
    }

    #[test]
    fn the_pipe_becomes_timed_lines() {
        let dir = scratch("journal-lines");
        pump("первая\r\nвторая\nбез перевода в конце".as_bytes(), &dir, || "T".to_owned());
        let text = fs::read_to_string(dir.join(FILE)).expect("журнал написан");
        assert_eq!(text, "T  первая\nT  вторая\nT  без перевода в конце\n");
    }

    #[test]
    fn the_journal_appends_across_launches() {
        let dir = scratch("journal-append");
        pump("до перезапуска\n".as_bytes(), &dir, || "T".to_owned());
        pump("после\n".as_bytes(), &dir, || "T".to_owned());
        let text = fs::read_to_string(dir.join(FILE)).expect("журнал написан");
        assert_eq!(text, "T  до перезапуска\nT  после\n");
    }

    #[test]
    fn a_grown_journal_moves_aside_at_launch() {
        let dir = scratch("journal-rotate");
        fs::write(dir.join(FILE), vec![b'x'; LIMIT as usize + 1]).expect("большой журнал");
        fs::write(dir.join(OLD), "самый старый").expect("прошлый журнал");
        pump("новая\n".as_bytes(), &dir, || "T".to_owned());
        assert_eq!(fs::metadata(dir.join(OLD)).expect("прошлый есть").len(), LIMIT + 1);
        assert_eq!(fs::read_to_string(dir.join(FILE)).expect("новый есть"), "T  новая\n");
    }

    #[test]
    fn a_journal_growing_mid_session_moves_aside_too() {
        let dir = scratch("journal-rotate-live");
        fs::write(dir.join(FILE), vec![b'x'; LIMIT as usize - 10]).expect("почти полный журнал");
        let input: String = (0..CHECK_EVERY).map(|at| format!("строка {at}\n")).collect();
        pump(input.as_bytes(), &dir, || "T".to_owned());
        assert!(dir.join(OLD).exists(), "разросшийся журнал переложен");
        assert!(fs::metadata(dir.join(FILE)).map(|meta| meta.len()).unwrap_or(0) < LIMIT);
    }

    /// Держит главное допущение модуля: `eprintln!` спрашивает дескриптор
    /// stderr при каждой записи, и подменённый `SetStdHandle` его ловит.
    ///
    /// Запускать отдельно: `cargo test stderr_reaches_the_journal -- --ignored
    /// --nocapture`. Без `--nocapture` тестовый прогон перехватывает
    /// `eprintln!` сам и до дескриптора дело не доходит; а подмена stderr
    /// задела бы соседние тесты, идущие параллельно.
    #[cfg(windows)]
    #[test]
    #[ignore = "меняет stderr всего процесса тестов; запускать с --nocapture"]
    fn stderr_reaches_the_journal() {
        use std::os::windows::io::IntoRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::Console::{GetStdHandle, SetStdHandle, STD_ERROR_HANDLE};

        let (mut reader, writer) = io::pipe().expect("труба");
        let raw = writer.into_raw_handle();
        // SAFETY: возвращаем прежний дескриптор до конца теста
        let before = unsafe { GetStdHandle(STD_ERROR_HANDLE) }.expect("прежний stderr");
        unsafe { SetStdHandle(STD_ERROR_HANDLE, HANDLE(raw)) }.expect("подмена");
        eprintln!("проверка журнала");
        unsafe { SetStdHandle(STD_ERROR_HANDLE, before) }.expect("возврат");
        // SAFETY: дескриптор больше нигде не используется
        drop(unsafe { <std::fs::File as std::os::windows::io::FromRawHandle>::from_raw_handle(raw) });

        let mut text = String::new();
        reader.read_to_string(&mut text).expect("труба читается");
        assert_eq!(text, "проверка журнала\n");
    }
}
