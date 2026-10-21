//! Превью обрезки: кадр под ручкой у видео, волна и прослушивание у звука.
//!
//! Всё достаёт тот же ffmpeg, что и конвертирует, — своих декодеров нет.
//! Замер на этой машине: кадр из 1080p — 0,34 с, из 4K AV1 — 0,94 с; волна
//! трёхминутного MP3 — 0,12 с; тридцать секунд звука в WAV —
//! 0,06 с. Поэтому кадр просят на отпускании ручки, а не на каждом её сдвиге.
//!
//! ВАЖНО: **звук играет Rust, а не webview.** Тег `<audio>` потребовал бы
//! открыть CSP (`media-src blob:`) и возить мегабайты через IPC; `PlaySound`
//! из winmm играет WAV прямо из памяти, без окна и без COM. Плата — позиции
//! воспроизведения у него не спросить, и бегунок карточка ведёт по часам.

use std::ffi::OsString;
use std::path::Path;

/// Столбиков у волны. Полоса при окне в 440 точек — около 330: по 2,8 точки
/// на столбик, из них точка — просвет. Двести столбиков выходили меньше
/// точки шириной и сливались в серую ленту.
pub const WAVE_BARS: usize = 120;

/// Частота, до которой звук прореживается перед подсчётом волны. Пику в
/// корзине из сотен отсчётов точность выше не нужна, а меньше строк в трубе.
const WAVE_RATE: u32 = 8000;

/// Самое долгое прослушивание: тридцать секунд хватает, чтобы услышать место
/// обрезки, а WAV на них — пять мегабайт в памяти.
pub const LISTEN_MAX: f64 = 30.0;

/// Сколько звучит конец отрезка, когда отпустили правую ручку: слушают то,
/// что перед обрезкой, а не то, что после неё.
pub const LISTEN_TAIL: f64 = 3.0;

/// Рамка кадра в логических точках: 384×216 — втрое больше первой редакции,
/// которую живая проверка назвала слишком маленькой. Кадр вписывается в неё,
/// сохраняя свои пропорции.
pub const FRAME_BOX: (f64, f64) = (384.0, 216.0);

fn base() -> Vec<OsString> {
    ["-hide_banner", "-nostdin", "-loglevel", "error"].map(OsString::from).to_vec()
}

fn push(args: &mut Vec<OsString>, values: &[&str]) {
    args.extend(values.iter().map(OsString::from));
}

/// Секунды для ffmpeg — с тремя знаками, как у конвертации.
fn seconds(value: f64) -> String {
    format!("{value:.3}")
}

/// Один кадр в BMP, вписанный в `width`×`height` физических точек. Поиск до
/// `-i` — быстрый по ключевым кадрам и точный, потому что дальше ffmpeg
/// декодирует до нужного места сам. BMP, а не JPEG: его рисует `StretchDIBits`
/// как есть, без декодера (`popup.rs`).
pub fn frame_args(input: &Path, at: f64, width: u32, height: u32, output: &Path) -> Vec<OsString> {
    let mut args = base();
    push(&mut args, &["-y", "-ss", &seconds(at), "-i"]);
    args.push(input.as_os_str().to_owned());
    push(&mut args, &[
        "-frames:v", "1",
        "-vf", &format!("scale=w={width}:h={height}:force_original_aspect_ratio=decrease"),
        "-pix_fmt", "bgr24", "-c:v", "bmp", "-f", "image2", "-update", "1",
    ]);
    args.push(output.as_os_str().to_owned());
    args
}

/// Волна: пик каждой из `WAVE_BARS` корзин, строками в stdout. Считает сам
/// ffmpeg — `astats` на корзину и `ametadata` в трубу, — так что двоичного
/// звука через трубу не идёт вовсе.
pub fn wave_args(input: &Path, duration: f64) -> Vec<OsString> {
    let samples = (duration.max(0.0) * f64::from(WAVE_RATE)).ceil() as usize;
    let bucket = samples.div_ceil(WAVE_BARS).max(1);
    let graph = format!(
        "aresample={WAVE_RATE},aformat=channel_layouts=mono,asetnsamples=n={bucket}:p=0,\
         astats=metadata=1:reset=1:measure_perchannel=none:measure_overall=Peak_level,\
         ametadata=mode=print:key=lavfi.astats.Overall.Peak_level:file=-"
    );
    let mut args = base();
    push(&mut args, &["-i"]);
    args.push(input.as_os_str().to_owned());
    push(&mut args, &["-vn", "-af", &graph, "-f", "null", "-"]);
    args
}

