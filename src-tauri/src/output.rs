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
#[tauri::command]
pub fn open_output_dir(app: AppHandle, module: String) -> Result<(), String> {
    let module = Module::from_id(&module).ok_or_else(|| "unknown module".to_owned())?;
    let (Some(dir), _) = dir_for(&app, module) else {
        return Err("no folder".to_owned());
    };
    // Папка появляется с первым готовым файлом, а открыть её хотят и до него
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    crate::links::launch(&dir.to_string_lossy());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
