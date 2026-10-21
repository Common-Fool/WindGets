//! Что внутри файла — по выводу `ffmpeg -i`, без ffprobe.
//!
//! ffprobe отдал бы то же самое JSON-ом, но это второй exe в сто пять
//! мегабайт ради одного вопроса. `ffmpeg -i` печатает формат, длину и потоки
//! строками, неизменными годами (по ним же в запасе читает файлы yt-dlp), и
//! конвертеру из них нужно немногое: есть ли видео, есть ли звук, размеры,
//! частота кадров, пиксельный формат и частота звука.
//!
//! Образцы строк в тестах сняты с настоящего ffmpeg 9.0.2.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq)]
pub struct Video {
    pub codec: String,
    /// `yuv420p`, `rgba`, `bgra`… Без него не понять, есть ли прозрачность.
    pub pix_fmt: Option<String>,
    pub width: u32,
    pub height: u32,
    pub fps: Option<f64>,
    /// Номер потока во входе (`Stream #0:2`). Кодирование берёт его, а не
    /// «первое видео»: обложка (`attached pic`) тоже видео, и стоит она чаще
    /// всего первой — `0:v:0` выбрал бы её вместо ролика.
    pub stream: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Audio {
    pub sample_rate: Option<u32>,
    /// `mono`, `stereo`, `5.1`…
    pub layout: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Probe {
    /// Имя формата из строки `Input #0, <формат>, from …`.
    pub format: String,
    pub duration: Option<f64>,
    /// Первое видео, кроме обложки (`attached pic`).
    pub video: Option<Video>,
    pub audio: Option<Audio>,
}

/// Чем файл является для конвертера.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Video,
    Audio,
    Image,
}

/// Разбирает вывод `ffmpeg -hide_banner -i <файл>`. `None` — ffmpeg файла не
/// прочёл вовсе.
pub fn parse(lines: &[&str]) -> Option<Probe> {
    let mut probe: Option<Probe> = None;

    for raw in lines {
        let line = raw.trim();

        if let Some(rest) = line.strip_prefix("Input #0, ") {
            let format = rest.split(", from '").next().unwrap_or(rest).to_owned();
            probe = Some(Probe { format, duration: None, video: None, audio: None });
            continue;
        }
        let Some(probe) = probe.as_mut() else {
            continue;
        };

        if let Some(rest) = line.strip_prefix("Duration: ") {
            let value = rest.split(',').next().unwrap_or("").trim();
            probe.duration = parse_clock(value);
        } else if line.starts_with("Stream #0:") {
            if let Some(description) = after(line, ": Video: ") {
                if probe.video.is_none() && !line.contains("(attached pic)") {
                    probe.video = parse_video(description).map(|video| Video { stream: stream_number(line), ..video });
                }
            } else if let Some(description) = after(line, ": Audio: ") {
                if probe.audio.is_none() {
                    probe.audio = Some(parse_audio(description));
                }
            }
        }
    }

    probe
}

/// `Stream #0:2[0x1](und): Video: …` → 2.
fn stream_number(line: &str) -> Option<u32> {
    let rest = line.strip_prefix("Stream #0:")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

fn after<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    line.find(marker).map(|at| &line[at + marker.len()..])
}

/// `00:01:23.45` в секунды. `N/A` и прочее — `None`.
fn parse_clock(value: &str) -> Option<f64> {
    let mut seconds = 0.0;
    for part in value.split(':') {
        let number: f64 = part.parse().ok()?;
        seconds = seconds * 60.0 + number;
    }
    (seconds.is_finite() && seconds >= 0.0).then_some(seconds)
}

/// Режет описание потока по запятым верхнего уровня.
///
/// Запятые бывают и внутри скобок — `rgba(pc, gbr/unknown/unknown)`, — и
/// простой `split(',')` рвал бы пиксельный формат пополам.
fn split_top(description: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;

    for (at, symbol) in description.char_indices() {
        match symbol {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(description[start..at].trim());
                start = at + 1;
            }
            _ => {}
        }
    }
    parts.push(description[start..].trim());
    parts
}