/// Столбики волны из строк `lavfi.astats.Overall.Peak_level=-12.3`, от нуля
/// до единицы.
///
/// Пики в децибелах переводятся в амплитуду и делятся на самый громкий:
/// тихая запись иначе легла бы ровной чертой по дну полосы. Тишина (`-inf`) —
/// ноль.
pub fn parse_wave<'a>(lines: impl IntoIterator<Item = &'a str>) -> Vec<f32> {
    let peaks: Vec<f64> = lines
        .into_iter()
        .filter_map(|line| line.trim().strip_prefix("lavfi.astats.Overall.Peak_level="))
        .map(|value| match value.parse::<f64>() {
            Ok(db) if db.is_finite() => 10f64.powf(db / 20.0),
            _ => 0.0,
        })
        .collect();

    let loudest = peaks.iter().copied().fold(0.0, f64::max);
    if loudest <= 0.0 {
        return vec![0.0; peaks.len()];
    }
    peaks.iter().map(|peak| (peak / loudest).clamp(0.0, 1.0) as f32).collect()
}

/// Кусок для прослушивания: WAV в 16 бит, стерео, 44,1 кГц — то, что
/// `PlaySound` играет везде без кодеков.
pub fn clip_args(input: &Path, from: f64, length: f64, output: &Path) -> Vec<OsString> {
    let mut args = base();
    push(&mut args, &["-y"]);
    if from > 0.0 {
        push(&mut args, &["-ss", &seconds(from)]);
    }
    push(&mut args, &["-i"]);
    args.push(input.as_os_str().to_owned());
    push(&mut args, &[
        "-t", &seconds(length),
        "-vn", "-ac", "2", "-ar", "44100", "-c:a", "pcm_s16le", "-f", "wav",
    ]);
    args.push(output.as_os_str().to_owned());
    args
}

/// Что слушать: от левой ручки вперёд, но не дальше правой и не дольше
/// `LISTEN_MAX`; от правой — последние секунды перед ней.
pub fn listen_span(from: f64, until: f64, duration: f64, tail: bool) -> Option<(f64, f64)> {
    if !(from.is_finite() && until.is_finite() && duration.is_finite()) || duration <= 0.0 {
        return None;
    }
    let from = from.clamp(0.0, duration);
    let until = until.clamp(0.0, duration);
    let (start, end) = if tail {
        ((until - LISTEN_TAIL).max(from), until)
    } else {
        (from, until.min(from + LISTEN_MAX))
    };
    (end - start > 0.05).then_some((start, end - start))
}

/// Воспроизведение WAV из памяти через `PlaySound`.
///
/// ВАЖНО: **буфер обязан жить, пока звук играет.** `SND_MEMORY` с
/// `SND_ASYNC` читает прямо из переданной памяти, поэтому буфер лежит в
/// статике, а заменяется и освобождается только после остановки прежнего
/// звука. `PlaySound` с пустым именем останавливает звук сразу, а не по
/// окончании.
#[cfg(windows)]
pub mod sound {
    use std::sync::Mutex;

    use windows::core::PCWSTR;
    use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_FLAGS, SND_MEMORY, SND_NODEFAULT};

    static PLAYING: Mutex<Option<Vec<u8>>> = Mutex::new(None);

    pub fn play(wav: Vec<u8>) -> Result<(), String> {
        // Пустой Vec даёт ненулевой указатель без данных: PlaySound читать нечего.
        if wav.is_empty() {
            return Err("пустой буфер звука".to_owned());
        }
        let mut slot = PLAYING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        // Сперва остановить прежний звук — только потом отпускать его буфер
        unsafe {
            let _ = PlaySoundW(PCWSTR::null(), None, SND_FLAGS(0));
        }
        let buffer = slot.insert(wav);
        let started = unsafe {
            PlaySoundW(PCWSTR(buffer.as_ptr().cast()), None, SND_MEMORY | SND_ASYNC | SND_NODEFAULT)
        };
        if !started.as_bool() {
            *slot = None;
            return Err("PlaySound отказал".to_owned());
        }
        Ok(())
    }

    pub fn stop() {
        let mut slot = PLAYING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        unsafe {
            let _ = PlaySoundW(PCWSTR::null(), None, SND_FLAGS(0));
        }
        *slot = None;
    }
}

