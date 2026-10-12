//! Запись файлов приложения на диск.
//!
//! ВАЖНО: **`fs::write` сперва обрезает файл, а потом пишет.** Между этими
//! мгновениями настоящего файла нет вовсе, и всё, что случится в этот момент —
//! выключение света, гибель процесса, — оставляет на диске огрызок. Читаем мы
//! такие файлы молча: разобранный с ошибкой JSON превращается в значения по
//! умолчанию, то есть пропажа настроек выглядит как «оно само сбросилось».
//!
//! Поэтому пишем рядом, а на место ставим переименованием: оно на NTFS
//! неделимо, и прежний файл либо цел, либо сменился целиком. Данные до этого
//! доводим до диска сами — без этого порядок записи и переименования никем не
//! обещан.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

/// Расширение временного соседа. Он живёт доли секунды, но попасться на глаза
/// может — пусть будет понятно, чей он.
const TEMPORARY: &str = "tmp";

/// Пишет текст так, чтобы прежний файл либо уцелел целиком, либо сменился
/// целиком.
pub fn write_atomically(path: &Path, text: &str) -> io::Result<()> {
    let temporary = path.with_extension(TEMPORARY);

    let outcome = fill(&temporary, text).and_then(|()| fs::rename(&temporary, path));

    if outcome.is_err() {
        // Не вышло — соседа за собой убираем: он неполный, и путать его с
        // делом незачем
        let _ = fs::remove_file(&temporary);
    }

    outcome
}

/// Создаёт временного соседа и доводит его содержимое до диска.
fn fill(temporary: &Path, text: &str) -> io::Result<()> {
    let mut file = File::create(temporary)?;
    file.write_all(text.as_bytes())?;
    // Без этого переименование может лечь на диск раньше самих данных
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Каталог под тесты этого файла: имена внутри свои у каждого теста.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("windgets-tests");
        fs::create_dir_all(&dir).expect("каталог под тесты создаётся");
        dir.join(name)
    }

    #[test]
    fn the_file_appears_with_the_text_in_it() {
        let path = scratch("appears.json");
        let _ = fs::remove_file(&path);

        write_atomically(&path, "{\"a\":1}").expect("файл пишется");
        assert_eq!(fs::read_to_string(&path).expect("читается"), "{\"a\":1}");

        fs::remove_file(&path).expect("прибирается");
    }

    /// Вторая запись меняет содержимое целиком и не оставляет соседа: файл
    /// правят при каждой галочке в настройках, и мусор рядом копился бы.
    #[test]
    fn the_second_write_replaces_the_first_and_leaves_nothing_behind() {
        let path = scratch("replaces.json");
        let _ = fs::remove_file(&path);

        write_atomically(&path, "старое").expect("первая запись");
        write_atomically(&path, "новое").expect("вторая запись");

        assert_eq!(fs::read_to_string(&path).expect("читается"), "новое");
        assert!(
            !path.with_extension(TEMPORARY).exists(),
            "временный сосед остался на диске"
        );

        fs::remove_file(&path).expect("прибирается");
    }

    /// Отказ не должен оставлять после себя огрызок: каталога нет — значит,
    /// нет и соседа.
    #[test]
    fn a_failed_write_leaves_no_leftovers() {
        let path = scratch("nowhere").join("deeper").join("settings.json");

        assert!(write_atomically(&path, "что-нибудь").is_err());
        assert!(!path.with_extension(TEMPORARY).exists());
    }
}
