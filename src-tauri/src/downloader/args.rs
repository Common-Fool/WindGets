//! Строка yt-dlp, разбор его вывода и причин отказа — без процессов.
//!
//! Строка проверена живьём: 720p и 1080p берут H.264 со звуком AAC,
//! «лучшее» — что есть (на YouTube это 4K AV1 с Opus), звук — MP3 через
//! ffmpeg. yt-dlp пишет наши метки `@@` в оба потока: прогресс и имя файла в
//! stdout, шаги обработки в stderr, — поэтому читаются оба.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    #[default]
    Video,
    Audio,
}

/// Потолок видео.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Quality {
    P720,
    #[default]
    P1080,
    Best,
}

/// Куда что лежит.
pub struct Paths<'a> {
    pub ffmpeg: &'a Path,
    pub quickjs: &'a Path,
    /// Рабочая папка: сюда качается и склеивается, отсюда готовое переезжает.
    pub work: &'a Path,
    pub cache: &'a Path,
}

/// Полная строка yt-dlp.
pub fn ytdlp_args(mode: Mode, quality: Quality, paths: &Paths<'_>, url: &str) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();
    let mut push = |value: &str| args.push(OsString::from(value));

    for flag in [
        // Чужой конфиг yt-dlp в профиле человека не должен менять наше поведение
        "--ignore-config",
        "--no-playlist",
        // Ссылка на сам плейлист качает только первый ролик
        "-I", "1",
        "--encoding", "utf-8",
        "--color", "never",
        "--newline",
        "--progress",
        "--progress-delta", "0.5",
        // `--print` сам по себе включает «не качать»
        "--no-simulate",
        "--no-mtime",
        "--windows-filenames",
        "--socket-timeout", "30",
        "--progress-template",
        "download:@@p|%(progress.status)s|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(progress.speed)s|%(progress.eta)s|%(info.format_id)s",
        "--progress-template",
        "postprocess:@@pp|%(progress.status)s|%(progress.postprocessor)s",
        "--print", "before_dl:@@t|%(format_id)s|%(title)s",
        "--print", "after_move:@@f|%(filepath)s",
    ] {
        push(flag);
    }

    let mut pair = |flag: &str, value: OsString| {
        args.push(OsString::from(flag));
        args.push(value);
    };
    pair("--cache-dir", paths.cache.as_os_str().to_owned());
    pair("--ffmpeg-location", paths.ffmpeg.as_os_str().to_owned());
    let mut runtime = OsString::from("quickjs:");
    runtime.push(paths.quickjs.as_os_str());
    pair("--js-runtimes", runtime);
    let mut home = OsString::from("home:");
    home.push(paths.work.as_os_str());
    pair("-P", home);
    let mut temp = OsString::from("temp:");
    temp.push(paths.work.as_os_str());
    pair("-P", temp);
    pair("-o", OsString::from("%(title).150B.%(ext)s"));

    let mut push = |value: &str| args.push(OsString::from(value));
    match (mode, quality) {
        (Mode::Audio, _) => {
            for flag in ["-f", "ba/b", "-x", "--audio-format", "mp3", "--audio-quality", "192K"] {
                push(flag);
            }
        }
        (Mode::Video, Quality::Best) => {
            for flag in ["-f", "bv*+ba/b", "--merge-output-format", "mp4", "--remux-video", "mp4"] {
                push(flag);
            }
        }
        (Mode::Video, cap) => {
            let height = if cap == Quality::P720 { 720 } else { 1080 };
            // H.264 со звуком m4a — то, что откроет любой плеер; дальше по
            // цепочке — что есть не выше потолка, и в конце — что есть вообще.
            // `<=?` пропускает форматы без известной высоты: у сайтов, кроме
            // YouTube, её часто нет
            let chain = format!(
                "bv*[height<=?{height}][vcodec^=avc1]+ba[ext=m4a]/bv*[height<=?{height}][vcodec^=avc1]+ba/bv*[height<=?{height}]+ba/b[height<=?{height}]/bv*+ba/b"
            );
            for flag in ["-f", chain.as_str(), "--merge-output-format", "mp4", "--remux-video", "mp4"] {
                push(flag);
            }
        }
    }

    // Ссылка после `--`: что бы в ней ни стояло, флагом она не прочтётся
    push("--");
    push(url);
    args
}

/// Строка вывода, которую понял разбор.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Перед скачиванием: какие форматы взяты и как называется ролик.
    Title { format_id: String, title: String },
    Progress {
        finished: bool,
        downloaded: Option<u64>,
        total: Option<u64>,
        speed: Option<f64>,
        eta: Option<u64>,
        format_id: String,
    },
    /// Шаг обработки после скачивания: склейка, перепаковка, MP3.
    Step { started: bool, name: String },
    /// Готовый файл в рабочей папке.
    File(PathBuf),
}

