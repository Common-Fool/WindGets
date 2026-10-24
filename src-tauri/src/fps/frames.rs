//! Разбор вывода PresentMon: строка CSV на каждый кадр каждого процесса.
//!
//! ВАЖНО: **столбцы ищутся по заголовку, а не по номеру.** Набор столбцов
//! зависит от версии и ключей (`--v1_metrics`, слежение за выводом на экран,
//! за видеокартой), и номер «пятый столбец» поплыл бы молча. У каждого
//! показателя — список имён с запасными: у метрик 1.x и 2.x они разные.

/// Где в строке нужные показатели.
#[derive(Debug, Clone, PartialEq)]
pub struct Columns {
    app: usize,
    pid: usize,
    frame: usize,
    displayed: Option<usize>,
    gpu_busy: Option<usize>,
    latency: Option<usize>,
    mode: Option<usize>,
}

/// Кадр одного процесса.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// Имя exe как его даёт PresentMon: `Cyberpunk2077.exe`.
    pub app: String,
    pub pid: u32,
    /// Время кадра — от прошлого кадра до этого, мс.
    pub ms: f32,
    /// Дошёл ли кадр до экрана. Пропущенный — `false`.
    pub displayed: bool,
    /// Сколько из кадра видеокарта была занята, мс.
    pub gpu_busy: Option<f32>,
    /// Задержка от начала кадра до экрана, мс.
    pub latency: Option<f32>,
    /// Способ вывода: `Hardware: Independent Flip`, `Composed: Flip`…
    pub mode: Option<String>,
}

fn find(names: &[&str], wanted: &[&str]) -> Option<usize> {
    wanted
        .iter()
        .find_map(|want| names.iter().position(|name| name.eq_ignore_ascii_case(want)))
}

impl Columns {
    /// Заголовок CSV. Без имени, PID или времени кадра строка бесполезна —
    /// тогда `None`, и это видно в журнале, а не нулями на карточке.
    pub fn from_header(line: &str) -> Option<Columns> {
        let names: Vec<&str> = line.split(',').map(str::trim).collect();
        Some(Columns {
            app: find(&names, &["Application"])?,
            pid: find(&names, &["ProcessID"])?,
            frame: find(&names, &["FrameTime", "MsBetweenPresents", "msBetweenPresents"])?,
            displayed: find(&names, &["DisplayedTime", "MsBetweenDisplayChange"]),
            gpu_busy: find(&names, &["GPUBusy", "MsGPUBusy", "msGPUActive"]),
            latency: find(&names, &["DisplayLatency", "MsUntilDisplayed"]),
            mode: find(&names, &["PresentMode"]),
        })
    }

    /// Строка кадра. Испорченная или чужая (предупреждение посреди вывода) —
    /// `None`.
    pub fn parse(&self, line: &str) -> Option<Frame> {
        let cells: Vec<&str> = line.split(',').collect();
        let number = |index: Option<usize>| -> Option<f32> {
            let text = cells.get(index?)?.trim();
            text.parse::<f32>().ok().filter(|value| value.is_finite() && *value >= 0.0)
        };

        let ms = number(Some(self.frame)).filter(|ms| *ms > 0.0)?;
        // «NA» или ноль в столбце показа — кадр не дошёл до экрана. Столбца
        // нет вовсе (слежение за экраном выключено) — считаем показанным
        let displayed = match self.displayed {
            None => true,
            Some(_) => number(self.displayed).is_some_and(|value| value > 0.0),
        };

        Some(Frame {
            app: cells.get(self.app)?.trim().to_owned(),
            pid: cells.get(self.pid)?.trim().parse().ok()?,
            ms,
            displayed,
            gpu_busy: number(self.gpu_busy),
            latency: if displayed { number(self.latency) } else { None },
            mode: self
                .mode
                .and_then(|index| cells.get(index))
                .map(|text| text.trim().to_owned())
                .filter(|text| !text.is_empty() && text != "Unknown"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V2: &str = "Application,ProcessID,SwapChainAddress,PresentRuntime,SyncInterval,PresentFlags,AllowsTearing,PresentMode,CPUStartTime,FrameTime,CPUBusy,CPUWait,GPULatency,GPUTime,GPUBusy,GPUWait,DisplayLatency,DisplayedTime,AnimationError,AnimationTime,MsFlipDelay";

    #[test]
    fn a_v2_row_is_read_by_names() {
        let columns = Columns::from_header(V2).unwrap();
        let frame = columns
            .parse("game.exe,4242,0x1,DXGI,0,0,1,Hardware: Independent Flip,1000.5,6.94,5.1,1.8,0.4,6.2,6.0,0.9,12.5,6.94,0.1,1000.0,0")
            .unwrap();
        assert_eq!(frame.app, "game.exe");
        assert_eq!(frame.pid, 4242);
        assert!((frame.ms - 6.94).abs() < 1e-4);
        assert!(frame.displayed);
        assert_eq!(frame.gpu_busy, Some(6.0));
        assert_eq!(frame.latency, Some(12.5));
        assert_eq!(frame.mode.as_deref(), Some("Hardware: Independent Flip"));
    }

    #[test]
    fn a_dropped_frame_is_not_displayed() {
        let columns = Columns::from_header(V2).unwrap();
        let frame = columns
            .parse("game.exe,4242,0x1,DXGI,0,0,1,Composed: Flip,1000.5,6.94,5.1,1.8,0.4,6.2,6.0,0.9,NA,NA,NA,NA,NA")
            .unwrap();
        assert!(!frame.displayed);
        assert_eq!(frame.latency, None);
    }

    #[test]
    fn v1_names_still_work() {
        let header = "Application,ProcessID,SwapChainAddress,Runtime,SyncInterval,PresentFlags,Dropped,TimeInSeconds,msInPresentAPI,msBetweenPresents,AllowsTearing,PresentMode,msUntilRenderComplete,msUntilDisplayed,msBetweenDisplayChange";
        let columns = Columns::from_header(header).unwrap();
        let frame = columns.parse("old.exe,7,0x1,D3D9,1,0,0,1.0,0.1,16.6,0,Composed: Copy with GPU GDI,3.0,20.0,16.6").unwrap();
        assert!((frame.ms - 16.6).abs() < 1e-4);
        assert_eq!(frame.latency, Some(20.0));
    }

    #[test]
    fn junk_is_not_a_frame() {
        let columns = Columns::from_header(V2).unwrap();
        assert_eq!(columns.parse("warning: lost events"), None);
        assert_eq!(columns.parse(""), None);
        assert!(Columns::from_header("Something,Else").is_none());
    }
}
