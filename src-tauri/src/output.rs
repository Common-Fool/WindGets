//! Куда конвертер и загрузчик кладут готовое.
//!
//! По умолчанию — `Рабочий стол\WinGets\Converter` и `…\Downloads`. Рабочий
//! стол берётся у системы (known folder), поэтому стол, перенесённый на другой
//! диск или в OneDrive, находится сам. Своя папка — в настройках, полем
//! `output_dirs`; нет записи — умолчание, посчитанное заново при каждом
//! запросе, а не записанное однажды.
//!
//! Готовое никогда не затирает существующее: занято имя — берётся « (1)»,
//! « (2)» и дальше.

use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use tauri::{AppHandle, Manager, WebviewWindow};

use crate::settings::{self, SettingsState};
use crate::tools::Module;

/// Общая папка обоих модулей на рабочем столе.
const ROOT: &str = "WinGets";

/// Сколько чисел пробуем, прежде чем сдаться. Тысяча одноимённых файлов в
/// одной папке — это уже не случай, а поломка.
const ATTEMPTS: u32 = 1000;

/// Папка по умолчанию.
pub fn default_dir(app: &AppHandle, module: Module) -> Option<PathBuf> {
    let desktop = app.path().desktop_dir().ok()?;
    Some(desktop.join(ROOT).join(module.folder()))
}

/// Папка, которую модуль берёт сейчас, и своя ли она.
pub fn dir_for(app: &AppHandle, module: Module) -> (Option<PathBuf>, bool) {
    let own = app
        .state::<SettingsState>()
        .get()
        .output_dirs
        .get(module.id())
        .map(PathBuf::from);
    match own {
        Some(dir) => (Some(dir), true),
        None => (default_dir(app, module), false),
    }
}

/// Имя файла с номером: `clip.mp4`, `clip (1).mp4`, `clip (2).mp4`.
fn candidate(stem: &str, ext: &str, number: u32) -> String {
    if number == 0 {
        format!("{stem}.{ext}")
    } else {
        format!("{stem} ({number}).{ext}")
    }
}

/// Проверяет запись до долгой работы. Существующий каталог может быть только
/// для чтения: create_dir_all этого не замечает, а чужой файл проба не трогает.
pub fn probe_write(dir: &Path) -> io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::fs::create_dir_all(dir)?;
    for _ in 0..ATTEMPTS {
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!(".windgets-write-{}-{number}.tmp", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => {
                drop(file);
                return std::fs::remove_file(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "no free probe name"))
}

/// Занимает свободное имя в папке пустым файлом и отдаёт путь к нему.
///
/// ВАЖНО: **имя занимается `create_new`, а не проверкой «есть ли такой».**
/// Между проверкой и записью имя успел бы занять кто угодно, а переименование
/// на Windows существующий файл заменяет молча — и чужой файл пропал бы.
/// Созданный здесь пустой файл — наш, и заменять его готовым можно.
pub fn reserve(dir: &Path, stem: &str, ext: &str) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    for number in 0..ATTEMPTS {
        let path = dir.join(candidate(stem, ext, number));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "no free name"))
}

/// Занимает рабочего соседа для готового файла: `clip.mp4.part`, а занят —
/// `clip.mp4.1.part` и дальше. Тем же `create_new`, что и само имя: ffmpeg
/// пишет с `-y`, и чужой `.part` с тем же именем он затёр бы молча.
pub fn reserve_part(reserved: &Path) -> io::Result<PathBuf> {
    let name = reserved.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    for number in 0..ATTEMPTS {
        let part = match number {
            0 => reserved.with_file_name(format!("{name}.part")),
            _ => reserved.with_file_name(format!("{name}.{number}.part")),
        };
        match OpenOptions::new().write(true).create_new(true).open(&part) {
            Ok(_) => return Ok(part),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "no free name"))
}

/// Ставит готовый файл на занятое за ним имя.
///
/// Переименование — если оба на одном диске; иначе копия и удаление:
/// рабочая папка загрузчика живёт в профиле на системном диске, а папку вывода
/// могли перенести на другой.
pub fn place(from: &Path, reserved: &Path) -> io::Result<()> {
    if std::fs::rename(from, reserved).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, reserved)?;
    let _ = std::fs::remove_file(from);
    Ok(())
}