fn number(value: &str) -> Option<f64> {
    value.trim().parse::<f64>().ok().filter(|number| number.is_finite() && *number >= 0.0)
}

/// Разбирает строку с меткой `@@`. Остальное — не наше.
pub fn parse_line(line: &str) -> Option<Event> {
    let line = line.trim_end();
    if let Some(rest) = line.strip_prefix("@@t|") {
        // Название может содержать `|` — режем только первую черту
        let (format_id, title) = rest.split_once('|')?;
        return Some(Event::Title { format_id: format_id.to_owned(), title: title.to_owned() });
    }
    if let Some(rest) = line.strip_prefix("@@p|") {
        let fields: Vec<&str> = rest.split('|').collect();
        if fields.len() < 7 {
            return None;
        }
        let total = number(fields[2]).or_else(|| number(fields[3]));
        return Some(Event::Progress {
            finished: fields[0] == "finished",
            downloaded: number(fields[1]).map(|bytes| bytes as u64),
            total: total.map(|bytes| bytes as u64),
            speed: number(fields[4]),
            eta: number(fields[5]).map(|seconds| seconds as u64),
            format_id: fields[6].to_owned(),
        });
    }
    if let Some(rest) = line.strip_prefix("@@pp|") {
        let (status, name) = rest.split_once('|')?;
        return Some(Event::Step { started: status == "started", name: name.to_owned() });
    }
    if let Some(path) = line.strip_prefix("@@f|") {
        return (!path.is_empty()).then(|| Event::File(PathBuf::from(path)));
    }
    None
}

/// Какая часть по счёту качается: у видео со звуком их две (`137+140`).
pub fn part_of(format_ids: &str, current: &str) -> Option<(usize, usize)> {
    let parts: Vec<&str> = format_ids.split('+').collect();
    let index = parts.iter().position(|part| *part == current)?;
    Some((index + 1, parts.len()))
}

/// Почему загрузка не удалась. Код, а не текст — карточка переводит сама.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    /// Сайт поменялся, и этот yt-dlp его больше не понимает: лечится свежим.
    Broken,
    /// Нужен вход: закрытое, для подписчиков, по возрасту. Без cookies этого
    /// не скачать — и делать этого мы не будем.
    SignIn,
    /// «Подтвердите, что вы не бот»: YouTube не пускает адрес — почти всегда
    /// VPN или прокси. Пишет он это словами «Sign in…», но вход тут ни при
    /// чём, лечится выключенным VPN (22 октября, слово пользователя).
    BotCheck,
    Unavailable,
    Unsupported,
    Network,
    Failed,
}

/// Разбирает жалобы yt-dlp. Поломка сайта проверяется первой: только она
/// лечится обновлением, и пропустить её за другой причиной значило бы не
/// попробовать единственное лекарство.
pub fn classify_error(lines: &[String]) -> Reason {
    let errors: Vec<&str> = lines
        .iter()
        .map(String::as_str)
        .filter(|line| line.contains("ERROR:"))
        .collect();
    let text = if errors.is_empty() {
        lines.join("\n")
    } else {
        errors.join("\n")
    };
    let has = |markers: &[&str]| markers.iter().any(|marker| text.contains(marker));

    if has(&[
        "Confirm you are on the latest version",
        "please report this issue",
        "nsig extraction failed",
        "Signature extraction failed",
        "Requested format is not available",
        "HTTP Error 403",
    ]) {
        Reason::Broken
    } else if has(&["not a bot", "not a robot"]) {
        Reason::BotCheck
    } else if has(&["Sign in", "sign in", "Private video", "members-only", "confirm your age"]) {
        Reason::SignIn
    } else if has(&["Unsupported URL"]) {
        Reason::Unsupported
    } else if has(&["unavailable", "has been removed", "not available in your country", "does not exist"]) {
        Reason::Unavailable
    } else if has(&[
        "Unable to download webpage",
        "getaddrinfo failed",
        "Failed to resolve",
        "timed out",
        "Connection refused",
        "Connection reset",
    ]) {
        Reason::Network
    } else {
        Reason::Failed
    }
}