#[cfg(not(windows))]
pub mod sound {
    pub fn play(_wav: Vec<u8>) -> Result<(), String> {
        Err("нет звука".to_owned())
    }

    pub fn stop() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_nonpositive_duration_cannot_be_listened_to() {
        for duration in [-1.0, 0.0, f64::NEG_INFINITY, f64::INFINITY, f64::NAN] {
            for tail in [false, true] {
                assert_eq!(listen_span(0.0, 1.0, duration, tail), None);
            }
        }
    }

    #[test]
    fn empty_sound_is_refused() {
        assert!(sound::play(Vec::new()).is_err());
    }


    fn strings(args: &[OsString]) -> Vec<String> {
        args.iter().map(|arg| arg.to_string_lossy().into_owned()).collect()
    }

    /// Строки сняты с настоящего ffmpeg 8.1: номер кадра и пик через одну.
    #[test]
    fn the_wave_is_read_and_scaled_to_the_loudest() {
        let lines = [
            "frame:0    pts:0       pts_time:0",
            "lavfi.astats.Overall.Peak_level=-20.000000",
            "frame:1    pts:7200    pts_time:0.9",
            "lavfi.astats.Overall.Peak_level=-6.020600",
            "frame:2    pts:14400   pts_time:1.8",
            "lavfi.astats.Overall.Peak_level=-inf",
        ];
        let wave = parse_wave(lines);
        assert_eq!(wave.len(), 3);
        assert!((wave[1] - 1.0).abs() < 1e-6, "самый громкий — во всю высоту");
        assert!((wave[0] - 0.2).abs() < 0.01, "-20 дБ против -6 — пятая часть: {}", wave[0]);
        assert_eq!(wave[2], 0.0, "тишина — ноль");
        assert!(parse_wave(["lavfi.astats.Overall.Peak_level=-inf"]).iter().all(|bar| *bar == 0.0));
    }

    #[test]
    fn the_wave_buckets_cover_the_whole_file() {
        let args = strings(&wave_args(Path::new("C:\\a b.mp3"), 180.0));
        let graph = &args[args.iter().position(|arg| arg == "-af").unwrap() + 1];
        // 180 с × 8000 / 120 = 12000 отсчётов на столбик
        assert!(graph.contains("asetnsamples=n=12000:p=0"), "{graph}");
        assert!(graph.ends_with("file=-"));
        assert!(args.contains(&"C:\\a b.mp3".to_owned()), "путь одним словом");
        // Нулевая длина не делит на ноль
        let empty = strings(&wave_args(Path::new("x"), 0.0));
        assert!(empty.iter().any(|arg| arg.contains("asetnsamples=n=1:")));
    }

    #[test]
    fn a_frame_is_sought_before_the_input() {
        let args = strings(&frame_args(Path::new("in.mp4"), 12.5, 384, 216, Path::new("out.bmp")));
        let seek = args.iter().position(|arg| arg == "-ss").unwrap();
        let input = args.iter().position(|arg| arg == "-i").unwrap();
        assert!(seek < input);
        assert_eq!(args[seek + 1], "12.500");
        assert_eq!(args.last().map(String::as_str), Some("out.bmp"));
        assert!(args.iter().any(|arg| arg.contains("scale=w=384:h=216:force_original_aspect_ratio=decrease")));
    }

