//! Распаковка архива встроенным `tar.exe` Windows.
//!
//! Своего распаковщика нет, и крейт под 7z ради одного файла не нужен: в
//! Windows 10 и 11 лежит bsdtar с liblzma, и 7z со сборкой ffmpeg он
//! распаковывает (замерено: 35 МБ за две секунды). Из архива берётся ровно
//! один файл — остальное, включая второй ffmpeg-овский exe по сотне мегабайт,
//! на диск не попадает вовсе.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use super::Failure;
use crate::child::{self, Line};

/// Сколько ждём `tar`. Список читается за сотые доли секунды, распаковка
/// одного файла — пару секунд; минута означает, что он встал.
const TAR_LIMIT: Duration = Duration::from_secs(60);

/// Какой путь внутри архива кончается на `member`.
///
/// Ровно один: у сборки ffmpeg папка внутри архива названа версией, и точного
/// пути заранее не знать, но два совпадения значили бы, что архив устроен не
/// так, как мы думаем, — и брать наугад нельзя.
pub fn member_to_extract(listing: &[String], member: &str) -> Option<String> {
    let wanted = member.to_ascii_lowercase();
    let mut found = listing.iter().filter(|entry| {
        entry
            .replace('\\', "/")
            .to_ascii_lowercase()
            .ends_with(&wanted)
    });
    match (found.next(), found.next()) {
        (Some(entry), None) => Some(entry.clone()),
        _ => None,
    }
}

/// Достаёт из архива один файл в `into` и отдаёт путь к нему.
pub fn unpack(archive: &Path, member: &str, into: &Path) -> Result<PathBuf, Failure> {
    let listing = tar(&["-tf".as_ref(), archive.as_os_str()])?;
    let entry = member_to_extract(&listing, member).ok_or_else(|| {
        eprintln!("[компоненты] в архиве нет {member}");
        Failure::NoBuild
    })?;

    std::fs::create_dir_all(into).map_err(|error| {
        eprintln!("[компоненты] папка распаковки не создалась: {error}");
        Failure::Disk
    })?;
    tar(&[
        "-xf".as_ref(),
        archive.as_os_str(),
        "-C".as_ref(),
        into.as_os_str(),
        entry.as_ref(),
    ])?;

    let unpacked = into.join(&entry);
    if unpacked.is_file() {
        Ok(unpacked)
    } else {
        eprintln!("[компоненты] tar промолчал, но файла нет: {}", unpacked.display());
        Err(Failure::Unpack)
    }
}

/// Запускает `tar` и отдаёт строки его stdout. Ненулевой код — отказ.
fn tar(args: &[&std::ffi::OsStr]) -> Result<Vec<String>, Failure> {
    #[cfg(windows)]
    let program = crate::autostart::system_exe("System32\\tar.exe");
    #[cfg(not(windows))]
    let program = PathBuf::from("tar");

    let mut command = Command::new(program);
    command.args(args);

    let (code, lines) = child::run(command, TAR_LIMIT).map_err(|error| {
        eprintln!("[компоненты] tar не запустился: {error}");
        Failure::Unpack
    })?;

    if code != Some(0) {
        let complaint: Vec<&str> = lines
            .iter()
            .filter_map(|line| match line {
                Line::Err(text) => Some(text.as_str()),
                Line::Out(_) => None,
            })
            .collect();
        eprintln!("[компоненты] tar отказал ({code:?}): {}", complaint.join(" | "));
        return Err(Failure::Unpack);
    }

    Ok(lines
        .into_iter()
        .filter_map(|line| match line {
            Line::Out(text) if !text.trim().is_empty() => Some(text),
            _ => None,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing() -> Vec<String> {
        [
            "ffmpeg-9.0.2-essentials_build/",
            "ffmpeg-9.0.2-essentials_build/LICENSE",
            "ffmpeg-9.0.2-essentials_build/bin/",
            "ffmpeg-9.0.2-essentials_build/bin/ffmpeg.exe",
            "ffmpeg-9.0.2-essentials_build/bin/ffplay.exe",
            "ffmpeg-9.0.2-essentials_build/bin/ffprobe.exe",
            "ffmpeg-9.0.2-essentials_build/doc/ffmpeg.html",
        ]
        .map(str::to_owned)
        .to_vec()
    }

    /// Папка внутри архива названа версией — находим по хвосту пути, а
    /// соседние exe и страница документации с тем же именем не мешают.
    #[test]
    fn the_member_is_found_by_its_tail() {
        assert_eq!(
            member_to_extract(&listing(), "/bin/ffmpeg.exe").as_deref(),
            Some("ffmpeg-9.0.2-essentials_build/bin/ffmpeg.exe")
        );
    }

    #[test]
    fn a_missing_or_doubled_member_is_refused() {
        assert_eq!(member_to_extract(&listing(), "/bin/ffmpeg-new.exe"), None);

        let mut doubled = listing();
        doubled.push("other/bin/ffmpeg.exe".to_owned());
        assert_eq!(member_to_extract(&doubled, "/bin/ffmpeg.exe"), None);
    }

    /// Обратная косая черта и регистр в списке не мешают.
    #[test]
    fn slashes_and_case_do_not_matter() {
        let listing = vec!["Build\\BIN\\FFmpeg.exe".to_owned()];
        assert!(member_to_extract(&listing, "/bin/ffmpeg.exe").is_some());
    }
}