/// Последняя жалоба коротко: без `ERROR:` и без `[youtube] <id>:` в начале.
pub fn detail(lines: &[String]) -> Option<String> {
    let line = lines.iter().rev().find(|line| line.contains("ERROR:"))?;
    let mut text = line.split_once("ERROR:").map_or(line.as_str(), |(_, rest)| rest).trim();
    if text.starts_with('[') {
        if let Some((_, rest)) = text.split_once("]: ").or_else(|| text.split_once("] ")) {
            text = rest;
        }
        if let Some((head, rest)) = text.split_once(": ") {
            if !head.contains(' ') {
                text = rest;
            }
        }
    }
    let mut short: String = text.chars().take(160).collect();
    if text.chars().count() > 160 {
        short.push('…');
    }
    Some(short)
}

/// Жалоба для журнала — без хвостов ссылок.
///
/// yt-dlp повторяет адрес в отказе целиком («Unsupported URL: …»), а в
/// строке запроса бывают токены и ключи доступа: журнал лежит на диске, и
/// человек пересылает его, когда что-то сломалось. Адрес оставляем до `?` и
/// `#` — сайт и путь для разбора хватает, — а данные для входа перед `@`
/// убираем совсем.
pub fn without_secrets(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("http://").into_iter().chain(rest.find("https://")).min() {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let end = tail.find(char::is_whitespace).unwrap_or(tail.len());
        let link = &tail[..end];
        let link = &link[..link.find(['?', '#']).unwrap_or(link.len())];
        let (scheme, after) = link.split_once("://").unwrap_or(("", link));
        let host_end = after.find('/').unwrap_or(after.len());
        let after = match after[..host_end].rfind('@') {
            Some(sign) => &after[sign + 1..],
            None => after,
        };
        out.push_str(scheme);
        out.push_str("://");
        out.push_str(after);
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(mode: Mode, quality: Quality) -> Vec<String> {
        let paths = Paths {
            ffmpeg: Path::new("C:\\tools\\ffmpeg\\ffmpeg.exe"),
            quickjs: Path::new("C:\\tools\\quickjs\\qjs.exe"),
            work: Path::new("C:\\tools\\work\\1"),
            cache: Path::new("C:\\tools\\yt-dlp\\cache"),
        };
        ytdlp_args(mode, quality, &paths, "https://www.youtube.com/watch?v=jNQXAC9IVRw")
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn the_url_comes_last_after_a_separator() {
        let args = build(Mode::Video, Quality::P1080);
        let separator = args.iter().position(|arg| arg == "--").unwrap();
        assert_eq!(separator, args.len() - 2);
        assert!(args.last().unwrap().starts_with("https://"));
        assert!(args.contains(&"--ignore-config".to_owned()));
        assert!(args.contains(&"quickjs:C:\\tools\\quickjs\\qjs.exe".to_owned()));
        assert!(args.contains(&"C:\\tools\\ffmpeg\\ffmpeg.exe".to_owned()));
        assert!(args.contains(&"home:C:\\tools\\work\\1".to_owned()));
    }

    #[test]
    fn the_height_cap_reaches_the_format_chain() {
        let chain = |quality| {
            let args = build(Mode::Video, quality);
            args[args.iter().position(|arg| arg == "-f").unwrap() + 1].clone()
        };
        assert!(chain(Quality::P720).contains("height<=?720"));
        assert!(chain(Quality::P1080).contains("height<=?1080"));
        assert!(chain(Quality::P1080).starts_with("bv*[height<=?1080][vcodec^=avc1]+ba[ext=m4a]"));
        assert_eq!(chain(Quality::Best), "bv*+ba/b");
    }

    #[test]
    fn audio_mode_extracts_mp3() {
        let args = build(Mode::Audio, Quality::Best);
        for flag in ["-x", "--audio-format", "mp3", "ba/b"] {
            assert!(args.contains(&flag.to_owned()), "нет {flag}");
        }
        assert!(!args.contains(&"--merge-output-format".to_owned()));
    }

    /// Строки — ровно такие, какие yt-dlp напечатал на живой загрузке.
    #[test]
    fn live_lines_parse() {
        assert_eq!(
            parse_line("@@t|133+140|Me at the zoo"),
            Some(Event::Title { format_id: "133+140".to_owned(), title: "Me at the zoo".to_owned() })
        );
        assert_eq!(
            parse_line("@@p|downloading|1024|433081|NA|145700.76993011739|2|133"),
            Some(Event::Progress {
                finished: false,
                downloaded: Some(1024),
                total: Some(433_081),
                speed: Some(145_700.769_930_117_39),
                eta: Some(2),
                format_id: "133".to_owned(),
            })
        );
        assert_eq!(
            parse_line("@@pp|started|Merger"),
            Some(Event::Step { started: true, name: "Merger".to_owned() })
        );
        assert_eq!(
            parse_line("@@f|C:\\work\\Me at the zoo.mp4"),
            Some(Event::File(PathBuf::from("C:\\work\\Me at the zoo.mp4")))
        );
        assert_eq!(parse_line("[youtube] Extracting URL"), None);
    }

    /// `|` в названии не рвёт его, а неизвестные поля прогресса — `NA`.
    #[test]
    fn awkward_lines_parse() {
        assert_eq!(
            parse_line("@@t|22|Live | Official"),
            Some(Event::Title { format_id: "22".to_owned(), title: "Live | Official".to_owned() })
        );
        let Some(Event::Progress { total, speed, eta, .. }) =
            parse_line("@@p|downloading|500|NA|1000.5|NA|NA|22")
        else {
            panic!("строка прогресса не разобралась");
        };
        assert_eq!((total, speed, eta), (Some(1000), None, None));
    }

    #[test]
    fn parts_are_counted_from_the_format_list() {
        assert_eq!(part_of("133+140", "133"), Some((1, 2)));
        assert_eq!(part_of("133+140", "140"), Some((2, 2)));
        assert_eq!(part_of("251", "251"), Some((1, 1)));
        assert_eq!(part_of("251", "18"), None);
    }

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_owned).collect()
    }

    /// Образцы сняты с живого yt-dlp 2026.08.19.
    #[test]
    fn errors_are_classified_from_live_samples() {
        assert_eq!(
            classify_error(&lines("WARNING: [generic] Falling back on generic information extractor\nERROR: Unsupported URL: https://example.com/")),
            Reason::Unsupported
        );
        assert_eq!(
            classify_error(&lines("ERROR: [youtube] aaaaaaaaaaa: This video is unavailable")),
            Reason::Unavailable
        );
        assert_eq!(
            classify_error(&lines("ERROR: [generic] video: Unable to download webpage: HTTPSConnection(host='x.invalid', port=443): Failed to resolve 'x.invalid' ([Errno 11001] getaddrinfo failed)")),
            Reason::Network
        );
        // Проверка на бота — не вход, хоть и начинается с «Sign in»
        assert_eq!(
            classify_error(&lines("ERROR: [youtube] abc: Sign in to confirm you’re not a bot. Use --cookies-from-browser or --cookies for the authentication.")),
            Reason::BotCheck
        );
        assert_eq!(
            classify_error(&lines("ERROR: [youtube] abc: Sign in to confirm you're not a bot")),
            Reason::BotCheck
        );
        assert_eq!(
            classify_error(&lines("ERROR: [youtube] abc: Private video. Sign in if you've been granted access to this video")),
            Reason::SignIn
        );
        assert_eq!(classify_error(&lines("ERROR: something new")), Reason::Failed);
    }

    /// Поломка сайта главнее: только она лечится обновлением.
    #[test]
    fn a_broken_site_wins_over_other_markers() {
        let text = "ERROR: [youtube] abc: Unable to download webpage: HTTP Error 403: Forbidden (caused by <HTTPError 403>); please report this issue";
        assert_eq!(classify_error(&lines(text)), Reason::Broken);
    }

    #[test]
    fn the_detail_is_the_last_complaint_without_prefixes() {
        assert_eq!(
            detail(&lines("WARNING: x\nERROR: [youtube] aaaaaaaaaaa: This video is unavailable")).as_deref(),
            Some("This video is unavailable")
        );
        assert_eq!(
            detail(&lines("ERROR: Unsupported URL: https://example.com/")).as_deref(),
            Some("Unsupported URL: https://example.com/")
        );
        assert_eq!(detail(&lines("WARNING: only")), None);
    }

    #[test]
    fn url_punctuation_cannot_leave_a_secret_in_the_journal() {
        for tail in ["?name=O'Neil&token=PRIVATE_TOKEN", "#name=O'Neil&token=PRIVATE_TOKEN", "?name=one|two&token=PRIVATE_TOKEN", "?name=one\"two&token=PRIVATE_TOKEN"] {
            let line = format!("ERROR: Unsupported URL: https://example.org/video{tail} next");
            assert_eq!(without_secrets(&line), "ERROR: Unsupported URL: https://example.org/video next");
        }
        assert_eq!(
            without_secrets("https://example.org/O'Neil?token=PRIVATE_TOKEN"),
            "https://example.org/O'Neil"
        );
    }

    #[test]
    fn the_journal_gets_links_without_their_secrets() {
        let line = "ERROR: Unsupported URL: https://user:pass@example.com/video/1?token=abc#t=5 | next";
        assert_eq!(without_secrets(line), "ERROR: Unsupported URL: https://example.com/video/1 | next");
        assert_eq!(without_secrets("нет ссылок"), "нет ссылок");
        assert_eq!(without_secrets("http://a.b/c?x https://d.e/f"), "http://a.b/c https://d.e/f");
    }
}
