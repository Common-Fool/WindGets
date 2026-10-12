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
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// Имя файла в каталоге настроек. Рядом с `settings.json` и `notes.json`.
const TODO_FILE: &str = "todo.json";

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
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
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
#[tauri::command]
pub fn todo_lists(app: AppHandle) -> Result<Board, String> {
    let outcome = todo_path(&app).and_then(|path| read(&path));

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
pub fn set_todo_lists(app: AppHandle, board: Board) -> Result<(), String> {
    let outcome = todo_path(&app).and_then(|path| write(&path, &board));

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
