//! Файл мест окон под защитой: «хорошая» копия рядом с ним.
//!
//! Места и размеры окон ведёт `tauri-plugin-window-state` в
//! `.window-state.json`. Две беды он не закрывает, и обе стоят раскладки
//! целиком — то есть ровно того, что виджеты на втором мониторе держат
//! неделями:
//!
//! - **пишет он `fs::write`**: файл сперва обрезается, потом пишется. Гибель
//!   процесса между этими мгновениями оставляет огрызок, а огрызок плагин при
//!   запуске молча читает как «мест нет»;
//! - **уснувший монитор.** Пропавший (или уснувший так, что Windows его
//!   убрала) второй монитор Windows замещает каскадом окон на основном — 26,26
//!   / 52,52 / … Места пишутся по концу каждого перетаскивания и на выходе, и
//!   выход при спящем мониторе записывал каскад насовсем.
//!
//! Поэтому после каждой записи мест, если мониторов не меньше, чем было при
//! снятии прежней копии, файл копируется в `.window-state.good.json` вместе с
//! числом мониторов, а при каждой записи запоминается, при скольких она была.
//! При запуске (до плагина) копия возвращается на место, если основной файл
//! не разбирается или был записан при меньшем числе мониторов, чем есть
//! сейчас и было у копии. Убранный насовсем монитор копию не вернёт: мониторов
//! сейчас меньше, чем у неё, и раскладкой остаётся то, что расставил человек.
//! Начали без части мониторов — хорошую копию в этом запуске не обновляем,
//! даже если они вернулись: сама раскладка пока могла остаться каскадом.
//! Ошибка чтения основного файла запрещает его замену до перезапуска.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

const MAIN: &str = tauri_plugin_window_state::DEFAULT_FILENAME;
const GOOD: &str = ".window-state.good.json";
/// Число мониторов при последней записи основного файла.
const WRITTEN: &str = ".window-state.monitors";

#[derive(Serialize, Deserialize)]
struct Good {
    monitors: u32,
    places: serde_json::Value,
}

/// Сколько мониторов видит Windows прямо сейчас.
#[cfg(windows)]
fn monitors() -> u32 {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CMONITORS};
    // SAFETY: только читает системную метрику
    unsafe { GetSystemMetrics(SM_CMONITORS) }.max(1) as u32
}

#[cfg(not(windows))]
fn monitors() -> u32 {
    1
}

/// Каталог данных приложения — тот же, что берёт плагин
/// (`app_config_dir`): `%APPDATA%\<идентификатор>`. Зовётся до сборки
/// приложения, когда спросить у Tauri ещё не у кого.
pub fn dir(identifier: &str) -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|root| Path::new(&root).join(identifier))
}

fn read_good(dir: &Path) -> Option<Good> {
    let text = crate::disk::read_existing(&dir.join(GOOD)).ok()??;
    serde_json::from_str(&text).ok()
}

#[derive(Debug)]
enum MainError {
    Read(std::io::Error),
    Invalid,
}

fn read_main(dir: &Path) -> Result<Option<serde_json::Value>, MainError> {
    let Some(text) = crate::disk::read_existing(&dir.join(MAIN)).map_err(MainError::Read)? else {
        return Ok(None);
    };
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .filter(serde_json::Value::is_object)
        .map(Some)
        .ok_or(MainError::Invalid)
}

/// Непрочитанный файл не трогаем до перезапуска. Если начали без части
/// мониторов, вернувшиеся мониторы ещё не означают вернувшуюся раскладку.
struct Session {
    writable: bool,
    monitor_limit: Option<u32>,
}

impl Session {
    const fn new() -> Self {
        Self { writable: true, monitor_limit: None }
    }

