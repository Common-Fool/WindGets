//! Лицензии: полный список чужого кода и его лицензий.
//!
//! Сам список собирает `scripts/notices.mjs` из манифестов пакетов — крейтов,
//! npm и сборок сайдкара — в `THIRD-PARTY-NOTICES.txt` и короткую выжимку
//! `src/lib/credits.json` для раздела «Лицензия». Rust тут только открывает
//! полный файл и следит, чтобы он не отстал от зависимостей.

use std::path::PathBuf;

use tauri::{AppHandle, Manager};

/// Имя файла рядом с программой: так он лежит и в установщике, и в корне
/// репозитория.
const NOTICES: &str = "THIRD-PARTY-NOTICES.txt";

/// Тот же файл в дереве исходников: в разработке ресурсов рядом с программой
/// нет, как и у сайдкара (`sensors::locate`).
const IN_TREE: &str = "../THIRD-PARTY-NOTICES.txt";

/// Где лежит полный список: сперва ресурсы установленного приложения, потом
/// дерево исходников.
fn locate(resource_dir: Option<PathBuf>) -> Option<PathBuf> {
    let candidates = [resource_dir.map(|dir| dir.join(NOTICES)), Some(IN_TREE.into())];
    candidates.into_iter().flatten().find(|path| path.is_file())
}

/// Открывает полный список лицензий программой, которая открывает текстовые
/// файлы.
///
/// Проводником, отдельным процессом (`links::launch`): путь локальный и
/// короткий, без запятых, — ровно то, что проводник понимает.
#[tauri::command]
pub fn open_notices(app: AppHandle) -> Result<(), String> {
    let path = locate(app.path().resource_dir().ok()).ok_or_else(|| "notices not found".to_owned())?;
    // Полный путь: проводник, получивший относительный, открыл бы «Документы»
    let path = path.canonicalize().map_err(|error| error.to_string())?;
    let shown = path.to_string_lossy();
    // `canonicalize` на Windows возвращает путь с `\\?\`, а проводник его не понимает
    crate::links::launch(shown.strip_prefix(r"\\?\").unwrap_or(&shown));
    Ok(())
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};
    use std::path::Path;

    fn root() -> &'static Path {
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."))
    }

    /// Отпечаток без оглядки на переводы строк — ровно как у генератора.
    fn fingerprint(path: &str) -> String {
        let text = std::fs::read_to_string(root().join(path)).unwrap_or_else(|error| panic!("{path}: {error}"));
        let digest = Sha256::digest(text.replace("\r\n", "\n").as_bytes());
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// Список лицензий собран по нынешним зависимостям.
    ///
    /// Сменилась любая зависимость — крейт, пакет npm или сборка сайдкара, —
    /// сменился и отпечаток её файла блокировки, и тогда список врёт: в нём нет
    /// нового кода или висит ушедший. Лечится `npm run notices`.
    #[test]
    fn notices_are_fresh() {
        let notices = std::fs::read_to_string(root().join("THIRD-PARTY-NOTICES.txt")).expect("THIRD-PARTY-NOTICES.txt читается");
        let credits = std::fs::read_to_string(root().join("src/lib/credits.json")).expect("credits.json читается");

        for path in ["src-tauri/Cargo.lock", "package-lock.json", "src-sensors/Sensors.csproj"] {
            let hash = fingerprint(path);
            assert!(
                notices.contains(&format!("{path} sha256={hash}")),
                "THIRD-PARTY-NOTICES.txt собран не по нынешнему {path} — запустите `npm run notices`"
            );
            assert!(
                credits.contains(&hash),
                "src/lib/credits.json собран не по нынешнему {path} — запустите `npm run notices`"
            );
        }
    }

    /// Каждая прямая зависимость программы названа в полном списке.
    ///
    /// Страховка поверх отпечатков: генератор, ошибившийся в обходе графа,
    /// выпустил бы свежий по отпечаткам, но неполный список.
    #[test]
    fn every_direct_dependency_is_listed() {
        let notices = std::fs::read_to_string(root().join("THIRD-PARTY-NOTICES.txt")).expect("THIRD-PARTY-NOTICES.txt читается");
        let manifest = std::fs::read_to_string(root().join("src-tauri/Cargo.toml")).expect("Cargo.toml читается");

        // Разделы обычных зависимостей: общий и виндовый. Сборочные и тестовые
        // в программу не попадают
        let mut section = "";
        for line in manifest.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                section = line;
                continue;
            }
            let runtime = section == "[dependencies]" || section == "[target.'cfg(windows)'.dependencies]";
            if !runtime || line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((name, _)) = line.split_once('=') else {
                continue;
            };
            let name = name.trim();
            assert!(
                notices.contains(&format!("  - {name} ")),
                "зависимость «{name}» не названа в THIRD-PARTY-NOTICES.txt — запустите `npm run notices`"
            );
        }

        for name in ["react", "react-dom", "@tauri-apps/api", "LibreHardwareMonitorLib", "PawnIO.Modules"] {
            assert!(notices.contains(&format!("  - {name} ")), "«{name}» не назван в THIRD-PARTY-NOTICES.txt");
        }
    }
}
