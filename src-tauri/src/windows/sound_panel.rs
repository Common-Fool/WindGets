//! Боковая панель звука в том же окне. Место считается от пульта, а не от
//! раскрытого прямоугольника; координаты экрана здесь физические.

use std::sync::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};
use crate::settings::SettingsState;
use crate::widgets;

const VERTICAL_VIEW: usize = 2;
const ANCHOR_FILE: &str = "sound-window-anchor.json";
static PANEL: Mutex<Option<PanelState>> = Mutex::new(None);

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
struct Point { x: i32, y: i32 }
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
struct Rect { x: i32, y: i32, width: i32, height: i32 }
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Side { Left, #[default] Right }

#[derive(Clone, Copy)]
struct PanelState {
    anchor: Point,
    last: Rect,
    side: Side,
    panel_width: f64,
    panel_height: f64,
    scale: f64,
    factor: f64,
}

impl PanelState {
    fn anchor_at(self, actual: Rect, factor: f64) -> Point {
        if actual.x == self.last.x && actual.y == self.last.y { return self.anchor; }
        // После переноса мышью старый подъём забываем, но ширину панели
        // слева сохраняем: сам пульт находится правее левого края окна.
        Point {
            x: actual.x + if self.side == Side::Left { pixels(self.panel_width, factor) } else { 0 },
            y: actual.y,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub view: usize,
    pub size: usize,
    pub scale: f64,
    pub request_id: u64,
    pub panel_width: f64,
    pub panel_height: f64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reply { request_id: u64, side: Side }

fn valid(request: &Request, view: usize, size: usize, scale: f64) -> bool {
    request.view == VERTICAL_VIEW && request.view == view && request.size == size
        && request.scale.is_finite() && (request.scale - scale).abs() < 0.0001
        && [request.panel_width, request.panel_height].into_iter()
            .all(|value| value.is_finite() && (0.0..=100_000.0).contains(&value))
        && ((request.panel_width == 0.0) == (request.panel_height == 0.0))
}

fn pixels(logical: f64, factor: f64) -> i32 { (logical * factor).round() as i32 }

fn rect(window: &WebviewWindow) -> Result<Rect, String> {
    let place = window.outer_position().map_err(|error| error.to_string())?;
    let size = window.inner_size().map_err(|error| error.to_string())?;
    Ok(Rect { x: place.x, y: place.y, width: size.width as i32, height: size.height as i32 })
}

fn work_area(window: &WebviewWindow, anchor: Point, base: (i32, i32)) -> Result<Rect, String> {
    let monitor = window.monitor_from_point(
        f64::from(anchor.x) + f64::from(base.0) / 2.0,
        f64::from(anchor.y) + f64::from(base.1) / 2.0,
    ).map_err(|error| error.to_string())?
        .or_else(|| window.current_monitor().ok().flatten())
        .ok_or_else(|| "не найдена рабочая область звука".to_owned())?;
    let area = monitor.work_area();
    Ok(Rect { x: area.position.x, y: area.position.y, width: area.size.width as i32, height: area.size.height as i32 })
}

fn fit_axis(wanted: i32, start: i32, available: i32, size: i32) -> i32 {
    wanted.clamp(start, start.saturating_add(available.saturating_sub(size).max(0)))
}

fn layout(anchor: Point, base: (i32, i32), panel: (i32, i32), area: Rect, previous: Side) -> (Rect, Side) {
    let left = anchor.x - area.x;
    let right = area.x + area.width - (anchor.x + base.0);
    let side = if left > right { Side::Left } else if right > left { Side::Right } else { previous };
    let width = base.0 + panel.0;
    // Вертикальная панель прокручивает содержимое внутри основы. Даже если
    // WebView пришлёт запоздавший замер длинного списка, окно не растёт.
    let height = base.1;
    let wanted_x = anchor.x - if side == Side::Left { panel.0 } else { 0 };
    (Rect {
        x: fit_axis(wanted_x, area.x, area.width, width),
        y: fit_axis(anchor.y, area.y, area.height, height),
        width,
        height,
    }, side)
}

fn apply(window: &WebviewWindow, target: Rect) -> Result<(), String> {
    window.set_size(PhysicalSize::new(target.width as u32, target.height as u32))
        .map_err(|error| error.to_string())?;
    window.set_position(PhysicalPosition::new(target.x, target.y))
        .map_err(|error| error.to_string())
}

fn fit_main(window: &WebviewWindow, request: Request) -> Result<Reply, String> {
    if window.label() != "sound" { return Err("боковая панель есть только у звука".to_owned()); }
    let widget = widgets::find("sound").ok_or_else(|| "звук отсутствует в реестре".to_owned())?;
    let settings = window.app_handle().state::<SettingsState>().get();
    let own = settings.widget(widget);
    let scale = settings.scale_of(widget);
    if !valid(&request, own.view, own.size, scale) {
        return Err("замер прежней раскладки звука пропущен".to_owned());
    }
    let actual = rect(window)?;
    let factor = window.scale_factor().map_err(|error| error.to_string())?;
    let previous = *PANEL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let anchor = previous.map(|state| state.anchor_at(actual, factor)).unwrap_or(Point { x: actual.x, y: actual.y });
    let (base_width, base_height) = widget.preset_size(own.view, own.size);
    let base_height = base_height.ok_or_else(|| "у вертикального звука нет высоты основы".to_owned())?;
    let base = (pixels(base_width * scale, factor), pixels(base_height * scale, factor));
    let area = work_area(window, anchor, base)?;
    let panel = (pixels(request.panel_width, factor), pixels(request.panel_height, factor));
    let (target, side) = layout(anchor, base, panel, area, previous.map(|state| state.side).unwrap_or_default());
    // При уменьшении масштаба прежний минимум иначе удержал бы ширину
    // большого пульта, даже если новый пресет уже посчитан правильно.
    window.set_min_size(Some(widget.min_size(scale))).map_err(|error| error.to_string())?;
    apply(window, target)?;
    *PANEL.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = (panel.0 > 0).then_some(PanelState {
        anchor, last: target, side,
        panel_width: request.panel_width, panel_height: request.panel_height, scale, factor,
    });
    Ok(Reply { request_id: request.request_id, side })
}

pub async fn fit(window: WebviewWindow, request: Request) -> Result<Reply, String> {
    let app = window.app_handle().clone();
    let (send, receive) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || { let _ = send.send(fit_main(&window, request)); })
        .map_err(|error| error.to_string())?;
    receive.await.map_err(|_| "окно звука не завершило размещение панели".to_owned())?
}

/// Масштаб меняет обе стороны раскрытого окна, не отрезая боковую панель.
pub fn rescale(window: &WebviewWindow, scale: f64) -> bool {
    let previous = *PANEL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let (width, height) = previous.map(|state| {
        let ratio = scale / state.scale;
        (state.panel_width * ratio, state.panel_height * ratio)
    }).unwrap_or((0.0, 0.0));
    fit_main(window, Request { view: VERTICAL_VIEW, size: 0, scale, request_id: 0,
        panel_width: width, panel_height: height }).is_ok()
}

/// Смена вида сперва возвращает место пульта, затем ставит новый пресет.
pub fn restore_base(window: &WebviewWindow) -> bool {
    if window.label() != "sound" { return false; }
    let previous = PANEL.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
    let Some(previous) = previous else { return false; };
    if let Ok(actual) = rect(window) {
        let anchor = previous.anchor_at(actual, window.scale_factor().unwrap_or(previous.factor));
        let _ = window.set_position(PhysicalPosition::new(anchor.x, anchor.y));
    }
    true
}

pub fn clamp_base(window: &WebviewWindow) {
    let Ok(actual) = rect(window) else { return; };
    let anchor = Point { x: actual.x, y: actual.y };
    let Ok(area) = work_area(window, anchor, (actual.width, actual.height)) else { return; };
    let (target, _) = layout(anchor, (actual.width, actual.height), (0, 0), area, Side::Right);
    let _ = window.set_position(PhysicalPosition::new(target.x, target.y));
}

pub fn forget() { *PANEL.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None; }

#[derive(Deserialize, Serialize)]
struct SavedAnchor { anchor: Point, expanded: Rect }

impl SavedAnchor {
    fn restore_for(&self, actual: Rect) -> Option<Point> {
        (actual == self.expanded).then_some(self.anchor)
    }
}

/// Плагин запоминает раскрытый прямоугольник. Рядом сохраняем его якорь,
/// чтобы после перезапуска левый край панели не стал местом самого пульта.
pub fn save_anchor(app: &AppHandle) {
    let result = (|| -> Result<(), String> {
        // Выключенный модуль окна не имеет. Другие окна продолжают писать
        // свои позиции, но сохранённый якорь звука им стирать нельзя.
        let Some(window) = app.get_webview_window("sound") else { return Ok(()); };
        let path = app.path().app_config_dir().map_err(|error| error.to_string())?.join(ANCHOR_FILE);
        let previous = *PANEL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let saved = if let Some(previous) = previous {
            let actual = rect(&window)?;
            let factor = window.scale_factor().unwrap_or(previous.factor);
            Some(SavedAnchor { anchor: previous.anchor_at(actual, factor), expanded: actual })
        } else { None };
        // Если запись была, стираем содержимое атомарно, чтобы давний якорь
        // никогда не ожил повторно. У нового профиля спутник не создаём.
        if saved.is_none() && !path.exists() { return Ok(()); }
        let text = serde_json::to_string(&saved).map_err(|error| error.to_string())?;
        crate::disk::write_atomically(&path, &text).map_err(|error| error.to_string())
    })();
    if let Err(error) = result { eprintln!("место пульта звука не сохранилось: {error}"); }
}

pub fn restore_anchor(window: &WebviewWindow) {
    if window.label() != "sound" { return; }
    // Это новое окно после старта или повторного включения модуля:
    // прежнее раскрытие в памяти не относится к восстановленной основе.
    forget();
    let saved = (|| -> Option<SavedAnchor> {
        let path = window.app_handle().path().app_config_dir().ok()?.join(ANCHOR_FILE);
        serde_json::from_str::<Option<SavedAnchor>>(&std::fs::read_to_string(path).ok()?).ok()?
    })();
    let (Some(saved), Ok(actual)) = (saved, rect(window)) else { return; };
    // Пара файлов могла прерваться между записями, монитор могли отключить:
    // чужую или более новую позицию старой записью не перезаписываем.
    if let Some(anchor) = saved.restore_for(actual) {
        let _ = window.set_position(PhysicalPosition::new(anchor.x, anchor.y));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const AREA: Rect = Rect { x: 0, y: 0, width: 1920, height: 1040 };
    const BASE: (i32, i32) = (118, 252);

    #[test]
    fn opens_toward_the_greater_work_area_space() {
        let (left, side) = layout(Point { x: 1700, y: 100 }, BASE, (320, 400), AREA, Side::Right);
        assert_eq!(side, Side::Left);
        assert_eq!(left, Rect { x: 1380, y: 100, width: 438, height: 252 });
        let (right, side) = layout(Point { x: 20, y: 100 }, BASE, (320, 400), AREA, Side::Left);
        assert_eq!(side, Side::Right);
        assert_eq!(right.x, 20);
    }

    #[test]
    fn changing_panels_and_closing_never_accumulates_displacement() {
        let anchor = Point { x: 1750, y: 788 };
        let (large, _) = layout(anchor, BASE, (320, 700), AREA, Side::Right);
        assert_eq!((large.x, large.y, large.height), (1430, 788, BASE.1));
        let (small, _) = layout(anchor, BASE, (280, 300), AREA, Side::Left);
        assert_eq!((small.x, small.y, small.height), (1470, 788, BASE.1));
        let (closed, _) = layout(anchor, BASE, (0, 0), AREA, Side::Left);
        assert_eq!((closed.x, closed.y), (anchor.x, anchor.y));
    }

    #[test]
    fn dragging_rebases_to_the_visible_strip_and_forgets_old_lift() {
        let state = PanelState { anchor: Point { x: 1750, y: 788 },
            last: Rect { x: 1430, y: 340, width: 438, height: 700 }, side: Side::Left,
            panel_width: 320.0, panel_height: 700.0, scale: 1.0, factor: 1.0 };
        assert_eq!(state.anchor_at(state.last, 1.0), state.anchor);
        let moved = Rect { x: 800, y: 100, ..state.last };
        assert_eq!(state.anchor_at(moved, 1.0), Point { x: 1120, y: 100 });
        // Перенос на монитор с другим DPI сохраняет место видимого пульта,
        // а не прибавляет ширину панели, снятую ещё на прежнем мониторе.
        assert_eq!(state.anchor_at(moved, 1.5), Point { x: 1280, y: 100 });
    }

    #[test]
    fn negative_monitor_coordinates_and_ties_keep_the_anchor() {
        let area = Rect { x: -1920, y: -200, width: 1920, height: 1040 };
        let anchor = Point { x: -1019, y: -100 };
        let (target, side) = layout(anchor, BASE, (300, 400), area, Side::Left);
        assert_eq!(side, Side::Left);
        assert_eq!(target.x + 300, anchor.x);
        let (target, side) = layout(anchor, BASE, (300, 400), area, Side::Right);
        assert_eq!(side, Side::Right);
        assert_eq!(target.x, anchor.x);
    }

    #[test]
    fn panels_larger_than_the_work_area_keep_the_base_height() {
        let (target, _) = layout(Point { x: 1700, y: 788 }, BASE, (2000, 1300), AREA, Side::Right);
        assert_eq!((target.x, target.y, target.height), (0, 788, BASE.1));
    }

    #[test]
    fn any_panel_content_height_leaves_the_vertical_window_height_unchanged() {
        let anchor = Point { x: 600, y: 400 };
        for scale in [0.9, 1.0, 1.3] {
            for dpi in [1.0, 1.5] {
                let base = (pixels(118.0 * scale, dpi), pixels(252.0 * scale, dpi));
                for panel_height in [0.0, 90.0, 252.0, 1_300.0] {
                    let panel = (pixels(280.0 * scale, dpi), pixels(panel_height * scale, dpi));
                    let (target, _) = layout(anchor, base, panel, AREA, Side::Right);
                    assert_eq!(target.height, base.1);
                    assert_eq!(target.y, anchor.y);
                }
            }
        }
    }

    #[test]
    fn font_scale_and_monitor_dpi_do_not_change_the_anchor() {
        let anchor = Point { x: 1600, y: 400 };
        for scale in [0.9, 1.0, 1.3] {
            for dpi in [1.0, 1.5] {
                let base = (pixels(118.0 * scale, dpi), pixels(252.0 * scale, dpi));
                let panel = (pixels(280.0 * scale, dpi), pixels(500.0 * scale, dpi));
                let (opened, side) = layout(anchor, base, panel, AREA, Side::Right);
                assert_eq!(side, Side::Left);
                assert_eq!(opened.x + panel.0, anchor.x);
                assert!(opened.y + opened.height <= AREA.height);
                let (closed, _) = layout(anchor, base, (0, 0), AREA, side);
                assert_eq!((closed.x, closed.y), (anchor.x, anchor.y));
            }
        }
    }

    #[test]
    fn rejects_old_layout_scale_and_invalid_measurements() {
        let request = Request { view: 2, size: 0, scale: 1.3, request_id: 7,
            panel_width: 320.0, panel_height: 400.0 };
        assert!(valid(&request, 2, 0, 1.3));
        assert!(!valid(&request, 0, 0, 1.3));
        assert!(!valid(&request, 2, 1, 1.3));
        assert!(!valid(&request, 2, 0, 1.0));
        for value in [f64::NAN, f64::INFINITY, -1.0] {
            assert!(!valid(&Request { panel_width: value, ..request }, 2, 0, 1.3));
        }
        assert!(!valid(&Request { panel_width: 0.0, ..request }, 2, 0, 1.3));
    }

    #[test]
    fn saved_anchor_applies_only_to_its_matching_window_state() {
        let saved = SavedAnchor { anchor: Point { x: -240, y: 500 },
            expanded: Rect { x: -560, y: 340, width: 438, height: 700 } };
        let expanded = saved.expanded;
        let text = serde_json::to_string(&Some(saved)).expect("сериализация якоря");
        let back: Option<SavedAnchor> = serde_json::from_str(&text).expect("чтение якоря");
        let back = back.expect("якорь сохранён");
        assert_eq!(back.restore_for(expanded), Some(Point { x: -240, y: 500 }));
        assert_eq!(back.restore_for(Rect { x: -500, ..expanded }), None,
            "более новая позиция окна не возвращается к старому якорю");
        assert_eq!(back.restore_for(Rect { width: 118, height: 252, ..expanded }), None,
            "уже свёрнутое окно не получает смещение панели повторно");
    }
}
