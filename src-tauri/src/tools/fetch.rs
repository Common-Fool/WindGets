//! Скачивание файла выпуска с проверкой SHA-256 на лету.

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use super::Failure;

/// Как часто показываем, сколько скачано. Чаще нет смысла: полоса в карточке
/// шириной в пару сотен точек, а каждое событие — перерисовка всех окон.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// Качает `url` в `dest` и сверяет сумму.
///
/// Файл пишется кусками по мере прихода, сумма считается по тем же кускам —
/// второго чтения тридцати пяти мегабайт с диска нет. Не сошлась сумма или
/// отменили — недокачанное стирается, на месте программы ничего не меняется.
pub async fn download(
    client: &reqwest::Client,
    url: &str,
    expected: [u8; 32],
    dest: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<(), Failure> {
    let mut response = client.get(url).send().await.map_err(network)?;
    if !response.status().is_success() {
        eprintln!("[компоненты] файл выпуска не отдали: {}", response.status());
        return Err(Failure::Network);
    }

    let total = response.content_length();
    let mut file = File::create(dest).map_err(disk)?;
    let mut hasher = Sha256::new();
    let mut received = 0u64;
    let mut shown = Instant::now();

    let outcome = async {
        progress(0, total);
        while let Some(chunk) = response.chunk().await.map_err(network)? {
            if cancel.load(Ordering::SeqCst) {
                return Err(Failure::Cancelled);
            }
            file.write_all(&chunk).map_err(disk)?;
            hasher.update(&chunk);
            received += chunk.len() as u64;

            if shown.elapsed() >= PROGRESS_INTERVAL {
                shown = Instant::now();
                progress(received, total);
            }
        }
        progress(received, total);
        file.flush().map_err(disk)?;

        if hasher.finalize_reset().as_slice() != expected {
            eprintln!("[компоненты] сумма не сошлась: {url}");
            return Err(Failure::BadChecksum);
        }
        Ok(())
    }
    .await;

    drop(file);
    if outcome.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    outcome
}

/// Проверенный файл, запертый от записи до запуска.
///
/// Пока значение живо, файл открыт без права записи и удаления для других:
/// подменить его нельзя. Запускать — по `path`, полученному от самого хендла.
pub struct Sealed {
    _file: File,
    pub path: std::path::PathBuf,
}

/// Запирает скачанный установщик и заново сверяет его сумму — уже по
/// запертому хендлу.
///
/// ВАЖНО: **запускаем те же байты, что проверили.** Папка программ лежит в
/// `%LOCALAPPDATA%`, куда пишет любой процесс пользователя без повышения, а
/// установщики идут с правами приложения, то есть администратора. Между
/// проверкой при скачивании и запуском чужой процесс успевал бы подменить файл
/// и получить права без окна UAC. Поэтому: открыть с разделением только на
/// чтение (запись, переименование и удаление другим запрещены, открытый
/// заранее чужой писатель даёт отказ), пересчитать сумму по этому хендлу и
/// запускать по настоящему пути файла, а не по имени, которое можно увести
/// через переименование папки.
#[cfg(windows)]
pub fn seal(path: &Path, expected: [u8; 32]) -> Result<Sealed, Failure> {
    use std::io::Read;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{GetFinalPathNameByHandleW, FILE_NAME_NORMALIZED, FILE_SHARE_READ};

    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0)
        .open(path)
        .map_err(disk)?;

    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 16];
    loop {
        let read = file.read(&mut buffer).map_err(disk)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    if hasher.finalize().as_slice() != expected {
        eprintln!("[компоненты] сумма установщика не сошлась перед запуском: {}", path.display());
        return Err(Failure::BadChecksum);
    }

    let handle = HANDLE(file.as_raw_handle());
    let mut wide = vec![0u16; 1024];
    let length = unsafe { GetFinalPathNameByHandleW(handle, &mut wide, FILE_NAME_NORMALIZED) } as usize;
    if length == 0 || length >= wide.len() {
        eprintln!("[компоненты] не узнать настоящий путь установщика: {}", path.display());
        return Err(Failure::Disk);
    }
    let full = String::from_utf16_lossy(&wide[..length]);
    Ok(Sealed { _file: file, path: plain_path(&full) })
}

/// `\\?\C:\…` → `C:\…`: путь с приставкой не всякая программа переносит в
/// собственном `argv[0]`. Сетевой `\\?\UNC\…` оставляем как есть.
///
/// ВАЖНО: **приставку снимаем, только если путь без неё значит то же самое.**
/// Обычный путь Windows нормализует: точку и пробел в конце части имени
/// обрезает. Junction на папку `verified.` дал бы проверенный файл там, а
/// запуск по `…\verified.\setup.exe` ушёл бы в соседнюю `verified` к чужому
/// exe. Такой путь остаётся с приставкой — она нормализацию отключает.
fn plain_path(full: &str) -> std::path::PathBuf {
    match full.strip_prefix(r"\\?\") {
        Some(local) if !local.starts_with("UNC\\") && survives_normalization(local) => std::path::PathBuf::from(local),
        _ => std::path::PathBuf::from(full),
    }
}

fn survives_normalization(local: &str) -> bool {
    local.split('\\').skip(1).all(|part| !part.is_empty() && !part.ends_with(['.', ' ']))
}

fn network(error: reqwest::Error) -> Failure {
    eprintln!("[компоненты] скачивание оборвалось: {error}");
    Failure::Network
}

fn disk(error: std::io::Error) -> Failure {
    eprintln!("[компоненты] запись на диск не прошла: {error}");
    Failure::Disk
}

#[cfg(test)]
mod tests {
    use super::plain_path;
    use std::path::PathBuf;

    #[test]
    fn a_plain_local_path_loses_the_prefix() {
        assert_eq!(plain_path(r"\\?\D:\Apps\tools\update\setup.exe"), PathBuf::from(r"D:\Apps\tools\update\setup.exe"));
    }

    /// Путь, который Windows без приставки прочла бы иначе, запускается с ней.
    #[test]
    fn a_path_the_normalizer_would_change_keeps_the_prefix() {
        for full in [
            r"\\?\D:\Apps\verified.\setup.exe",
            r"\\?\D:\Apps\verified \setup.exe",
            r"\\?\D:\Apps\tools\setup.exe.",
            r"\\?\UNC\server\share\setup.exe",
        ] {
            assert_eq!(plain_path(full), PathBuf::from(full));
        }
    }
}
