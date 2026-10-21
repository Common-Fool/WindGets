//! Виджет списка дел: доска списков задач с галочками, сохраняемая на диске.
//!
//! Данных извне тут нет вовсе — ни датчиков, ни сети, ни опроса. Rust нужен
//! ровно затем, чтобы список дел дожил до следующего запуска.
//!
//! Дела хранятся доской списков (`Board`) в файле `todo.json`. Формат объекта
//! содержит массив списков `lists` и индекс активного списка `active`.
//! Для совместимости с прошлой версией, где в `todo.json` лежал голый массив
//! дел, разбор умеет прозрачно принимать старый формат (через `untagged` enum в serde)
//! и отдавать его как доску из одного списка.
//!
//! ВАЖНО: **пропажа файла и отказ чтения — разные ответы.** Файла нет — список
//! ещё не вели, и пустая доска тут правда. Всё остальное — файл на месте, а
//! прочитать или разобрать JSON не вышло, — обязано дойти до карточки
//! отказом: она в этом случае запирает запись, чтобы не затереть целый список
//! пустотой.
//!
//! Состояния в памяти виджет не держит: окно одно, читают список один раз при
//! его сборке, а дальше он живёт во фронте.

use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// Имя файла в каталоге настроек. Рядом с `settings.json` и `notes.json`.
const TODO_FILE: &str = "todo.json";

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

/// Отдельная задача с идентификатором, текстом и статусом выполнения.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: u64,
    pub text: String,
    pub done: bool,
}

/// Доска списков дел: несколько списков задач и индекс активного списка.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Board {
    pub lists: Vec<Vec<Task>>,
    #[serde(default)]
    pub active: usize,
}

/// Промежуточный тип для прозрачного разбора двух форматов файла todo.json:
/// нового формата (объект с полями `lists` и `active`) и старого формата
/// (голый массив `Vec<Task>`).
#[derive(Deserialize)]
#[serde(untagged)]
enum StoredBoard {
    Board(Board),
    Legacy(Vec<Task>),
}

/// Номер сеанса отдаём окну отдельно от книги: на диск он не попадает.
#[derive(Serialize)]
pub struct Snapshot {
    #[serde(flatten)]
    board: Board,
    session: u64,
}

fn todo_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("не найден каталог настроек: {error}"))?;
    Ok(dir.join(TODO_FILE))
}

/// Читает доску списков дел. Разница между пропажей и отказом — в шапке файла.
///
/// Испорченный JSON тут тоже отказ, а не пустая доска. У настроек правило
/// обратное — испорченный файл читается как значения по умолчанию, — но
/// настройки восстанавливаются одним заходом в окно, а список дел пишут руками
/// и восстановить его неоткуда.
///
/// Указатель `active` приводится к нулю, если указывает за пределы списков.
fn read(path: &Path) -> Result<Board, String> {
    // Общей дорогой, как у заметок: повтор занятого файла и снятая метка
    // порядка байт
    let text = match crate::disk::read_existing(path) {
        Ok(Some(text)) => text,
        Ok(None) => {
            return Ok(Board {
                lists: Vec::new(),
                active: 0,
            })
        }
        Err(error) => return Err(format!("не прочитать список дел: {error}")),
    };

    let stored: StoredBoard = serde_json::from_str(&text)
        .map_err(|error| format!("не разобрать JSON списка дел: {error}"))?;

    let mut board = match stored {
        StoredBoard::Board(board) => board,
        StoredBoard::Legacy(tasks) => Board {
            lists: vec![tasks],
            active: 0,
        },
    };

    if board.active >= board.lists.len() {
        board.active = 0;
    }

    Ok(board)
}