    #[test]
    fn listening_starts_at_the_left_handle_and_ends_before_the_right() {
        // Левая: вперёд, но не дальше правой и не дольше тридцати секунд
        assert_eq!(listen_span(10.0, 20.0, 60.0, false), Some((10.0, 10.0)));
        assert_eq!(listen_span(10.0, 200.0, 300.0, false), Some((10.0, LISTEN_MAX)));
        // Правая: три секунды перед ней, но не раньше левой
        assert_eq!(listen_span(10.0, 20.0, 60.0, true), Some((17.0, 3.0)));
        assert_eq!(listen_span(19.0, 20.0, 60.0, true), Some((19.0, 1.0)));
        // Мимо файла и пустой отрезок — ничего
        assert_eq!(listen_span(80.0, 90.0, 60.0, false), None);
        assert_eq!(listen_span(f64::NAN, 1.0, 60.0, false), None);
    }

    /// Живьём на ffmpeg из PATH: кадр выходит JPEG-ом, волна — ровно
    /// `WAVE_BARS` столбиков с громким местом во всю высоту, кусок — WAV.
    #[test]
    #[ignore = "нужен ffmpeg в PATH: cargo test -- --ignored previews_come"]
    fn previews_come_out_of_real_ffmpeg() {
        use crate::child::{self, Line};
        use std::process::Command;
        use std::time::Duration;

        let ffmpeg = crate::tools::system_ffmpeg().expect("ffmpeg в PATH");
        let dir = std::env::temp_dir().join(format!("hw-preview-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: Vec<OsString>| {
            let mut command = Command::new(&ffmpeg);
            command.args(args);
            child::run(command, Duration::from_secs(60)).unwrap()
        };

        let video = dir.join("clip.mp4");
        let mut make = vec![OsString::from("-hide_banner")];
        for arg in ["-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=s=640x360:r=25:d=4", "-pix_fmt", "yuv420p"] {
            make.push(arg.into());
        }
        make.push(video.clone().into());
        assert_eq!(run(make).0, Some(0));

        let song = dir.join("song.mp3");
        let mut make = vec![OsString::from("-hide_banner")];
        for arg in ["-loglevel", "error", "-y", "-f", "lavfi", "-i", "sine=f=440:d=20,volume='0.1+0.9*gte(t,10)':eval=frame"] {
            make.push(arg.into());
        }
        make.push(song.clone().into());
        assert_eq!(run(make).0, Some(0));

        let frame = dir.join("frame.bmp");
        assert_eq!(run(frame_args(&video, 2.0, 384, 216, &frame)).0, Some(0));
        let bytes = std::fs::read(&frame).unwrap();
        // 640×360 вписывается в 384×216 ровно
        #[cfg(windows)]
        assert_eq!(super::super::popup::bmp_parts(&bytes).map(|(w, h, _)| (w, h)), Some((384, 216)));

        let (code, lines) = run(wave_args(&song, 20.0));
        assert_eq!(code, Some(0));
        let wave = parse_wave(lines.iter().map(Line::text));
        assert!(wave.len().abs_diff(WAVE_BARS) <= 1, "столбиков {}", wave.len());
        let (quiet, loud) = wave.split_at(wave.len() / 2);
        assert!(loud.iter().any(|bar| *bar > 0.95), "громкая половина во всю высоту");
        assert!(quiet[..quiet.len() - 2].iter().all(|bar| *bar < 0.2), "тихая — низко: {quiet:?}");

        let clip = dir.join("clip.wav");
        assert_eq!(run(clip_args(&song, 5.0, 3.0, &clip)).0, Some(0));
        let wav = std::fs::read(&clip).unwrap();
        assert_eq!(&wav[..4], b"RIFF");
        // Три секунды 44,1 кГц стерео по два байта — около полумегабайта
        assert!((520_000..540_000).contains(&wav.len()), "{} байт", wav.len());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_clip_is_plain_wav() {
        let args = strings(&clip_args(Path::new("in.mp3"), 0.0, 30.0, Path::new("c.wav")));
        assert!(!args.contains(&"-ss".to_owned()), "с начала файла поиск не нужен");
        assert!(args.contains(&"pcm_s16le".to_owned()));
        assert_eq!(args[args.iter().position(|arg| arg == "-t").unwrap() + 1], "30.000");
    }
}
