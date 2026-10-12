//! Виджет заметок: книга страниц с текстом, сохраняемая на диске.
//!
//! Данных извне тут нет вовсе — ни датчиков, ни сети, ни опроса. Rust нужен
//! ровно затем, чтобы написанное дожило до следующего запуска: своего диска у
//! webview нет, а `localStorage` лежит в профиле WebView2, то есть в месте, о
//! котором пользователь не знает и которое уходит вместе с кешем.
//!
//! Заметки хранятся книгой страниц (`Book`) в файле `notes.json`.
//!
//! ВАЖНО: **пропажа файла и отказ чтения — разные ответы.** Файла нет — заметок
//! ещё не вели, и пустая книга тут правда. Всё остальное — файл на месте, а
//! прочитать или разобрать его не вышло, — обязано дойти до карточки отказом:
//! она в этом случае запирает запись, чтобы не затереть написанное руками.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// Имя файла книги заметок в каталоге настроек.
const NOTES_FILE: &str = "notes.json";

/// Книга заметок: несколько страниц текста и индекс активной страницы.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Book {
    pub pages: Vec<String>,
    #[serde(default)]
    pub active: usize,
}

fn notes_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("не найден каталог настроек: {error}"))?;
    Ok(dir.join(NOTES_FILE))
}

/// Читает книгу заметок с диска.
///
/// Исходы:
/// 1) `notes.json` нет — пустая книга (ноль страниц);
/// 2) `notes.json` есть и разобран — книга `Book`;
/// 3) `notes.json` есть, но не читается или испорчен JSON — отказ (Err).
///
/// Указатель `active` проверяется на выход за границы списка страниц: если он
/// указывает мимо, приводим его к нулю, чтобы ручная правка файла не выглядела
/// как поломка.
fn read(path: &Path) -> Result<Book, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Book {
                pages: Vec::new(),
                active: 0,
            });
        }
        Err(error) => return Err(format!("не прочитать заметки: {error}")),
    };

    let mut book: Book = serde_json::from_str(&text)
        .map_err(|error| format!("не разобрать JSON заметок: {error}"))?;

    if book.active >= book.pages.len() {
        book.active = 0;
    }

    Ok(book)
}

/// Кладёт книгу заметок на диск через временного соседа.
///
/// Каталог создаём сами: до первого захода в настройки его может не быть
/// вовсе, а заметку пишут раньше — виджет включён с первого запуска.
fn write(path: &Path, book: &Book) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("не создать каталог настроек: {error}"))?;
    }

    let text = serde_json::to_string_pretty(book)
        .map_err(|error| format!("не сериализовать заметки: {error}"))?;

    // Через временного соседа: обрыв записи не должен оставлять испорченный файл
    crate::disk::write_atomically(path, &text)
        .map_err(|error| format!("не записать заметки: {error}"))
}

/// Отдаёт книгу заметок карточке.
///
/// Отказ уходит и в лог, и отказом команды: карточка по нему запирает запись.
#[tauri::command]
pub fn notes_pages(app: AppHandle) -> Result<Book, String> {
    let outcome = notes_path(&app).and_then(|path| read(&path));

    if let Err(error) = &outcome {
        eprintln!("заметки не прочитались: {error}");
    }

    outcome
}

/// Пишет книгу заметок и говорит, вышло ли.
///
/// Отказ уходит в две стороны сразу: в лог — разработчику, с причиной, и
/// отказом команды — карточке, которая покажет «Not saved».
#[tauri::command]
pub fn set_notes_pages(app: AppHandle, book: Book) -> Result<(), String> {
    let outcome = notes_path(&app).and_then(|path| write(&path, &book));

    if let Err(error) = &outcome {
        eprintln!("заметки не сохранились: {error}");
    }

    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Каталог под тесты этого файла: имена внутри свои у каждого теста.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("windgets-tests");
        fs::create_dir_all(&dir).expect("каталог под тесты создаётся");
        dir.join(name)
    }

    /// Пропажа файла даёт пустую книгу без ошибки.
    #[test]
    fn a_missing_file_reads_as_an_empty_book() {
        let path = scratch("notes-missing.json");
        let _ = fs::remove_file(&path);

        assert_eq!(
            read(&path).expect("пропажа файла — не отказ"),
            Book {
                pages: Vec::new(),
                active: 0,
            }
        );
    }

    /// Нечитаемый notes.json (например, каталог) — отказ, а не пустая книга.
    #[test]
    fn an_unreadable_file_is_a_refusal() {
        let path = scratch("notes-unreadable-json");
        fs::create_dir_all(&path).expect("каталог создаётся");

        assert!(read(&path).is_err());

        fs::remove_dir(&path).expect("прибирается");
    }

    /// Испорченный JSON заметок — отказ, чтобы не затереть написанное руками.
    #[test]
    fn corrupted_json_is_a_refusal() {
        let path = scratch("notes-corrupted.json");
        fs::write(&path, "{not valid json").expect("файл пишется");

        assert!(read(&path).is_err());

        fs::remove_file(&path).expect("прибирается");
    }

    /// Книга заметок переживает запись и чтение без искажений.
    #[test]
    fn book_round_trip() {
        let path = scratch("notes-round-trip.json");
        let book = Book {
            pages: vec![
                "Купить хлеб\n\n  и молоко  \n".to_owned(),
                "Вторая страница с «кавычками» и \\ слэшами".to_owned(),
            ],
            active: 1,
        };

        write(&path, &book).expect("книга пишется");
        assert_eq!(read(&path).expect("читается"), book);

        fs::remove_file(&path).expect("прибирается");
    }

    /// Индекс active за пределами списка страниц при чтении сбрасывается в 0.
    #[test]
    fn out_of_bounds_active_is_clamped_to_zero() {
        let path = scratch("notes-clamp-active.json");

        // active = 5 при 2 страницах
        let raw_json = r#"{"pages": ["Стр 1", "Стр 2"], "active": 5}"#;
        fs::write(&path, raw_json).expect("запись тестового json");

        let loaded = read(&path).expect("читается");
        assert_eq!(loaded.active, 0);
        assert_eq!(loaded.pages.len(), 2);

        fs::remove_file(&path).expect("прибирается");
    }

    /// Отсутствующее в JSON поле active по умолчанию читается как 0.
    #[test]
    fn missing_active_defaults_to_zero() {
        let path = scratch("notes-default-active.json");

        let raw_json = r#"{"pages": ["Стр 1", "Стр 2"]}"#;
        fs::write(&path, raw_json).expect("запись тестового json");

        let loaded = read(&path).expect("читается");
        assert_eq!(loaded.active, 0);
        assert_eq!(loaded.pages.len(), 2);

        fs::remove_file(&path).expect("прибирается");
    }

    /// Несуществующий каталог создаётся при первой записи.
    #[test]
    fn a_missing_directory_is_created_along_the_way() {
        let dir = scratch("notes-fresh-dir");
        let _ = fs::remove_dir_all(&dir);

        let path = dir.join(NOTES_FILE);
        let book = Book {
            pages: vec!["Первая страница".to_owned()],
            active: 0,
        };

        write(&path, &book).expect("каталог создаётся вместе с файлом");
        assert_eq!(read(&path).expect("читается"), book);

        fs::remove_dir_all(&dir).expect("прибирается");
    }
}