/// Кладёт доску списков дел на диск через временного соседа.
///
/// Каталог создаём сами: до первого захода в настройки его может не быть вовсе.
fn write(path: &Path, board: &Board) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("не создать каталог настроек: {error}"))?;
    }

    let text = serde_json::to_string_pretty(board)
        .map_err(|error| format!("не сериализовать список дел: {error}"))?;

    // Через временного соседа: обрыв записи не должен оставлять испорченный файл
    crate::disk::write_atomically(path, &text)
        .map_err(|error| format!("не записать список дел: {error}"))
}

/// Отдаёт доску списков дел карточке.
///
/// Отказ уходит и в лог, и отказом команды: карточка по нему запирает запись.
///
/// Async и в пул по той же причине, что у заметок.
#[tauri::command]
pub async fn todo_lists(app: AppHandle) -> Result<Snapshot, String> {
    let session = SAVES.issue();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        // Новое окно не читает промежуточный снимок во время замены файла.
        let _written = SAVES.written.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        todo_path(&app).and_then(|path| read(&path)).map(|board| Snapshot { board, session })
    })
        .await
        .map_err(|error| format!("чтение списка дел сорвалось: {error}"))
        .and_then(|outcome| outcome);

    if let Err(error) = &outcome {
        eprintln!("список дел не прочитался: {error}");
    }

    outcome
}