/// `h264 (High) (avc1 / 0x31637661), yuv420p(progressive), 720x1280 [SAR 1:1
/// DAR 9:16], 3334 kb/s, 30 fps, 30 tbr, 15360 tbn (default)`.
fn parse_video(description: &str) -> Option<Video> {
    let parts = split_top(description);
    let codec = parts.first()?.split_whitespace().next()?.to_owned();

    let mut width = 0;
    let mut height = 0;
    let mut pix_fmt = None;
    let mut fps = None;

    for (index, part) in parts.iter().enumerate().skip(1) {
        if let Some((w, h)) = dimensions(part) {
            width = w;
            height = h;
        } else if let Some(number) = part.strip_suffix(" fps") {
            fps = number.trim().parse::<f64>().ok().filter(|fps| *fps > 0.0);
        } else if index == 1 {
            // Сразу за кодеком — пиксельный формат, с уточнением в скобках
            let name = part.split('(').next().unwrap_or("").trim();
            if !name.is_empty() && !name.contains(' ') {
                pix_fmt = Some(name.to_owned());
            }
        }
    }

    (width > 0 && height > 0).then_some(Video { codec, pix_fmt, width, height, fps, stream: None })
}

/// `1920x1080` или `1920x1080 [SAR 1:1 DAR 16:9]`.
fn dimensions(part: &str) -> Option<(u32, u32)> {
    let token = part.split_whitespace().next()?;
    let (w, h) = token.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

/// `aac (LC) (mp4a / 0x6134706D), 44100 Hz, mono, fltp, 69 kb/s (default)`.
fn parse_audio(description: &str) -> Audio {
    let parts = split_top(description);
    let mut audio = Audio { sample_rate: None, layout: None };

    if let Some(at) = parts.iter().position(|part| part.ends_with(" Hz")) {
        audio.sample_rate = parts[at].trim_end_matches(" Hz").trim().parse().ok();
        audio.layout = parts.get(at + 1).map(|layout| (*layout).to_owned());
    }
    audio
}

/// Нужен ли подсчёт кадров, чтобы отличить картинку от ролика.
///
/// Только у GIF: статичный и анимированный там один формат, а по длине их не
/// различить надёжно — у двухкадрового длина та же, что у одной картинки.
pub fn needs_frame_count(probe: &Probe) -> bool {
    probe.format == "gif"
}

/// Последнее `frame=N` из `-progress` прохода с `-c copy`.
pub fn parse_frames(lines: &[&str]) -> Option<u64> {
    lines
        .iter()
        .rev()
        .find_map(|line| line.trim().strip_prefix("frame=")?.trim().parse().ok())
}

/// Видео, звук или картинка.
pub fn classify(probe: &Probe, frames: Option<u64>) -> Option<Kind> {
    match (&probe.video, &probe.audio) {
        (Some(_), _) if is_still(probe, frames) => Some(Kind::Image),
        (Some(_), _) => Some(Kind::Video),
        (None, Some(_)) => Some(Kind::Audio),
        (None, None) => None,
    }
}

fn is_still(probe: &Probe, frames: Option<u64>) -> bool {
    let format = probe.format.as_str();
    // Одиночные картинки ffmpeg читает «трубой» (`png_pipe`, `webp_pipe`) или
    // как `image2` (JPEG)
    if format.ends_with("_pipe") || format == "image2" {
        return true;
    }
    if format == "gif" {
        return frames == Some(1);
    }
    // Снимок в контейнере — HEIC или AVIF с телефона: видео без звука и без
    // длины. Настоящий ролик без звука длину имеет
    probe.audio.is_none() && probe.duration.is_none_or(|duration| duration <= 0.1)
}

/// Есть ли в пиксельном формате прозрачность.
pub fn has_alpha(pix_fmt: Option<&str>) -> bool {
    let Some(name) = pix_fmt else {
        return false;
    };
    ["rgba", "bgra", "argb", "abgr", "ya", "yuva", "gbrap", "pal8"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(text: &str) -> Probe {
        let lines: Vec<&str> = text.lines().collect();
        parse(&lines).expect("вывод ffmpeg разбирается")
    }

    const PORTRAIT: &str = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'portrait.mp4':
  Metadata:
    major_brand     : isom
  Duration: 00:00:04.00, start: 0.000000, bitrate: 3415 kb/s
  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(progressive), 720x1280 [SAR 1:1 DAR 9:16], 3334 kb/s, 30 fps, 30 tbr, 15360 tbn (default)
  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706D), 44100 Hz, mono, fltp, 69 kb/s (default)
At least one output file must be specified";

    #[test]
    fn a_video_with_sound_is_read_whole() {
        let probe = probe(PORTRAIT);
        assert_eq!(probe.format, "mov,mp4,m4a,3gp,3g2,mj2");
        assert_eq!(probe.duration, Some(4.0));
        let video = probe.video.clone().unwrap();
        assert_eq!((video.width, video.height), (720, 1280));
        assert_eq!(video.pix_fmt.as_deref(), Some("yuv420p"));
        assert_eq!(video.fps, Some(30.0));
        assert_eq!(probe.audio.clone().unwrap().layout.as_deref(), Some("mono"));
        assert_eq!(classify(&probe, None), Some(Kind::Video));
    }

    /// Обложка в MP3 — видеопоток с пометкой `attached pic`; файл при этом
    /// звуковой, а не видео.
    #[test]
    fn an_mp3_with_a_cover_is_audio() {
        let probe = probe("Input #0, mp3, from 'cover.mp3':
  Duration: 00:00:03.00, start: 0.025056, bitrate: 67 kb/s
  Stream #0:0: Audio: mp3 (mp3float), 44100 Hz, mono, fltp, 64 kb/s, start 0.025057
  Stream #0:1: Video: png, rgba(pc, gbr/unknown/unknown), 300x200 [SAR 1:1 DAR 3:2], 90k tbr, 90k tbn, start 0.025056 (attached pic)");
        assert!(probe.video.is_none());
        assert_eq!(classify(&probe, None), Some(Kind::Audio));
    }

    #[test]
    fn hi_res_surround_sound_is_read() {
        let probe = probe("Input #0, flac, from 'hires51.flac':
  Duration: 00:00:05.00, start: 0.000000, bitrate: 1541 kb/s
  Stream #0:0: Audio: flac, 96000 Hz, 5.1, s16");
        let audio = probe.audio.clone().unwrap();
        assert_eq!(audio.sample_rate, Some(96_000));
        assert_eq!(audio.layout.as_deref(), Some("5.1"));
        assert_eq!(classify(&probe, None), Some(Kind::Audio));
    }

    /// Запятые в скобках пиксельного формата не рвут его пополам.
    #[test]
    fn a_png_with_alpha_is_an_image_with_alpha() {
        let probe = probe("Input #0, png_pipe, from 'alpha.png':
  Duration: N/A, bitrate: N/A
  Stream #0:0: Video: png, rgba(pc, gbr/unknown/unknown), 300x200 [SAR 1:1 DAR 3:2], 25 fps, 25 tbr, 25 tbn");
        assert_eq!(probe.duration, None);
        let video = probe.video.clone().unwrap();
        assert_eq!(video.pix_fmt.as_deref(), Some("rgba"));
        assert!(has_alpha(video.pix_fmt.as_deref()));
        assert_eq!(classify(&probe, None), Some(Kind::Image));
    }

    #[test]
    fn a_jpeg_is_an_image_without_alpha() {
        let probe = probe("Input #0, image2, from 'photo.jpg':
  Duration: 00:00:00.04, start: 0.000000, bitrate: 15771 kb/s
  Stream #0:0: Video: mjpeg (Baseline), yuvj420p(pc, bt470bg/unknown/unknown), 1600x1200 [SAR 1:1 DAR 4:3], 25 fps, 25 tbr, 25 tbn");
        assert_eq!(classify(&probe, None), Some(Kind::Image));
        assert!(!has_alpha(probe.video.unwrap().pix_fmt.as_deref()));
    }

    /// Статичный и анимированный GIF — один формат; различает их число кадров.
    #[test]
    fn a_gif_is_told_by_its_frames() {
        let probe = probe("Input #0, gif, from 'anim.gif':
  Duration: 00:00:02.00, start: 0.000000, bitrate: 477 kb/s
  Stream #0:0: Video: gif, bgra, 320x240 [SAR 64:64 DAR 4:3], 10 fps, 10 tbr, 100 tbn");
        assert!(needs_frame_count(&probe));
        assert_eq!(classify(&probe, Some(20)), Some(Kind::Video));
        assert_eq!(classify(&probe, Some(1)), Some(Kind::Image));
        assert_eq!(probe.video.unwrap().pix_fmt.as_deref(), Some("bgra"));
    }

    #[test]
    fn frames_come_from_the_last_progress_block() {
        assert_eq!(parse_frames(&["frame=3", "progress=continue", "frame=20", "progress=end"]), Some(20));
        assert_eq!(parse_frames(&["progress=end"]), None);
    }

    /// Файл, который ffmpeg не прочёл, — не видео и не звук, а отказ.
    #[test]
    fn an_unreadable_file_gives_nothing() {
        assert_eq!(parse(&["notes.txt: Invalid data found when processing input"]), None);
    }

    #[test]
    fn clock_values_parse() {
        assert_eq!(parse_clock("00:01:23.50"), Some(83.5));
        assert_eq!(parse_clock("N/A"), None);
    }
}