    fn heal(&mut self, dir: &Path, now: u32, main: Result<Option<serde_json::Value>, MainError>) {
        if let Err(MainError::Read(error)) = &main {
            self.writable = false;
            eprintln!("места окон не прочитались, запись до перезапуска запрещена: {error}");
            return;
        }
        let Some(good) = read_good(dir) else {
            return;
        };
        if now < good.monitors {
            self.monitor_limit = Some(now);
        }
        let written = std::fs::read_to_string(dir.join(WRITTEN))
            .ok()
            .and_then(|text| text.trim().parse::<u32>().ok());

        let reason = match main {
            Err(MainError::Invalid) => "файл мест не разобрался",
            Ok(Some(_)) if written.is_some_and(|written| written < good.monitors) && now >= good.monitors => {
                "места записаны при спящем мониторе"
            }
            _ => return,
        };

        match serde_json::to_string_pretty(&good.places)
            .map_err(|error| error.to_string())
            .and_then(|text| crate::disk::write_atomically(&dir.join(MAIN), &text).map_err(|error| error.to_string()))
        {
            Ok(()) => eprintln!("{reason}: места окон возвращены из копии ({} мон.)", good.monitors),
            Err(error) => eprintln!("{reason}, копию мест вернуть не вышло: {error}"),
        }
    }

    fn filename(&self) -> &'static str {
        if self.writable { MAIN } else { ".window-state.unread.json" }
    }

    fn after_save(&self, dir: &Path, now: u32) {
        if !self.writable {
            return;
        }
        // Даже после возвращения монитора каскад остаётся каскадом. Метка
        // сохраняет нехватку, чтобы следующий запуск вернул хорошую копию.
        let written = self.monitor_limit.map_or(now, |limit| now.min(limit));
        if let Err(error) = crate::disk::write_atomically(&dir.join(WRITTEN), &written.to_string()) {
            eprintln!("число мониторов при записи мест не записалось: {error}");
        }
        if self.monitor_limit.is_some() || read_good(dir).is_some_and(|good| now < good.monitors) {
            return;
        }
        let Ok(Some(places)) = read_main(dir) else {
            return;
        };
        let good = Good { monitors: now, places };
        match serde_json::to_string(&good) {
            Ok(text) => {
                if let Err(error) = crate::disk::write_atomically(&dir.join(GOOD), &text) {
                    eprintln!("копия мест окон не записалась: {error}");
                }
            }
            Err(error) => eprintln!("копия мест окон не собралась: {error}"),
        }
    }
}

static SESSION: Mutex<Session> = Mutex::new(Session::new());

/// До плагина: вернуть хорошую копию, если основному файлу верить нельзя.
pub fn heal(dir: &Path) {
    SESSION.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).heal(dir, monitors(), read_main(dir));
}

/// Плагин пишет и на выходе сам: при отказе чтения его надо направить в
/// отдельный файл, иначе запрет в after_save пришёл бы уже после перезаписи.
pub fn filename() -> &'static str {
    SESSION.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).filename()
}

