//! Счёт по кадрам одной игры: FPS, средний, 1 % и 0,1 % low, время кадра,
//! занятость видеокарты, задержка, история.
//!
//! Чистая арифметика без процессов и окон — ради тестов.

use std::collections::VecDeque;

use serde::Serialize;

use super::frames::Frame;

/// Сколько миллисекунд кадров держим для low-показателей.
///
/// Тридцать секунд: за одну 1 % low — это пара кадров и пляшет от каждой
/// подгрузки, а за весь сеанс одна давняя загрузка уровня держала бы число
/// внизу часами.
const LOW_WINDOW_MS: f64 = 30_000.0;

/// Длина истории — по столбику на секунду, пять минут, как у железа.
pub const HISTORY: usize = 300;

/// Тактов в секунде: число на карточке обновляется четыре раза в секунду.
pub const TICKS_PER_SECOND: usize = 4;

/// Окно числа на карточке — полсекунды **по времени самих кадров**, а не по
/// тактам прихода. Секундное окно запаздывало на глаз.
///
/// ВАЖНО: **PresentMon отдаёт кадры пачками** — буфер ETW сбрасывается около
/// раза в секунду, и в такт 250 мс кадров часто не приходит вовсе. Окно по
/// тактам давало прочерк через раз (живая проверка на Diablo II).
const SHOWN_MS: f64 = 500.0;

/// Сколько тактов без единого кадра — уже «кадров нет», а не пауза между
/// пачками: полторы секунды.
const SILENT_TICKS: usize = 6;

#[derive(Debug, Clone, Copy)]
struct Sample {
    ms: f32,
    displayed: bool,
    gpu_busy: Option<f32>,
    latency: Option<f32>,
}

/// Числа за последний такт.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    /// Кадров в секунду за последние полсекунды. Кадров не было — `None`.
    pub fps: Option<f32>,
    /// Средний FPS с начала слежения за этой игрой.
    pub average: Option<f32>,
    /// FPS худшего процента кадров за последние 30 с.
    pub low1: Option<f32>,
    /// То же для 0,1 %: нужна тысяча кадров.
    pub low01: Option<f32>,
    /// Среднее время кадра за такт, мс.
    pub frame_ms: Option<f32>,
    /// Доля времени кадра, когда видеокарта была занята, %. Около сотни — упор
    /// в видеокарту, заметно ниже — в процессор или ограничитель кадров.
    pub gpu_busy: Option<f32>,
    /// Средняя задержка до экрана за такт, мс.
    pub latency: Option<f32>,
    /// Доля кадров такта, не дошедших до экрана, %.
    pub dropped: Option<f32>,
}

#[derive(Default)]
pub struct Tracker {
    tick: Vec<Sample>,
    /// Последние полсекунды кадров — окно числа на карточке.
    recent: VecDeque<Sample>,
    recent_ms: f64,
    /// Тактов подряд без кадров.
    silent: usize,
    ticks: usize,
    window: VecDeque<Sample>,
    window_ms: f64,
    total_frames: u64,
    total_ms: f64,
    history: VecDeque<f32>,
}

fn mean(values: impl Iterator<Item = f32>) -> Option<f32> {
    let (sum, count) = values.fold((0.0f64, 0u32), |(sum, count), value| (sum + value as f64, count + 1));
    (count > 0).then(|| (sum / count as f64) as f32)
}

/// Кадров в секунду по их собственному времени.
fn rate(samples: &[Sample]) -> Option<f32> {
    let total: f64 = samples.iter().map(|sample| sample.ms as f64).sum();
    (!samples.is_empty() && total > 0.0).then(|| (1000.0 * samples.len() as f64 / total) as f32)
}

