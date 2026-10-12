//! Что стоит и когда проверяли: `tools.json` в каталоге компонентов.
//!
//! Рядом с самими программами, а не в папке настроек: это не выбор человека, а
//! описание того, что лежит на этой машине. Удалили папку компонентов — с ней
//! ушла и запись, и приложение честно считает программы отсутствующими.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::catalog::Component;

const FILE: &str = "tools.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Entry {
    /// Номер выпуска на GitHub, из которого поставлена программа.
    pub tag: Option<String>,
    /// Когда поставили и когда последний раз спрашивали о свежем, секунды эпохи.
    pub installed_at: Option<i64>,
    pub checked_at: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stored {
    pub components: BTreeMap<Component, Entry>,
}

impl Stored {
    pub fn entry(&self, component: Component) -> Entry {
        self.components.get(&component).cloned().unwrap_or_default()
    }
}

/// Любая беда — «записей нет»: программы от этого не пропадут, пропадёт только
/// номер версии в настройках до следующей проверки.
pub fn load(root: &Path) -> Stored {
    std::fs::read_to_string(root.join(FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(root: &Path, stored: &Stored) {
    let write = || -> Result<(), String> {
        std::fs::create_dir_all(root).map_err(|error| error.to_string())?;
        let text = serde_json::to_string_pretty(stored).map_err(|error| error.to_string())?;
        crate::disk::write_atomically(&root.join(FILE), &text).map_err(|error| error.to_string())
    };

    if let Err(error) = write() {
        eprintln!("[компоненты] tools.json не записался: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn survives_round_trip() {
        let mut stored = Stored::default();
        stored.components.insert(
            Component::YtDlp,
            Entry { tag: Some("2026.08.19".to_owned()), installed_at: Some(1), checked_at: Some(2) },
        );
        let text = serde_json::to_string(&stored).unwrap();
        assert!(text.contains("\"ytDlp\""), "ключи те же, что видит фронт: {text}");
        assert_eq!(serde_json::from_str::<Stored>(&text).unwrap(), stored);
    }

    #[test]
    fn a_broken_file_reads_as_empty() {
        assert_eq!(serde_json::from_str::<Stored>("{}").unwrap(), Stored::default());
        assert!(serde_json::from_str::<Stored>("not json").is_err());
        assert_eq!(Stored::default().entry(Component::Ffmpeg), Entry::default());
    }
}
