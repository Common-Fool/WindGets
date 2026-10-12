//! Строки ffmpeg, разбор прогресса и обрезки — всё чистое, без процессов.
//!
//! Каждая строка проверена на неудобных входах до того, как попасть сюда:
//! вертикальное видео с телефона, повёрнутое метаданными, нечётный размер,
//! 10 бит, звук 96 кГц 5.1, PNG с прозрачностью, анимированный и статичный GIF.

use std::ffi::OsString;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::probe::{has_alpha, Kind, Probe};

/// Во что превращаем.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Target {
    Mp4,
    Gif,
    Mp3,
    Png,
    Jpg,
}

impl Target {
    pub fn ext(self) -> &'static str {
        match self {
            Target::Mp4 => "mp4",
            Target::Gif => "gif",
            Target::Mp3 => "mp3",
            Target::Png => "png",
            Target::Jpg => "jpg",
        }
    }
}

/// Качество видео: `crf` x264, `cq` NVENC и битрейт звука AAC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Quality {
    High,
    #[default]
    Balanced,
    Small,
}

impl Quality {
    fn crf(self) -> &'static str {
        match self {
            Quality::High => "18",
            Quality::Balanced => "23",
            Quality::Small => "28",
        }
    }

    /// Ступени NVENC — на четыре выше x264, и это замер, а не догадка: на
    /// одном и том же ролике 1080p `cq` 22 / 27 / 32 дали те же размер и SSIM,
    /// что x264 `medium` с `crf` 18 / 23 / 28 (29,1 МБ и 0,9955 против 31,9 и
    /// 0,9957; 17,2 и 0,9927 против 18,4 и 0,9921; 9,8 и 0,986 против 7,8 и
    /// 0,984). Одно и то же число у двух кодировщиков — разное качество.
    fn cq(self) -> &'static str {
        match self {
            Quality::High => "22",
            Quality::Balanced => "27",
            Quality::Small => "32",
        }
    }

    fn audio(self) -> &'static str {
        match self {
            Quality::High => "192k",
            Quality::Balanced => "160k",
            Quality::Small => "128k",
        }
    }
}

/// Чем кодировать H.264.
///
/// ВАЖНО: **видеокарта — если она есть, процессор — если нет.** Замер на
/// ролике 1080p в 20 секунд: x264 `medium` — 6,9 с, `veryfast` — 2,6 с, NVENC
/// — 1,9 с при том же качестве; на 4K в HEVC 10 бит и в AV1 — 12, 5,8 и 3,4 с.
/// Кодировщик выбирается пробой (`nvenc_probe`), а не по названию видеокарты:
/// NVENC есть в любой сборке ffmpeg, а работает только с драйвером NVIDIA.
/// AMF и QSV не берём — проверить их на этой машине нечем, а непроверенная
/// строка кодирования хуже медленной.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoder {
    Nvenc,
    X264,
}

impl Encoder {
    fn args(self, quality: Quality) -> Vec<&'static str> {
        match self {
            // `veryfast`, а не `medium`: в 2,7 раза быстрее на 1080p и вдвое
            // на 4K, а файл при том же `crf` даже меньше — ценой SSIM 0,9907
            // против 0,9921. Конвертер в виджете ждут, а не оставляют на ночь
            Encoder::X264 => vec!["-c:v", "libx264", "-preset", "veryfast", "-crf", quality.crf()],
            // `p5` — середина ряда: `p7` на пятую часть медленнее при почти
            // том же размере
            Encoder::Nvenc => vec![
                "-c:v", "h264_nvenc", "-preset", "p5", "-tune", "hq",
                "-rc", "vbr", "-cq", quality.cq(), "-b:v", "0",
            ],
        }
    }
}

/// Строка пробы NVENC: один чёрный кадр теми же ключами, что у настоящего
/// кодирования, — старый драйвер без `p5` отказал бы и тут. Без драйвера
/// NVIDIA ffmpeg отказывает за десятки миллисекунд, с ним отвечает за пятую
/// долю секунды.
pub fn nvenc_probe() -> Vec<&'static str> {
    let mut args = vec![
        "-hide_banner", "-nostdin", "-loglevel", "error",
        "-f", "lavfi", "-i", "color=c=black:s=256x256:r=1:d=1", "-frames:v", "1",
    ];
    args.extend(Encoder::Nvenc.args(Quality::Balanced));
    args.extend(["-f", "null", "-"]);
    args
}