/// После удачной записи плагином: отметить число мониторов и обновить копию,
/// если раскладка в этом запуске не была загружена без части мониторов.
pub fn after_save(dir: &Path) {
    SESSION.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).after_save(dir, monitors());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("windgets-tests").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("каталог под тест создаётся");
        dir
    }

    /// Огрызок основного файла заменяется хорошей копией.
    #[test]
    fn a_broken_file_comes_back_from_the_copy() {
        let dir = scratch("places-broken");
        std::fs::write(dir.join(MAIN), r#"{"cpu":{"x":-900,"y":10}}"#).unwrap();
        Session::new().after_save(&dir, 2);
        std::fs::write(dir.join(MAIN), r#"{"cpu":{"x":-9"#).unwrap();

        Session::new().heal(&dir, 2, read_main(&dir));
        let back: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join(MAIN)).unwrap()).unwrap();
        assert_eq!(back["cpu"]["x"], -900);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Целый файл, записанный при том же числе мониторов, не трогается.
    #[test]
    fn a_sound_file_stays_as_it_is() {
        let dir = scratch("places-sound");
        std::fs::write(dir.join(MAIN), r#"{"cpu":{"x":1}}"#).unwrap();
        Session::new().after_save(&dir, 2);
        std::fs::write(dir.join(MAIN), r#"{"cpu":{"x":2}}"#).unwrap();
        Session::new().after_save(&dir, 2);

        Session::new().heal(&dir, 2, read_main(&dir));
        assert!(std::fs::read_to_string(dir.join(MAIN)).unwrap().contains('2'));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Запись при меньшем числе мониторов, чем у копии, при их возвращении
    /// уступает копии.
    #[test]
    fn places_written_without_a_monitor_give_way_to_the_copy() {
        let dir = scratch("places-asleep");
        let good = Good { monitors: 2, places: serde_json::json!({ "cpu": { "x": -900 } }) };
        std::fs::write(dir.join(GOOD), serde_json::to_string(&good).unwrap()).unwrap();
        std::fs::write(dir.join(MAIN), r#"{"cpu":{"x":26}}"#).unwrap();
        std::fs::write(dir.join(WRITTEN), "1").unwrap();

        Session::new().heal(&dir, 2, read_main(&dir));
        assert!(std::fs::read_to_string(dir.join(MAIN)).unwrap().contains("-900"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Отказ чтения не даёт заменить новую раскладку старой копией и не
    /// позволяет поздней записи изменить ни копию, ни метку мониторов.
    #[test]
    fn an_unread_main_file_blocks_writes_for_the_session() {
        let dir = scratch("places-unread");
        std::fs::write(dir.join(MAIN), r#"{"cpu":{"x":1}}"#).unwrap();
        let mut session = Session::new();
        session.after_save(&dir, 2);
        std::fs::write(dir.join(MAIN), r#"{"cpu":{"x":2}}"#).unwrap();
        let main = std::fs::read(dir.join(MAIN)).unwrap();
        let good = std::fs::read(dir.join(GOOD)).unwrap();
        let written = std::fs::read(dir.join(WRITTEN)).unwrap();

        session.heal(&dir, 2, Err(MainError::Read(std::io::ErrorKind::PermissionDenied.into())));
        assert!(!session.writable);
        assert_ne!(session.filename(), MAIN);
        std::fs::write(dir.join(session.filename()), r#"{"cpu":{"x":26}}"#).unwrap();
        session.after_save(&dir, 3);
        assert_eq!(std::fs::read(dir.join(MAIN)).unwrap(), main);
        assert_eq!(std::fs::read(dir.join(GOOD)).unwrap(), good);
        assert_eq!(std::fs::read(dir.join(WRITTEN)).unwrap(), written);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_invalid_and_unread_files_are_different_outcomes() {
        let dir = scratch("places-read-outcomes");
        assert!(matches!(read_main(&dir), Ok(None)));
        std::fs::write(dir.join(MAIN), "{").unwrap();
        assert!(matches!(read_main(&dir), Err(MainError::Invalid)));
        std::fs::write(dir.join(MAIN), "[]").unwrap();
        assert!(matches!(read_main(&dir), Err(MainError::Invalid)));
        std::fs::remove_file(dir.join(MAIN)).unwrap();
        std::fs::create_dir(dir.join(MAIN)).unwrap();
        assert!(matches!(read_main(&dir), Err(MainError::Read(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Возвращение монитора не делает загруженный каскад хорошей копией.
    #[test]
    fn a_short_monitor_start_preserves_the_copy_until_restart() {
        let dir = scratch("places-monitor-returns");
        std::fs::write(dir.join(MAIN), r#"{"cpu":{"x":-900}}"#).unwrap();
        Session::new().after_save(&dir, 2);
        let good = std::fs::read(dir.join(GOOD)).unwrap();
        std::fs::write(dir.join(MAIN), r#"{"cpu":{"x":26}}"#).unwrap();
        std::fs::write(dir.join(WRITTEN), "1").unwrap();
        let mut session = Session::new();
        session.heal(&dir, 1, read_main(&dir));
        session.after_save(&dir, 1);
        session.after_save(&dir, 2);
        assert_eq!(std::fs::read(dir.join(GOOD)).unwrap(), good);
        assert_eq!(std::fs::read_to_string(dir.join(WRITTEN)).unwrap(), "1");

        let mut next = Session::new();
        next.heal(&dir, 2, read_main(&dir));
        assert_eq!(read_main(&dir).unwrap().unwrap()["cpu"]["x"], -900);
        std::fs::write(dir.join(MAIN), r#"{"cpu":{"x":-800}}"#).unwrap();
        next.after_save(&dir, 2);
        assert_eq!(read_good(&dir).unwrap().places["cpu"]["x"], -800);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
