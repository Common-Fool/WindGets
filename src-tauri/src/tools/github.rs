//! Выпуски на GitHub: какой свежий, какой файл брать и какая у него сумма.
//!
//! ВАЖНО: **сумма берётся из самого API, а не из файла сумм рядом.** У каждого
//! файла выпуска GitHub отдаёт поле `digest` вида `sha256:…` — проверено на
//! всех трёх программах. Одна дорога проверки на всех, и без поля файл не
//! ставится вовсе: второй дороги «на доверии» нет.

use serde::Deserialize;

use super::catalog::AssetRule;
use super::Failure;

/// Свежий выпуск — только нужные поля.
#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub digest: Option<String>,
}

/// Файл выпуска по правилу каталога.
///
/// Ровно один: ни одного — сборку переименовали, два — правило стало
/// двусмысленным. В обоих случаях брать наугад нельзя, и человек видит «сборка
/// не найдена, обновите приложение».
pub fn pick_asset(release: &Release, rule: AssetRule) -> Result<&Asset, Failure> {
    let mut found = release.assets.iter().filter(|asset| rule.matches(&asset.name));
    match (found.next(), found.next()) {
        (Some(asset), None) => Ok(asset),
        _ => Err(Failure::NoBuild),
    }
}

/// `sha256:<64 шестнадцатеричных знака>` в байты.
pub fn parse_digest(digest: Option<&str>) -> Result<[u8; 32], Failure> {
    let hex = digest
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .ok_or(Failure::NoChecksum)?;
    if hex.len() != 64 {
        return Err(Failure::NoChecksum);
    }

    let mut bytes = [0u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        let pair = hex.get(index * 2..index * 2 + 2).ok_or(Failure::NoChecksum)?;
        *byte = u8::from_str_radix(pair, 16).map_err(|_| Failure::NoChecksum)?;
    }
    Ok(bytes)
}

/// Пора ли спросить GitHub о свежем выпуске.
///
/// Часы, ушедшие назад (метка из будущего), — тоже «пора»: иначе перевод
/// часов на год вперёд и обратно заморозил бы проверки на этот год.
pub fn check_due(checked_at: Option<i64>, now: i64, every: i64) -> bool {
    match checked_at {
        None => true,
        Some(at) if at > now => true,
        Some(at) => now - at >= every,
    }
}

/// Спрашивает свежий выпуск репозитория.
pub async fn latest(client: &reqwest::Client, repo: &str) -> Result<Release, Failure> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let response = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| {
            eprintln!("[компоненты] GitHub не ответил: {error}");
            Failure::Network
        })?;

    let status = response.status();
    if !status.is_success() {
        // Безымянных запросов GitHub пускает шестьдесят в час; кончились —
        // 403 или 429 с нулём в остатке
        let exhausted = response
            .headers()
            .get("x-ratelimit-remaining")
            .is_some_and(|left| left.as_bytes() == b"0");
        eprintln!("[компоненты] GitHub ответил {status} на {repo}");
        return Err(match status.as_u16() {
            403 | 429 if exhausted => Failure::RateLimited,
            404 => Failure::NoBuild,
            _ => Failure::Network,
        });
    }

    response.json::<Release>().await.map_err(|error| {
        eprintln!("[компоненты] ответ GitHub не разобрался: {error}");
        Failure::Network
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(names: &[&str]) -> Release {
        Release {
            tag_name: "v1".to_owned(),
            assets: names
                .iter()
                .map(|name| Asset {
                    name: (*name).to_owned(),
                    browser_download_url: format!("https://example.com/{name}"),
                    size: 1,
                    digest: None,
                })
                .collect(),
        }
    }

    #[test]
    fn an_exact_name_is_picked() {
        let release = release(&["yt-dlp.exe", "yt-dlp_x86.exe", "SHA2-256SUMS"]);
        let asset = pick_asset(&release, AssetRule::Exact("yt-dlp.exe")).unwrap();
        assert_eq!(asset.name, "yt-dlp.exe");
    }

    #[test]
    fn a_missing_or_ambiguous_build_is_refused() {
        let rule = AssetRule::Around { prefix: "ffmpeg-", suffix: ".7z" };
        assert!(matches!(
            pick_asset(&release(&["other.zip"]), rule),
            Err(Failure::NoBuild)
        ));
        assert!(matches!(
            pick_asset(&release(&["ffmpeg-1.7z", "ffmpeg-2.7z"]), rule),
            Err(Failure::NoBuild)
        ));
    }

    #[test]
    fn a_digest_is_read_in_either_case() {
        let lower = "sha256:2aeabf0092c3262d6b2609824418f7dd7ed1f1df939f73b2b15645230cac0d77";
        let bytes = parse_digest(Some(lower)).unwrap();
        assert_eq!(bytes[0], 0x2a);
        assert_eq!(bytes[31], 0x77);
        assert_eq!(parse_digest(Some(&lower.to_uppercase().replace("SHA256", "sha256"))).unwrap(), bytes);
    }

    #[test]
    fn a_bad_digest_means_no_checksum() {
        assert!(matches!(parse_digest(None), Err(Failure::NoChecksum)));
        assert!(matches!(parse_digest(Some("md5:abcd")), Err(Failure::NoChecksum)));
        assert!(matches!(parse_digest(Some("sha256:abcd")), Err(Failure::NoChecksum)));
        let not_hex = format!("sha256:{}", "zz".repeat(32));
        assert!(matches!(parse_digest(Some(&not_hex)), Err(Failure::NoChecksum)));
    }

    #[test]
    fn a_check_is_due_after_a_day_and_after_the_clock_went_back() {
        let day = 86_400;
        assert!(check_due(None, 1_000_000, day));
        assert!(!check_due(Some(1_000_000 - 23 * 3600), 1_000_000, day));
        assert!(check_due(Some(1_000_000 - day), 1_000_000, day));
        assert!(check_due(Some(2_000_000), 1_000_000, day));
    }

    /// Поле `digest` в ответе API — ровно такой формы, как у настоящего
    /// выпуска QuickJS.
    #[test]
    fn a_release_parses_from_the_api_shape() {
        let json = r#"{"tag_name":"v0.17.0","assets":[{"name":"qjs-windows-x86_64.exe",
            "browser_download_url":"https://github.com/quickjs-ng/quickjs/releases/download/v0.17.0/qjs-windows-x86_64.exe",
            "size":2151363,"digest":"sha256:2aeabf0092c3262d6b2609824418f7dd7ed1f1df939f73b2b15645230cac0d77"}]}"#;
        let release: Release = serde_json::from_str(json).unwrap();
        let asset = pick_asset(&release, AssetRule::Exact("qjs-windows-x86_64.exe")).unwrap();
        assert_eq!(asset.size, 2_151_363);
        assert!(parse_digest(asset.digest.as_deref()).is_ok());
    }
}