/// Потолки, из которых выбирают в карточке. Любое другое число из правленного
/// руками вызова приводится к ближайшему разумному.
pub const VIDEO_SIZES: [u32; 4] = [0, 1080, 720, 480];
pub const BITRATES: [u32; 3] = [320, 192, 128];
/// «Как есть» (ноль) у GIF в этом ряду нет: GIF во весь кадр 1080p — это
/// десятки мегабайт. Появляется он, только когда исходник уже меньше какой-то
/// из ширин (`gif_sizes`).
pub const GIF_SIZES: [u32; 3] = [320, 480, 640];
pub const GIF_RATES: [u32; 3] = [10, 15, 24];
pub const IMAGE_SIZES: [u32; 5] = [0, 2560, 1920, 1280, 640];

/// Потолки, которые хоть что-то уменьшат: меньше самой большой стороны в
/// пачке. Потолок выше исходника не делает ничего — увеличения нет, — а
/// предложенный в карточке читался бы как «растянуть до 1080p». «Как есть»
/// остаётся всегда. Размер неизвестен — предлагается весь ряд.
///
/// Выбранный, но отсеянный потолок карточка показывает как «как есть», и это
/// не подмена: ffmpeg с ним и так оставил бы размер исходника.
pub fn offered_sizes(sizes: &[u32], largest: Option<u32>) -> Vec<u32> {
    sizes
        .iter()
        .copied()
        .filter(|&size| size == 0 || largest.is_none_or(|largest| size < largest))
        .collect()
}

/// Ширины GIF: как у всех, но «как есть» появляется, только когда какая-то
/// ширина отсеялась — исходник и так маленький.
pub fn gif_sizes(largest: Option<u32>) -> Vec<u32> {
    let mut sizes = offered_sizes(&GIF_SIZES, largest);
    if sizes.len() < GIF_SIZES.len() {
        sizes.push(0);
    }
    sizes
}

/// Какой потолок пойдёт в работу: выбранный, если он в ряду, иначе «как
/// есть», а если и его в ряду нет — `fallback`.
///
/// Отсеянный потолок и «как есть» — одно и то же: ffmpeg не увеличивает, и
/// размер остался бы исходным. Обратный случай бывает только у GIF: «как
/// есть», выбранное для маленького ролика, большому ролику уже не предлагается,
/// и GIF во весь кадр 1080p вышел бы на десятки мегабайт.
pub fn chosen_size(value: u32, offered: &[u32], fallback: u32) -> u32 {
    if offered.contains(&value) {
        value
    } else if offered.contains(&0) {
        0
    } else {
        fallback
    }
}

/// Настройки пачки. Ноль в размерах — «как есть».
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Options {
    pub target: Option<Target>,
    pub quality: Quality,
    /// Потолок видео по короткой стороне: 1080, 720, 480.
    pub video_size: u32,
    pub bitrate: u32,
    /// Длинная сторона GIF.
    pub gif_size: u32,
    pub gif_fps: u32,
    /// Длинная сторона картинки.
    pub image_size: u32,
    /// Обрезка — как ввёл человек: `1:05`, `90`, пусто.
    pub from: String,
    pub to: String,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            target: None,
            quality: Quality::Balanced,
            video_size: 0,
            bitrate: 192,
            gif_size: 480,
            gif_fps: 15,
            image_size: 0,
            from: String::new(),
            to: String::new(),
        }
    }
}

impl Options {
    /// Числа — только из списков карточки.
    pub fn sane(mut self) -> Self {
        let pick = |value: u32, allowed: &[u32], fallback: u32| {
            if allowed.contains(&value) { value } else { fallback }
        };
        self.video_size = pick(self.video_size, &VIDEO_SIZES, 0);
        self.bitrate = pick(self.bitrate, &BITRATES, 192);
        self.gif_size = pick(self.gif_size, &[0, 320, 480, 640], 480);
        self.gif_fps = pick(self.gif_fps, &GIF_RATES, 15);
        self.image_size = pick(self.image_size, &IMAGE_SIZES, 0);
        self
    }
}

