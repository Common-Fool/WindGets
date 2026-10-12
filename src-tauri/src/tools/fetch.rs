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

fn network(error: reqwest::Error) -> Failure {
    eprintln!("[компоненты] скачивание оборвалось: {error}");
    Failure::Network
}

fn disk(error: std::io::Error) -> Failure {
    eprintln!("[компоненты] запись на диск не прошла: {error}");
    Failure::Disk
}
