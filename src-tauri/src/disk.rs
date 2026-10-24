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
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Расширение временного соседа. Он живёт доли секунды, но попасться на глаза
/// может — пусть будет понятно, чей он.
const TEMPORARY: &str = "tmp";

/// Сколько раз пробуем, когда файл на мгновение занят чужим (антивирус,
/// индексатор, второй писатель), и сколько ждём между попытками.
const ATTEMPTS: u32 = 5;
const PAUSE: Duration = Duration::from_millis(40);

/// Номер очередного временного соседа в этом процессе.
static NEXT: AtomicU64 = AtomicU64::new(0);

/// Пишет текст или байты так, чтобы прежний файл либо уцелел целиком, либо
/// сменился целиком.
///
/// ВАЖНО: **временный сосед у каждой записи свой.** Прежде он был один на файл
/// (`settings.tmp`), и два писателя разом обрезали и переименовывали один и тот
/// же файл: на диск ложилась смесь двух текстов, то есть неразборчивый JSON.
/// Своё имя убирает только эту беду — порядок снимков вызывающий держит сам
/// (у настроек это очередь `SAVING`, у почты и Спотифая — замок хранилища).
pub fn write_atomically(path: &Path, data: impl AsRef<[u8]>) -> io::Result<()> {
    let temporary = temporary_for(path);

    let outcome = fill(&temporary, data.as_ref()).and_then(|()| replace(&temporary, path));

    if outcome.is_err() {
        // Не вышло — соседа за собой убираем: он неполный, и путать его с
        // делом незачем
        let _ = fs::remove_file(&temporary);
    }

    outcome
}

/// Читает файл, отличая «его нет» от «он есть, но не прочитался».
///
/// Разница решающая: отсутствующий файл — это «записей ещё нет», и писать
/// поверх можно; непрочитанный — это записи, которых мы не видели, и первая же
/// запись затёрла бы их пустым списком. Занятый на мгновение файл пробуем
/// несколько раз, прежде чем сдаться.
///
/// Метку порядка байт UTF-8 в начале снимаем: её ставят Блокнот и
/// PowerShell 5.1, а `serde_json` на ней спотыкается, и поправленный руками
/// файл ушёл бы в `*.bad.json` целиком. Текст не в UTF-8 — это файл, который
/// прочитался, но не разберётся: отдаём пустую строку, и он уходит в сторону
/// дорогой испорченного, а не запирает запись до перезапуска.
pub fn read_existing(path: &Path) -> io::Result<Option<String>> {
    let mut attempt = 1;
    loop {
        match fs::read(path) {
            Ok(bytes) => {
                let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
                return Ok(Some(String::from_utf8(bytes.to_vec()).unwrap_or_default()));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) if attempt >= ATTEMPTS => return Err(error),
            Err(_) => {
                attempt += 1;
                std::thread::sleep(PAUSE);
            }
        }
    }
}

/// Сколько отложенных копий одного файла держим рядом.
const ASIDE_LIMIT: u32 = 100;

/// Откладывает неразобравшийся файл в сторону — рядом, с пометкой `bad`, — чтобы
/// первая же запись не затёрла его и было что восстановить руками.
///
/// ВАЖНО: **прежнюю отложенную копию не затираем.** Переименование на Windows
/// молча заменяет существующий файл, а в `totp.bad.json` бывают секреты,
/// которых больше нет нигде: испортись файл второй раз, первая копия пропала
/// бы. Вторая ложится в `*.bad-2.json`, третья — в `*.bad-3.json`.
pub fn set_aside(path: &Path) -> io::Result<PathBuf> {
    let aside = (1..=ASIDE_LIMIT)
        .map(|number| match number {
            1 => path.with_extension("bad.json"),
            _ => path.with_extension(format!("bad-{number}.json")),
        })
        .find(|candidate| !candidate.exists())
        .ok_or_else(|| io::Error::new(io::ErrorKind::AlreadyExists, "отложенных копий слишком много"))?;
    fs::rename(path, &aside)?;
    Ok(aside)
}

/// Имя временного соседа: имя файла, номер процесса и номер записи.
fn temporary_for(path: &Path) -> PathBuf {
    let number = NEXT.fetch_add(1, Ordering::Relaxed);
    path.with_extension(format!("{}-{number}.{TEMPORARY}", std::process::id()))
}

/// Ставит соседа на место файла. Занятый чужим файл Windows не даёт заменить
/// (`ERROR_ACCESS_DENIED`, `ERROR_SHARING_VIOLATION`) — это мгновение, а не
/// отказ, поэтому пробуем ещё.
fn replace(temporary: &Path, path: &Path) -> io::Result<()> {
    let mut attempt = 1;
    loop {
        match fs::rename(temporary, path) {
            Ok(()) => return Ok(()),
            Err(error) if attempt >= ATTEMPTS || error.kind() == io::ErrorKind::NotFound => return Err(error),
            Err(_) => {
                attempt += 1;
                std::thread::sleep(PAUSE);
            }
        }
    }
}