/// Во что можно превратить файл такого вида.
pub fn targets(kind: Kind, has_audio: bool) -> Vec<Target> {
    match kind {
        Kind::Video if has_audio => vec![Target::Mp4, Target::Gif, Target::Mp3],
        Kind::Video => vec![Target::Mp4, Target::Gif],
        Kind::Audio => vec![Target::Mp3],
        Kind::Image => vec![Target::Png, Target::Jpg],
    }
}

/// Цели для пачки — общие у всех её файлов.
///
/// Одна пачка — одни настройки: видео и картинки в ней вместе не
/// превратить ни во что, и карточка скажет об этом прямо.
pub fn batch_targets(each: &[Vec<Target>]) -> Vec<Target> {
    let Some((first, rest)) = each.split_first() else {
        return Vec::new();
    };
    first
        .iter()
        .copied()
        .filter(|target| rest.iter().all(|targets| targets.contains(target)))
        .collect()
}

/// Отрезок, который оставить.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trim {
    pub from: f64,
    /// `None` — до конца файла.
    pub length: Option<f64>,
}

/// Почему обрезка не годится.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TrimError {
    BadFrom,
    BadTo,
    EmptyRange,
}

/// Время, как его вводят люди: `90`, `1:05`, `1:02:03.5`, `1,5`. Пусто — `None`.
pub fn parse_timecode(text: &str) -> Result<Option<f64>, ()> {
    let text = text.trim().replace(',', ".");
    if text.is_empty() {
        return Ok(None);
    }

    let parts: Vec<&str> = text.split(':').collect();
    if parts.len() > 3 || parts.iter().any(|part| part.is_empty()) {
        return Err(());
    }

    let (whole, last) = parts.split_at(parts.len() - 1);
    let seconds: f64 = last[0].parse().map_err(|_| ())?;
    if !seconds.is_finite() || seconds < 0.0 || (!whole.is_empty() && seconds >= 60.0) {
        return Err(());
    }

    // Целые минуты (или часы и минуты) перед секундами
    let mut minutes = 0.0;
    for (index, part) in whole.iter().enumerate() {
        let number: u32 = part.parse().map_err(|_| ())?;
        // Минуты при часах — не больше 59; первое число — любое
        if index > 0 && number >= 60 {
            return Err(());
        }
        minutes = minutes * 60.0 + f64::from(number);
    }
    Ok(Some(minutes * 60.0 + seconds))
}

/// Отрезок из двух полей. `Ok(None)` — обрезки нет.
pub fn resolve_trim(from: &str, to: &str, duration: Option<f64>) -> Result<Option<Trim>, TrimError> {
    let from = parse_timecode(from).map_err(|()| TrimError::BadFrom)?.unwrap_or(0.0);
    let mut to = parse_timecode(to).map_err(|()| TrimError::BadTo)?;

    if from == 0.0 && to.is_none() {
        return Ok(None);
    }
    if let Some(duration) = duration {
        if from >= duration {
            return Err(TrimError::BadFrom);
        }
        // Конец дальше файла — это «до конца», а не ошибка
        to = to.map(|to| to.min(duration));
    }
    if let Some(to) = to {
        if to <= from {
            return Err(TrimError::EmptyRange);
        }
    }

    Ok(Some(Trim { from, length: to.map(|to| to - from) }))
}

/// Сколько секунд пойдёт в результат — знаменатель прогресса.
pub fn segment(duration: Option<f64>, trim: Option<Trim>) -> Option<f64> {
    match trim {
        Some(Trim { length: Some(length), .. }) => Some(length),
        Some(Trim { from, length: None }) => duration.map(|duration| duration - from),
        None => duration,
    }
    .filter(|seconds| *seconds > 0.0)
}

/// Время из строки `-progress`: `out_time_us=1500000`. `Some(None)` — ffmpeg
/// сам ещё не знает (`N/A`).
pub fn progress_seconds(line: &str) -> Option<Option<f64>> {
    let value = line.trim().strip_prefix("out_time_us=")?;
    Some(value.parse::<f64>().ok().map(|us| us / 1_000_000.0))
}

