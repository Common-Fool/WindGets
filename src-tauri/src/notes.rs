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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// Имя файла книги заметок в каталоге настроек.
const NOTES_FILE: &str = "notes.json";

/// Сеанс чтения различает пересозданные окна, а ревизия приходит из окна:
/// порядок снимков не зависит от планирования IPC и рабочих потоков.
#[derive(Default)]
struct Saves {
    issued: AtomicU64,
    written: Mutex<(u64, u64)>,
}

impl Saves {
    fn issue(&self) -> u64 {
        self.issued.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn save(&self, revision: (u64, u64), write: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        let mut written = self.written.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if revision <= *written {
            return Ok(());
        }
        write()?;
        // Отказ не делает снимок сохранённым: его разрешено повторить.
        *written = revision;
        Ok(())
    }
}

static SAVES: Saves = Saves {
    issued: AtomicU64::new(0),
    written: Mutex::new((0, 0)),
};

/// Книга заметок: несколько страниц текста и индекс активной страницы.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Book {
    pub pages: Vec<String>,
    #[serde(default)]
    pub active: usize,
}

/// Номер сеанса отдаём окну отдельно от книги: на диск он не попадает.
#[derive(Serialize)]
pub struct Snapshot {
    #[serde(flatten)]
    book: Book,
    session: u64,
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
    // Общей дорогой (`disk::read_existing`): занятый антивирусом файл пробуем
    // ещё раз, а метку порядка байт от Блокнота снимаем — иначе карточка
    // запиралась бы до пересборки окна над целым файлом
    let text = match crate::disk::read_existing(path) {
        Ok(Some(text)) => text,
        Ok(None) => {
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
///
/// Async и в пул, как и запись: чтение с повторами и запись с доводкой до
/// диска умеют ждать, а синхронная команда стоит в обработчике IPC всех окон.
#[tauri::command]
pub async fn notes_pages(app: AppHandle) -> Result<Snapshot, String> {
    let session = SAVES.issue();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        // Новое окно не читает промежуточный снимок во время замены файла.
        let _written = SAVES.written.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        notes_path(&app).and_then(|path| read(&path)).map(|book| Snapshot { book, session })
    })
        .await
        .map_err(|error| format!("чтение заметок сорвалось: {error}"))
        .and_then(|outcome| outcome);

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
pub async fn set_notes_pages(app: AppHandle, book: Book, session: u64, revision: u64) -> Result<(), String> {
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        SAVES.save((session, revision), || notes_path(&app).and_then(|path| write(&path, &book)))
    })
        .await
        .map_err(|error| format!("запись заметок сорвалась: {error}"))
        .and_then(|outcome| outcome);

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

    /// Пул исполнил новый запрос первым: запоздалый уже не пишет старый файл.
    #[test]
    fn an_older_worker_cannot_overwrite_a_newer_save() {
        let path = scratch("notes-revision-order.json");
        let saves = Saves::default();
        let session = saves.issue();
        let old = (session, 1);
        let new = (session, 2);
        let expected = Book { pages: vec!["Свежий текст".to_owned()], active: 0 };
        saves.save(new, || write(&path, &expected)).expect("новый снимок записан");
        saves.save(old, || panic!("старый снимок не должен дойти до записи"))
            .expect("устаревший запрос пропущен");
        assert_eq!(read(&path).unwrap(), expected);
        fs::remove_file(path).unwrap();
    }

    /// Пересозданное окно начинает ревизии с единицы, но оно новее прежнего.
    #[test]
    fn a_new_window_session_outweighs_old_revision_numbers() {
        let saves = Saves::default();
        let old = saves.issue();
        let new = saves.issue();
        saves.save((old, 99), || Ok(())).unwrap();
        let mut new_written = false;
        saves.save((new, 1), || { new_written = true; Ok(()) }).unwrap();
        assert!(new_written);
        saves.save((old, 100), || panic!("старое окно не перетирает новое")).unwrap();
    }

    /// Номер сеанса есть только в ответе, а формат файла остался прежним.
    #[test]
    fn session_is_returned_beside_the_book_but_not_stored_in_it() {
        let book = Book { pages: vec!["Текст".to_owned()], active: 0 };
        let response = serde_json::to_value(Snapshot { book: book.clone(), session: 7 }).unwrap();
        assert_eq!(response["session"], 7);
        assert!(response["pages"].is_array());
        assert!(serde_json::to_value(book).unwrap().get("session").is_none());
    }

    /// Неудачная запись не занимает ревизию и не запирает повтор.
    #[test]
    fn a_failed_save_can_be_retried() {
        let saves = Saves::default();
        let revision = (saves.issue(), 1);
        assert!(saves.save(revision, || Err("диск занят".to_owned())).is_err());
        let mut retried = false;
        saves.save(revision, || { retried = true; Ok(()) }).unwrap();
        assert!(retried);
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