/// Настройки и обе карточки узнают о новой папке сразу.
fn announce(app: &AppHandle) {
    crate::settings_window::emit(app);
    crate::converter::publish(app);
    crate::downloader::publish(app);
}

/// Выбор своей папки — окном Windows поверх окна настроек.
#[tauri::command]
pub fn pick_output_dir(app: AppHandle, window: WebviewWindow, module: String) -> Result<(), String> {
    let module = Module::from_id(&module).ok_or_else(|| "unknown module".to_owned())?;
    let owner = window.hwnd().map(|hwnd| hwnd.0 as isize).unwrap_or(0);
    let (start, _) = dir_for(&app, module);

    crate::picker::pick(owner, crate::picker::Pick::Folder, start, move |chosen| {
        let Some(dir) = chosen.into_iter().next() else {
            return;
        };
        let state = app.state::<SettingsState>();
        settings::update(&app, &state, |settings| {
            settings
                .output_dirs
                .insert(module.id().to_owned(), dir.to_string_lossy().into_owned());
        });
        announce(&app);
    });
    Ok(())
}

/// Возврат к папке по умолчанию.
#[tauri::command]
pub fn reset_output_dir(app: AppHandle, module: String) -> Result<(), String> {
    let module = Module::from_id(&module).ok_or_else(|| "unknown module".to_owned())?;
    let state = app.state::<SettingsState>();
    settings::update(&app, &state, |settings| {
        settings.output_dirs.remove(module.id());
    });
    announce(&app);
    Ok(())
}

/// Открывает папку вывода в проводнике — отдельным процессом, не `opener`.
///
/// Async и в пул: своя папка бывает на сетевом диске, и создание её на
/// уснувшем сервере ждёт сеть — в главном потоке это стояло бы всё приложение.
#[tauri::command]
pub async fn open_output_dir(app: AppHandle, module: String) -> Result<(), String> {
    let module = Module::from_id(&module).ok_or_else(|| "unknown module".to_owned())?;
    let (Some(dir), _) = dir_for(&app, module) else {
        return Err("no folder".to_owned());
    };
    tauri::async_runtime::spawn_blocking(move || {
        // Папка появляется с первым готовым файлом, а открыть её хотят и до него
        std::fs::create_dir_all(&dir).map_err(|error| {
            // Карточка отказ не показывает (места нет), журнал — обязан
            eprintln!("папка вывода не открылась: {}: {error}", dir.display());
            error.to_string()
        })?;
        crate::links::launch(&dir.to_string_lossy());
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_probe_leaves_existing_files_and_no_temporary_file() {
        let dir = std::env::temp_dir().join(format!("hw-output-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let original = dir.join("clip.mp4");
        std::fs::write(&original, b"precious").unwrap();
        probe_write(&dir).unwrap();
        assert_eq!(std::fs::read(&original).unwrap(), b"precious");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        assert!(probe_write(&original).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn names_count_up_from_the_bare_one() {
        assert_eq!(candidate("clip", "mp4", 0), "clip.mp4");
        assert_eq!(candidate("clip", "mp4", 1), "clip (1).mp4");
        assert_eq!(candidate("Me at the zoo", "mp3", 2), "Me at the zoo (2).mp3");
    }

    /// Занятое имя не затирается: второй и третий файл получают номера, а
    /// первый остаётся нетронутым.
    #[test]
    fn a_taken_name_is_never_overwritten() {
        let dir = std::env::temp_dir().join(format!("hw-output-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("clip.mp4"), b"precious").unwrap();

        let first = reserve(&dir, "clip", "mp4").unwrap();
        let second = reserve(&dir, "clip", "mp4").unwrap();
        assert_eq!(first, dir.join("clip (1).mp4"));
        assert_eq!(second, dir.join("clip (2).mp4"));
        assert_eq!(std::fs::read(dir.join("clip.mp4")).unwrap(), b"precious");

        let part = dir.join("clip.part");
        std::fs::write(&part, b"done").unwrap();
        place(&part, &first).unwrap();
        assert_eq!(std::fs::read(&first).unwrap(), b"done");
        assert!(!part.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