/// Доля готового, от нуля до единицы.
pub fn fraction(done: f64, segment: f64) -> f64 {
    if segment <= 0.0 {
        return 0.0;
    }
    (done / segment).clamp(0.0, 1.0)
}

/// Кадров в секунду для GIF: выбранное, но не больше, чем у исходника —
/// удвоенные кадры только раздули бы файл.
pub fn gif_fps(chosen: u32, source: Option<f64>) -> String {
    match source {
        Some(source) if source < f64::from(chosen) => format!("{source:.2}"),
        _ => chosen.to_string(),
    }
}

/// Размер под длинную сторону, без увеличения и с чётными сторонами
/// (JPEG в 4:2:0 делит цвет пополам по обеим осям).
pub fn fit_long_side(width: u32, height: u32, limit: u32) -> (u32, u32) {
    let (width, height) = if limit == 0 || width.max(height) <= limit {
        (width, height)
    } else if width >= height {
        (limit, (f64::from(height) * f64::from(limit) / f64::from(width)).round() as u32)
    } else {
        ((f64::from(width) * f64::from(limit) / f64::from(height)).round() as u32, limit)
    };
    let even = |side: u32| (side - side % 2).max(2);
    (even(width), even(height))
}

fn seconds(value: f64) -> String {
    format!("{value:.3}")
}

/// Масштаб по длинной стороне, без увеличения. `keep` — чем отвечать на
/// вторую сторону: `-1` любым числом, `-2` чётным.
fn long_side(limit: u32, keep: &str) -> String {
    format!(
        "scale=w='if(gte(iw,ih),min(iw,{limit}),{keep})':h='if(gte(iw,ih),{keep},min(ih,{limit}))':flags=lanczos"
    )
}