/// Создаёт временного соседа и доводит его содержимое до диска.
fn fill(temporary: &Path, data: &[u8]) -> io::Result<()> {
    let mut file = File::create(temporary)?;
    file.write_all(data)?;
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

    /// Временные соседи, оставшиеся у файла: имя файла, точка, хвост `.tmp`.
    fn leftovers(path: &Path) -> Vec<PathBuf> {
        let stem = path.file_stem().and_then(|stem| stem.to_str()).unwrap_or_default().to_owned();
        let Ok(entries) = fs::read_dir(path.parent().expect("у файла есть каталог")) else {
            return Vec::new();
        };
        entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|candidate| {
                let name = candidate.file_name().and_then(|name| name.to_str()).unwrap_or_default();
                name.starts_with(&format!("{stem}.")) && name.ends_with(&format!(".{TEMPORARY}"))
            })
            .collect()
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
        assert!(leftovers(&path).is_empty(), "временный сосед остался на диске");

        fs::remove_file(&path).expect("прибирается");
    }

    /// Отказ не должен оставлять после себя огрызок: каталога нет — значит,
    /// нет и соседа.
    #[test]
    fn a_failed_write_leaves_no_leftovers() {
        let path = scratch("nowhere").join("deeper").join("settings.json");

        assert!(write_atomically(&path, "что-нибудь").is_err());
        assert!(leftovers(&path).is_empty());
    }

    /// Писатели разом не смешивают тексты: на диске всегда ровно один из
    /// записанных целиком. С общим временным соседом половина таких кругов
    /// оставляла неразборчивый JSON.
    #[test]
    fn writers_at_once_leave_one_whole_text() {
        let path = scratch("at-once.json");
        let _ = fs::remove_file(&path);

        let texts: Vec<String> = (0..8)
            .map(|writer| format!("{{\"writer\":{writer},\"tail\":\"{}\"}}", "x".repeat(4096 * (writer + 1))))
            .collect();

        for _round in 0..10 {
            std::thread::scope(|scope| {
                for text in &texts {
                    let path = &path;
                    scope.spawn(move || {
                        let _ = write_atomically(path, text);
                    });
                }
            });

            let written = fs::read_to_string(&path).expect("файл на месте");
            assert!(texts.contains(&written), "на диске смесь текстов");
        }

        assert!(leftovers(&path).is_empty(), "временные соседи остались на диске");
        fs::remove_file(&path).expect("прибирается");
    }

    /// Отсутствующий файл — не отказ: записей просто ещё нет.
    #[test]
    fn a_missing_file_reads_as_none_and_a_present_one_as_text() {
        let path = scratch("read-existing.json");
        let _ = fs::remove_file(&path);
        assert!(read_existing(&path).expect("отсутствие не ошибка").is_none());

        write_atomically(&path, "{}").expect("файл пишется");
        assert_eq!(read_existing(&path).expect("читается").as_deref(), Some("{}"));
        fs::remove_file(&path).expect("прибирается");
    }

    /// Метка порядка байт снимается, а не-UTF-8 читается как неразборчивое.
    #[test]
    fn a_byte_order_mark_is_dropped_and_foreign_bytes_read_as_garbage() {
        let path = scratch("read-existing-bom.json");
        fs::write(&path, b"\xEF\xBB\xBF{\"a\":1}").expect("файл пишется");
        assert_eq!(read_existing(&path).expect("читается").as_deref(), Some("{\"a\":1}"));

        // «Грац» в cp1251 — не UTF-8
        fs::write(&path, b"{\"city\":\"\xC3\xF0\xE0\xF6\"}").expect("файл пишется");
        assert_eq!(read_existing(&path).expect("читается").as_deref(), Some(""));
        fs::remove_file(&path).expect("прибирается");
    }

    /// Испорченный второй раз файл не затирает первую отложенную копию.
    #[test]
    fn setting_aside_twice_keeps_both_copies() {
        let path = scratch("aside.json");
        let first = path.with_extension("bad.json");
        let second = path.with_extension("bad-2.json");
        for leftover in [&path, &first, &second] {
            let _ = fs::remove_file(leftover);
        }

        fs::write(&path, "первая порча").expect("файл пишется");
        assert_eq!(set_aside(&path).expect("откладывается"), first);
        fs::write(&path, "вторая порча").expect("файл пишется");
        assert_eq!(set_aside(&path).expect("откладывается"), second);

        assert_eq!(fs::read_to_string(&first).expect("читается"), "первая порча");
        assert_eq!(fs::read_to_string(&second).expect("читается"), "вторая порча");
        for leftover in [&first, &second] {
            fs::remove_file(leftover).expect("прибирается");
        }
    }

    /// Каталог вместо файла — настоящий отказ чтения, а не «файла нет».
    #[test]
    fn an_unreadable_path_is_an_error_not_an_absence() {
        let path = scratch("read-existing-dir.json");
        let _ = fs::remove_file(&path);
        fs::create_dir_all(&path).expect("каталог создаётся");
        assert!(read_existing(&path).is_err());
        fs::remove_dir(&path).expect("прибирается");
    }
}