/// Пишет доску списков дел и сообщает результат.
///
/// Отказ уходит в две стороны: в лог с русской причиной для разработчика и
/// карточке отказом команды для показа «Not saved».
#[tauri::command]
pub async fn set_todo_lists(app: AppHandle, board: Board, session: u64, revision: u64) -> Result<(), String> {
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        SAVES.save((session, revision), || todo_path(&app).and_then(|path| write(&path, &board)))
    })
        .await
        .map_err(|error| format!("запись списка дел сорвалась: {error}"))
        .and_then(|outcome| outcome);

    if let Err(error) = &outcome {
        eprintln!("список дел не сохранился: {error}");
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
        let path = scratch("todo-revision-order.json");
        let saves = Saves::default();
        let session = saves.issue();
        let old = (session, 1);
        let new = (session, 2);
        let expected = Board { lists: vec![vec![Task { id: 1, text: "Свежее дело".to_owned(), done: false }]], active: 0 };
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
        let board = Board { lists: vec![Vec::new()], active: 0 };
        let response = serde_json::to_value(Snapshot { board: board.clone(), session: 7 }).unwrap();
        assert_eq!(response["session"], 7);
        assert!(response["lists"].is_array());
        assert!(serde_json::to_value(board).unwrap().get("session").is_none());
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

    /// Пропажа файла даёт пустую доску без ошибки.
    #[test]
    fn a_missing_file_reads_as_an_empty_board() {
        let path = scratch("todo-missing.json");
        let _ = fs::remove_file(&path);

        assert_eq!(
            read(&path).expect("пропажа файла — не отказ"),
            Board {
                lists: Vec::new(),
                active: 0,
            }
        );
    }

    /// Нечитаемое (например, каталог вместо файла) — отказ, а не пустая доска.
    /// Карточка по этому отказу запирает запись.
    #[test]
    fn an_unreadable_file_is_a_refusal() {
        let path = scratch("todo-unreadable");
        fs::create_dir_all(&path).expect("каталог создаётся");

        assert!(read(&path).is_err());

        fs::remove_dir(&path).expect("прибирается");
    }

    /// Испорченный JSON — отказ, а не пустая доска.
    #[test]
    fn corrupted_json_is_a_refusal() {
        let path = scratch("todo-corrupted.json");
        fs::write(&path, "{not valid json").expect("файл пишется");

        assert!(read(&path).is_err());

        fs::remove_file(&path).expect("прибирается");
    }

    /// Старый формат (голый массив дел) прозрачно читается как доска из одного списка.
    #[test]
    fn legacy_task_array_reads_as_a_single_list_board() {
        let path = scratch("todo-legacy-array.json");
        let legacy_json = r#"[
            {"id": 1, "text": "Старая задача 1", "done": false},
            {"id": 2, "text": "Старая задача 2", "done": true}
        ]"#;
        fs::write(&path, legacy_json).expect("запись старого json");

        let loaded = read(&path).expect("успешно читается старый массив");
        assert_eq!(
            loaded,
            Board {
                lists: vec![vec![
                    Task {
                        id: 1,
                        text: "Старая задача 1".to_owned(),
                        done: false,
                    },
                    Task {
                        id: 2,
                        text: "Старая задача 2".to_owned(),
                        done: true,
                    },
                ]],
                active: 0,
            }
        );

        fs::remove_file(&path).expect("прибирается");
    }

    /// Новый формат (объект с lists и active) читается как есть.
    #[test]
    fn new_board_format_reads_as_is() {
        let path = scratch("todo-new-format.json");
        let board = Board {
            lists: vec![
                vec![Task {
                    id: 1,
                    text: "Список 1 - Задача 1".to_owned(),
                    done: false,
                }],
                vec![Task {
                    id: 2,
                    text: "Список 2 - Задача 1".to_owned(),
                    done: true,
                }],
            ],
            active: 1,
        };

        write(&path, &board).expect("запись доски");
        assert_eq!(read(&path).expect("читается новый формат"), board);

        fs::remove_file(&path).expect("прибирается");
    }

    /// Доска списков дел переживает запись и чтение без искажений.
    #[test]
    fn board_round_trip() {
        let path = scratch("todo-round-trip.json");
        let board = Board {
            lists: vec![
                vec![
                    Task {
                        id: 1,
                        text: "Первая задача".to_owned(),
                        done: false,
                    },
                    Task {
                        id: 2,
                        text: "Вторая со спецсимволами: «quotes» & \\ slashes".to_owned(),
                        done: true,
                    },
                ],
                vec![Task {
                    id: 3,
                    text: "Второй список".to_owned(),
                    done: false,
                }],
            ],
            active: 1,
        };

        write(&path, &board).expect("список пишется");
        assert_eq!(read(&path).expect("читается"), board);

        fs::remove_file(&path).expect("прибирается");
    }

    /// Индекс active за пределами списка списков при чтении сбрасывается в 0.
    #[test]
    fn out_of_bounds_active_is_clamped_to_zero() {
        let path = scratch("todo-clamp-active.json");
        let raw_json = r#"{
            "lists": [
                [{"id": 1, "text": "Задача", "done": false}]
            ],
            "active": 5
        }"#;
        fs::write(&path, raw_json).expect("запись тестового json");

        let loaded = read(&path).expect("читается");
        assert_eq!(loaded.active, 0);
        assert_eq!(loaded.lists.len(), 1);

        fs::remove_file(&path).expect("прибирается");
    }

    /// Отсутствующее в JSON поле active по умолчанию читается как 0.
    #[test]
    fn missing_active_defaults_to_zero() {
        let path = scratch("todo-default-active.json");
        let raw_json = r#"{
            "lists": [
                [{"id": 1, "text": "Задача", "done": false}]
            ]
        }"#;
        fs::write(&path, raw_json).expect("запись тестового json");

        let loaded = read(&path).expect("читается");
        assert_eq!(loaded.active, 0);
        assert_eq!(loaded.lists.len(), 1);

        fs::remove_file(&path).expect("прибирается");
    }

    /// Несуществующий каталог создаётся при первой записи.
    #[test]
    fn a_missing_directory_is_created_along_the_way() {
        let dir = scratch("todo-fresh-dir");
        let _ = fs::remove_dir_all(&dir);

        let path = dir.join(TODO_FILE);
        let board = Board {
            lists: vec![vec![Task {
                id: 100,
                text: "Тестовая задача".to_owned(),
                done: false,
            }]],
            active: 0,
        };

        write(&path, &board).expect("каталог создаётся вместе с файлом");
        assert_eq!(read(&path).expect("читается"), board);

        fs::remove_dir_all(&dir).expect("прибирается");
    }
}