/// FPS худшей доли кадров: среднее время самых долгих кадров, обращённое в
/// кадры в секунду. Меньше кадров, чем нужно на одну штуку доли, — `None`.
fn low(window: &VecDeque<Sample>, share: f64) -> Option<f32> {
    let needed = (1.0 / share).round() as usize;
    if window.len() < needed {
        return None;
    }
    let mut times: Vec<f32> = window.iter().map(|sample| sample.ms).collect();
    times.sort_by(|left, right| right.total_cmp(left));
    let worst = ((times.len() as f64 * share).ceil() as usize).max(1);
    mean(times[..worst].iter().copied()).map(|ms| 1000.0 / ms)
}

impl Tracker {
    pub fn push(&mut self, frame: &Frame) {
        let sample = Sample {
            ms: frame.ms,
            displayed: frame.displayed,
            gpu_busy: frame.gpu_busy,
            latency: frame.latency,
        };
        self.tick.push(sample);
        self.window.push_back(sample);
        self.window_ms += frame.ms as f64;
        while self.window_ms > LOW_WINDOW_MS {
            let Some(old) = self.window.pop_front() else { break };
            self.window_ms -= old.ms as f64;
        }
        self.total_frames += 1;
        self.total_ms += frame.ms as f64;
    }

    /// Закрывает такт: считает числа, раз в секунду кладёт столбик в историю.
    pub fn close_tick(&mut self) -> Summary {
        let tick = std::mem::take(&mut self.tick);
        if tick.is_empty() {
            self.silent += 1;
            if self.silent >= SILENT_TICKS {
                self.recent.clear();
                self.recent_ms = 0.0;
            }
        } else {
            self.silent = 0;
        }
        for sample in tick {
            self.recent.push_back(sample);
            self.recent_ms += sample.ms as f64;
        }
        // Самый свежий кадр остаётся, даже если он один длиннее окна
        while self.recent_ms > SHOWN_MS && self.recent.len() > 1 {
            let Some(old) = self.recent.pop_front() else { break };
            self.recent_ms -= old.ms as f64;
        }
        let shown: Vec<Sample> = self.recent.iter().copied().collect();
        let fps = rate(&shown);

        self.ticks += 1;
        if self.ticks % TICKS_PER_SECOND == 0 {
            if self.history.len() == HISTORY {
                self.history.pop_front();
            }
            // Нет кадров — столбик в ноль: игра встала, и это видно на графике
            // Целая секунда без кадров — ноль, даже если число на карточке
            // ещё держится между пачками
            let quiet = self.silent >= TICKS_PER_SECOND;
            self.history.push_back(if quiet { 0.0 } else { fps.unwrap_or(0.0) });
        }

        let gpu_busy = {
            let busy: Vec<(f32, f32)> = shown.iter().filter_map(|sample| Some((sample.gpu_busy?, sample.ms))).collect();
            let frames: f32 = busy.iter().map(|(_, ms)| ms).sum();
            (frames > 0.0).then(|| (busy.iter().map(|(gpu, _)| gpu).sum::<f32>() / frames * 100.0).clamp(0.0, 100.0))
        };

        Summary {
            fps,
            average: (self.total_ms > 0.0).then(|| (1000.0 * self.total_frames as f64 / self.total_ms) as f32),
            low1: low(&self.window, 0.01),
            low01: low(&self.window, 0.001),
            frame_ms: fps.map(|fps| 1000.0 / fps),
            gpu_busy,
            latency: mean(shown.iter().filter_map(|sample| sample.latency)),
            dropped: (!shown.is_empty())
                .then(|| shown.iter().filter(|sample| !sample.displayed).count() as f32 / shown.len() as f32 * 100.0),
        }
    }

    pub fn history(&self) -> Vec<f32> {
        self.history.iter().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ms: f32) -> Frame {
        Frame {
            app: "game.exe".to_owned(),
            pid: 1,
            chain: "0x1".to_owned(),
            ms,
            displayed: true,
            gpu_busy: Some(ms / 2.0),
            latency: Some(10.0),
            mode: None,
        }
    }