/// Полная строка ffmpeg для одного файла.
///
/// Вход и выход — отдельными словами, а не склеенной строкой: имена файлов с
/// пробелами и кавычками тут просто данные.
pub fn ffmpeg_args(
    target: Target,
    options: &Options,
    probe: &Probe,
    trim: Option<Trim>,
    encoder: Encoder,
    input: &Path,
    output: &Path,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "-hide_banner",
        "-nostdin",
        "-loglevel",
        "error",
        "-nostats",
        "-progress",
        "pipe:1",
        "-y",
    ]
    .map(OsString::from)
    .to_vec();
    let mut push = |values: &[&str]| args.extend(values.iter().map(OsString::from));

    // Отрезок: начало — до входа (поиск по входу точен, когда кодируем
    // заново), длина — после
    let trim = trim.filter(|_| matches!(target, Target::Mp4 | Target::Gif | Target::Mp3));
    if let Some(trim) = trim.filter(|trim| trim.from > 0.0) {
        push(&["-ss", &seconds(trim.from)]);
    }
    push(&["-i"]);
    args.push(input.as_os_str().to_owned());
    let mut push = |values: &[&str]| args.extend(values.iter().map(OsString::from));
    if let Some(length) = trim.and_then(|trim| trim.length) {
        push(&["-t", &seconds(length)]);
    }

    let video = probe.video.as_ref();
    match target {
        Target::Mp4 => {
            // Потолок — по КОРОТКОЙ стороне: вертикальное видео с телефона
            // при «720p» остаётся 720×1280, а не сплющивается в 405×720.
            // Выражения считаются после автоповорота, так что повёрнутое
            // метаданными тоже встаёт правильно
            let scale = match options.video_size {
                0 => "scale=trunc(iw/2)*2:trunc(ih/2)*2".to_owned(),
                size => format!(
                    "scale=w='if(gte(iw,ih),-2,trunc(min(iw,{size})/2)*2)':h='if(gte(iw,ih),trunc(min(ih,{size})/2)*2,-2)'"
                ),
            };
            push(&["-map", "0:v:0", "-map", "0:a:0?", "-sn", "-dn"]);
            push(&["-vf", &format!("{scale},format=yuv420p")]);
            push(&encoder.args(options.quality));
            push(&[
                "-c:a", "aac", "-b:a", options.quality.audio(),
                "-movflags", "+faststart", "-f", "mp4",
            ]);
        }
        Target::Gif => {
            let fps = gif_fps(options.gif_fps, video.and_then(|video| video.fps));
            // Ноль — «как есть»: исходник уже меньше любой из ширин
            let scale = match options.gif_size {
                0 => String::new(),
                size => format!("{},", long_side(size, "-1")),
            };
            let graph = format!(
                "[0:v:0]fps={fps},{scale}split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle"
            );
            push(&["-an", "-sn", "-dn", "-filter_complex", &graph, "-loop", "0", "-f", "gif"]);
        }
        Target::Mp3 => {
            push(&["-map", "0:a:0", "-vn", "-sn", "-dn", "-c:a", "libmp3lame"]);
            push(&["-b:a", &format!("{}k", options.bitrate)]);
            // LAME не берёт выше 48 кГц и больше двух каналов
            let audio = probe.audio.as_ref();
            if audio.and_then(|audio| audio.sample_rate).is_some_and(|rate| rate > 48_000) {
                push(&["-ar", "48000"]);
            }
            let layout = audio.and_then(|audio| audio.layout.as_deref());
            if !matches!(layout, Some("mono") | Some("stereo") | None) {
                push(&["-ac", "2"]);
            }
            push(&["-map_metadata", "0", "-id3v2_version", "3", "-f", "mp3"]);
        }
        Target::Png => {
            push(&["-frames:v", "1"]);
            if options.image_size > 0 {
                push(&["-vf", &long_side(options.image_size, "-1")]);
            }
            push(&["-c:v", "png", "-f", "image2", "-update", "1"]);
        }
        Target::Jpg => {
            let alpha = has_alpha(video.and_then(|video| video.pix_fmt.as_deref()));
            match video.filter(|_| alpha) {
                // Прозрачное — на белый фон. Без подложки JPEG выбросил бы
                // прозрачность, и пустые места стали бы чёрными
                Some(video) => {
                    let (width, height) = fit_long_side(video.width, video.height, options.image_size);
                    push(&["-f", "lavfi", "-i", &format!("color=c=white:s={width}x{height}")]);
                    push(&[
                        "-frames:v", "1",
                        "-filter_complex",
                        &format!(
                            "[0:v]scale={width}:{height}:flags=lanczos,format=rgba[fg];[1:v][fg]overlay=format=auto:shortest=1,format=yuvj420p[o]"
                        ),
                        "-map", "[o]",
                    ]);
                }
                None => {
                    push(&["-frames:v", "1"]);
                    let filter = match options.image_size {
                        0 => "format=yuvj420p".to_owned(),
                        size => format!("{},format=yuvj420p", long_side(size, "-2")),
                    };
                    push(&["-vf", &filter]);
                }
            }
            push(&["-c:v", "mjpeg", "-q:v", "2", "-f", "image2", "-update", "1"]);
        }
    }

    args.push(output.as_os_str().to_owned());
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::converter::probe::{Audio, Video};

    fn video_probe(width: u32, height: u32, pix_fmt: &str, fps: Option<f64>) -> Probe {
        Probe {
            format: "mov,mp4,m4a,3gp,3g2,mj2".to_owned(),
            duration: Some(10.0),
            video: Some(Video {
                codec: "h264".to_owned(),
                pix_fmt: Some(pix_fmt.to_owned()),
                width,
                height,
                fps,
            }),
            audio: Some(Audio { sample_rate: Some(44_100), layout: Some("stereo".to_owned()) }),
        }
    }

    fn strings(args: &[OsString]) -> Vec<String> {
        args.iter().map(|arg| arg.to_string_lossy().into_owned()).collect()
    }

    fn position(args: &[String], value: &str) -> Option<usize> {
        args.iter().position(|arg| arg == value)
    }

    fn build(target: Target, options: &Options, probe: &Probe, trim: Option<Trim>) -> Vec<String> {
        let (input, output) = (Path::new("C:\\in put.mov"), Path::new("C:\\out.part"));
        strings(&ffmpeg_args(target, options, probe, trim, Encoder::X264, input, output))
    }

    #[test]
    fn every_target_reports_progress_and_ends_with_its_muxer() {
        let video = video_probe(1920, 1080, "yuv420p", Some(30.0));
        let image = Probe { format: "png_pipe".to_owned(), duration: None, audio: None, ..video.clone() };
        for (target, probe, muxer) in [
            (Target::Mp4, &video, "mp4"),
            (Target::Gif, &video, "gif"),
            (Target::Mp3, &video, "mp3"),
            (Target::Png, &image, "image2"),
            (Target::Jpg, &image, "image2"),
        ] {
            let args = build(target, &Options::default(), probe, None);
            for flag in ["-nostdin", "-nostats", "pipe:1"] {
                assert!(args.contains(&flag.to_owned()), "{target:?}: нет {flag}");
            }
            let muxer_at = position(&args, "-f").map(|at| at + 1);
            assert!(args.iter().rev().skip(1).take(10).any(|arg| arg == muxer), "{target:?}: {args:?}");
            assert!(muxer_at.is_some());
            assert_eq!(args.last().map(String::as_str), Some("C:\\out.part"));
            // Имя с пробелом — одно слово, а не два
            assert!(args.contains(&"C:\\in put.mov".to_owned()));
        }
    }

    /// Начало отрезка — до входа, длина — после.
    #[test]
    fn a_trim_seeks_before_the_input_and_limits_after() {
        let probe = video_probe(1920, 1080, "yuv420p", Some(30.0));
        let trim = Trim { from: 5.0, length: Some(10.0) };
        let args = build(Target::Mp4, &Options::default(), &probe, Some(trim));
        let seek = position(&args, "-ss").unwrap();
        let input = position(&args, "-i").unwrap();
        let limit = position(&args, "-t").unwrap();
        assert!(seek < input && input < limit);
        assert_eq!(args[seek + 1], "5.000");
        assert_eq!(args[limit + 1], "10.000");

        let plain = build(Target::Mp4, &Options::default(), &probe, None);
        assert!(position(&plain, "-ss").is_none() && position(&plain, "-t").is_none());
    }

    #[test]
    fn a_trim_from_the_very_start_has_no_seek() {
        let probe = video_probe(1920, 1080, "yuv420p", None);
        let args = build(Target::Mp3, &Options::default(), &probe, Some(Trim { from: 0.0, length: Some(3.0) }));
        assert!(position(&args, "-ss").is_none());
        assert!(position(&args, "-t").is_some());
    }

    #[test]
    fn quality_and_size_reach_the_encoder() {
        let probe = video_probe(1920, 1080, "yuv420p", Some(30.0));
        let options = Options { quality: Quality::Small, video_size: 720, ..Options::default() };
        let args = build(Target::Mp4, &options, &probe, None);
        assert_eq!(args[position(&args, "-crf").unwrap() + 1], "28");
        assert_eq!(args[position(&args, "-b:a").unwrap() + 1], "128k");
        let filter = &args[position(&args, "-vf").unwrap() + 1];
        assert!(filter.contains("min(ih,720)") && filter.ends_with("format=yuv420p"), "{filter}");
    }

    /// Видеокарта и процессор получают каждый свою шкалу качества, а всё
    /// остальное в строке у них общее.
    #[test]
    fn each_encoder_gets_its_own_quality_scale() {
        let probe = video_probe(1920, 1080, "yuv420p", Some(30.0));
        let (input, output) = (Path::new("C:\\in.mov"), Path::new("C:\\out.part"));
        let nvenc = strings(&ffmpeg_args(Target::Mp4, &Options::default(), &probe, None, Encoder::Nvenc, input, output));
        assert_eq!(nvenc[position(&nvenc, "-c:v").unwrap() + 1], "h264_nvenc");
        assert_eq!(nvenc[position(&nvenc, "-cq").unwrap() + 1], "27");
        assert!(position(&nvenc, "-crf").is_none());
        assert_eq!(nvenc[position(&nvenc, "-b:a").unwrap() + 1], "160k");

        let x264 = build(Target::Mp4, &Options::default(), &probe, None);
        assert_eq!(x264[position(&x264, "-preset").unwrap() + 1], "veryfast");
        assert_eq!(x264[position(&x264, "-crf").unwrap() + 1], "23");

        // Проба кодировщика — теми же ключами, что настоящее кодирование
        let tried = nvenc_probe();
        for key in ["h264_nvenc", "p5", "-cq", "null"] {
            assert!(tried.contains(&key), "{key}");
        }
        assert_eq!(tried.last(), Some(&"-"));
    }

    /// Потолок, которого исходник не достаёт, не предлагается: увеличения
    /// нет, и выбор ничего бы не поменял.
    #[test]
    fn only_shrinking_sizes_are_offered() {
        assert_eq!(offered_sizes(&VIDEO_SIZES, Some(720)), vec![0, 480]);
        assert_eq!(offered_sizes(&VIDEO_SIZES, Some(1080)), vec![0, 720, 480]);
        assert_eq!(offered_sizes(&VIDEO_SIZES, Some(2160)), VIDEO_SIZES.to_vec());
        assert_eq!(offered_sizes(&VIDEO_SIZES, None), VIDEO_SIZES.to_vec());
        assert_eq!(offered_sizes(&IMAGE_SIZES, Some(1600)), vec![0, 1280, 640]);

        assert_eq!(gif_sizes(Some(1920)), GIF_SIZES.to_vec(), "большому ролику «как есть» не нужно");
        assert_eq!(gif_sizes(Some(400)), vec![320, 0]);
        assert_eq!(gif_sizes(Some(200)), vec![0]);
    }

    #[test]
    fn a_size_out_of_the_row_becomes_as_is_or_the_default() {
        assert_eq!(chosen_size(720, &[0, 480], 0), 0, "720p у ролика 720p — это «как есть»");
        assert_eq!(chosen_size(480, &[0, 480], 0), 480);
        assert_eq!(chosen_size(480, &[320, 0], 480), 0);
        // «Как есть» от маленького ролика большому не достаётся
        assert_eq!(chosen_size(0, &GIF_SIZES, 480), 480);
    }

    #[test]
    fn a_gif_as_is_has_no_scale() {
        let probe = video_probe(400, 300, "yuv420p", Some(10.0));
        let options = Options { gif_size: 0, ..Options::default() }.sane();
        assert_eq!(options.gif_size, 0);
        let args = build(Target::Gif, &options, &probe, None);
        let graph = &args[position(&args, "-filter_complex").unwrap() + 1];
        assert!(graph.starts_with("[0:v:0]fps=10.00,split"), "{graph}");
    }

    #[test]
    fn mp3_downmixes_and_resamples_only_when_it_must() {
        let mut probe = video_probe(1920, 1080, "yuv420p", None);
        let plain = build(Target::Mp3, &Options::default(), &probe, None);
        assert!(position(&plain, "-ar").is_none() && position(&plain, "-ac").is_none());
        assert_eq!(plain[position(&plain, "-b:a").unwrap() + 1], "192k");

        probe.audio = Some(Audio { sample_rate: Some(96_000), layout: Some("5.1".to_owned()) });
        let surround = build(Target::Mp3, &Options::default(), &probe, None);
        assert_eq!(surround[position(&surround, "-ar").unwrap() + 1], "48000");
        assert_eq!(surround[position(&surround, "-ac").unwrap() + 1], "2");
    }

    #[test]
    fn gif_never_invents_frames() {
        assert_eq!(gif_fps(15, Some(30.0)), "15");
        assert_eq!(gif_fps(24, Some(10.0)), "10.00");
        assert_eq!(gif_fps(15, None), "15");
    }

    /// Прозрачная картинка в JPEG ложится на белое полотно её же размера.
    #[test]
    fn a_transparent_jpg_goes_onto_white() {
        let mut probe = video_probe(300, 200, "rgba", None);
        probe.format = "png_pipe".to_owned();
        let args = build(Target::Jpg, &Options::default(), &probe, None);
        assert!(args.contains(&"color=c=white:s=300x200".to_owned()), "{args:?}");
        assert!(args.iter().any(|arg| arg.contains("overlay")));

        probe.video.as_mut().unwrap().pix_fmt = Some("yuvj420p".to_owned());
        let opaque = build(Target::Jpg, &Options::default(), &probe, None);
        assert!(!opaque.iter().any(|arg| arg.contains("overlay")));
    }

    #[test]
    fn images_shrink_but_never_grow() {
        assert_eq!(fit_long_side(4000, 3000, 1920), (1920, 1440));
        assert_eq!(fit_long_side(3000, 4000, 1920), (1440, 1920));
        assert_eq!(fit_long_side(800, 600, 1920), (800, 600));
        assert_eq!(fit_long_side(301, 201, 0), (300, 200));
    }

    #[test]
    fn images_never_get_a_trim() {
        let mut probe = video_probe(800, 600, "yuv420p", None);
        probe.format = "image2".to_owned();
        let args = build(Target::Png, &Options::default(), &probe, Some(Trim { from: 5.0, length: Some(1.0) }));
        assert!(position(&args, "-ss").is_none() && position(&args, "-t").is_none());
    }

    #[test]
    fn timecodes_parse_the_way_people_type_them() {
        assert_eq!(parse_timecode(""), Ok(None));
        assert_eq!(parse_timecode("5"), Ok(Some(5.0)));
        assert_eq!(parse_timecode("90"), Ok(Some(90.0)));
        assert_eq!(parse_timecode("1:05"), Ok(Some(65.0)));
        assert_eq!(parse_timecode("01:05"), Ok(Some(65.0)));
        assert_eq!(parse_timecode("1:02:03"), Ok(Some(3723.0)));
        assert_eq!(parse_timecode("1:02:03.5"), Ok(Some(3723.5)));
        assert_eq!(parse_timecode("1,5"), Ok(Some(1.5)));

        for bad in ["1:60", "-1", "1:2:3:4", "abc", "1::2", "1:60:00"] {
            assert_eq!(parse_timecode(bad), Err(()), "{bad}");
        }
    }

    #[test]
    fn a_trim_is_checked_against_the_file() {
        assert_eq!(resolve_trim("", "", Some(60.0)), Ok(None));
        assert_eq!(
            resolve_trim("0:10", "0:20", Some(60.0)),
            Ok(Some(Trim { from: 10.0, length: Some(10.0) }))
        );
        assert_eq!(
            resolve_trim("0:10", "5:00", Some(60.0)),
            Ok(Some(Trim { from: 10.0, length: Some(50.0) }))
        );
        assert_eq!(resolve_trim("0:10", "", Some(60.0)), Ok(Some(Trim { from: 10.0, length: None })));
        assert_eq!(resolve_trim("0:20", "0:10", Some(60.0)), Err(TrimError::EmptyRange));
        assert_eq!(resolve_trim("2:00", "", Some(60.0)), Err(TrimError::BadFrom));
        assert_eq!(resolve_trim("x", "", Some(60.0)), Err(TrimError::BadFrom));
        assert_eq!(resolve_trim("", "y", Some(60.0)), Err(TrimError::BadTo));
    }

    #[test]
    fn progress_is_read_and_clamped() {
        assert_eq!(progress_seconds("out_time_us=1500000"), Some(Some(1.5)));
        assert_eq!(progress_seconds("out_time_us=N/A"), Some(None));
        assert_eq!(progress_seconds("frame=10"), None);
        assert_eq!(fraction(5.0, 10.0), 0.5);
        assert_eq!(fraction(12.0, 10.0), 1.0);
        assert_eq!(segment(Some(60.0), Some(Trim { from: 10.0, length: None })), Some(50.0));
        assert_eq!(segment(None, None), None);
    }

    #[test]
    fn a_batch_takes_only_shared_targets() {
        let video = targets(Kind::Video, true);
        let silent = targets(Kind::Video, false);
        let audio = targets(Kind::Audio, true);
        let image = targets(Kind::Image, false);
        assert_eq!(batch_targets(&[video.clone(), audio]), vec![Target::Mp3]);
        assert_eq!(batch_targets(&[video.clone(), silent]), vec![Target::Mp4, Target::Gif]);
        assert!(batch_targets(&[video, image.clone()]).is_empty());
        assert_eq!(batch_targets(&[image]), vec![Target::Png, Target::Jpg]);
        assert!(batch_targets(&[]).is_empty());
    }

    #[test]
    fn stray_numbers_fall_back_to_the_card_lists() {
        let options = Options { bitrate: 999, gif_fps: 60, image_size: 123, ..Options::default() }.sane();
        assert_eq!((options.bitrate, options.gif_fps, options.image_size), (192, 15, 0));
    }
}