    #[test]
    fn a_steady_game_reads_steady() {
        let mut tracker = Tracker::default();
        for _ in 0..144 {
            tracker.push(&frame(1000.0 / 144.0));
        }
        let summary = tracker.close_tick();
        assert!((summary.fps.unwrap() - 144.0).abs() < 0.1);
        assert!((summary.average.unwrap() - 144.0).abs() < 0.1);
        assert!((summary.low1.unwrap() - 144.0).abs() < 0.1);
        assert_eq!(summary.low01, None, "на 0,1 % нужна тысяча кадров");
        assert!((summary.gpu_busy.unwrap() - 50.0).abs() < 0.1);
        assert_eq!(summary.dropped, Some(0.0));
    }

    #[test]
    fn one_stutter_drags_the_low_down() {
        let mut tracker = Tracker::default();
        for _ in 0..199 {
            tracker.push(&frame(10.0));
        }
        tracker.push(&frame(100.0));
        let summary = tracker.close_tick();
        // Худшие 1 % из двухсот — два кадра: 100 и 10 мс, в среднем 55
        assert!((summary.low1.unwrap() - 1000.0 / 55.0).abs() < 0.1, "{summary:?}");
    }

    #[test]
    fn an_empty_tick_is_a_dash_and_a_zero_column() {
        let mut tracker = Tracker::default();
        let summary = tracker.close_tick();
        assert_eq!(summary.fps, None);
        assert_eq!(summary.frame_ms, None);
        assert!(tracker.history().is_empty(), "столбик — раз в секунду");
        for _ in 1..TICKS_PER_SECOND {
            tracker.close_tick();
        }
        assert_eq!(tracker.history(), vec![0.0]);
    }

    #[test]
    fn the_low_window_forgets_old_frames() {
        let mut tracker = Tracker::default();
        tracker.push(&frame(500.0));
        for _ in 0..4000 {
            tracker.push(&frame(10.0));
        }
        let summary = tracker.close_tick();
        // 40 с кадров по 10 мс: давняя полусекундная загрузка ушла из окна
        assert!((summary.low1.unwrap() - 100.0).abs() < 0.1, "{summary:?}");
    }

    /// Число на карточке — последние полсекунды кадров: давняя просадка из
    /// него уходит.
    #[test]
    fn the_shown_number_follows_the_last_half_second() {
        let mut tracker = Tracker::default();
        tracker.push(&frame(100.0));
        for _ in 0..50 {
            tracker.push(&frame(10.0));
        }
        let summary = tracker.close_tick();
        assert!((summary.fps.unwrap() - 100.0).abs() < 0.1, "{summary:?}");
    }

    /// Пустой такт между пачками PresentMon — не прочерк; прочерк — только
    /// после полутора секунд тишины.
    #[test]
    fn a_gap_between_batches_is_not_a_dash() {
        let mut tracker = Tracker::default();
        for _ in 0..60 {
            tracker.push(&frame(10.0));
        }
        tracker.close_tick();
        let summary = tracker.close_tick();
        assert!(summary.fps.is_some(), "{summary:?}");
        for _ in 0..SILENT_TICKS {
            tracker.close_tick();
        }
        assert_eq!(tracker.close_tick().fps, None);
    }

    /// Секунда без единого кадра — нулевой столбик, хотя число на карточке
    /// ещё держится (замечание Sol).
    #[test]
    fn a_silent_second_is_a_zero_column() {
        let mut tracker = Tracker::default();
        for _ in 0..TICKS_PER_SECOND {
            for _ in 0..25 {
                tracker.push(&frame(10.0));
            }
            tracker.close_tick();
        }
        for _ in 0..TICKS_PER_SECOND {
            tracker.close_tick();
        }
        assert_eq!(tracker.history(), vec![100.0, 0.0]);
    }

    #[test]
    fn history_is_capped() {
        let mut tracker = Tracker::default();
        for _ in 0..(HISTORY + 20) * TICKS_PER_SECOND {
            tracker.push(&frame(10.0));
            tracker.close_tick();
        }
        assert_eq!(tracker.history().len(), HISTORY);
    }
}
