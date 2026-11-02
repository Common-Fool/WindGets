//! Углы карточек по соседям: стык двух виджетов — прямой, свободный край —
//! скруглённый (1 ноября 2026, слово пользователя: виджеты стыкуются, и между
//! ними остаётся только тонкая полоса двух рамок).
//!
//! Считает Rust: только он видит все окна разом. Окну уходит событие
//! `corners` со списком прямых углов — и только когда список поменялся, хотя
//! пересчёт идёт на каждый сдвиг и размер любого окна (окон полтора десятка,
//! счёт — сравнение прямоугольников). Своё нынешнее окно спрашивает и само
//! (`window_corners`): событие, ушедшее до того, как страница начала слушать,
//! потерялось бы.

use std::collections::HashMap;
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::widgets;

/// Стык с запасом: прилипание ставит окна вплотную, но округление DPI
/// оставляет точку-другую.
const TOUCH: i32 = 2;

/// Сосед на стыке делает угол прямым, если доходит до него ближе этого (в
/// точках экрана): начнись он ниже скругления — угол торчал бы над пустотой.
const REACH: i32 = 12;

/// Прямоугольник окна в физических точках: лево, верх, право, низ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Прямые углы окна: верх-лево, верх-право, низ-право, низ-лево.
pub type Square = [bool; 4];

/// Последний разосланный ответ каждому окну.
static SENT: Mutex<Option<HashMap<String, Square>>> = Mutex::new(None);

/// Какие углы `me` прямые при соседях `others`.
///
/// Угол прямой, если к одной из двух его граней прилегает сосед и доходит
/// вдоль неё до самого угла (ближе [`REACH`]).
pub fn square(me: Rect, others: &[Rect]) -> Square {
    let near = |a: i32, b: i32| (a - b).abs() <= TOUCH;
    let above = |o: &Rect| near(o.bottom, me.top) && o.left < me.right && me.left < o.right;
    let below = |o: &Rect| near(o.top, me.bottom) && o.left < me.right && me.left < o.right;
    let left = |o: &Rect| near(o.right, me.left) && o.top < me.bottom && me.top < o.bottom;
    let right = |o: &Rect| near(o.left, me.right) && o.top < me.bottom && me.top < o.bottom;

    let any = |test: &dyn Fn(&Rect) -> bool| others.iter().any(test);
    [
        any(&|o| above(o) && o.left <= me.left + REACH) || any(&|o| left(o) && o.top <= me.top + REACH),
        any(&|o| above(o) && o.right >= me.right - REACH) || any(&|o| right(o) && o.top <= me.top + REACH),
        any(&|o| below(o) && o.right >= me.right - REACH) || any(&|o| right(o) && o.bottom >= me.bottom - REACH),
        any(&|o| below(o) && o.left <= me.left + REACH) || any(&|o| left(o) && o.bottom >= me.bottom - REACH),
    ]
}

/// Окна виджетов на экране с их прямоугольниками. Служебные (с рамкой) и окно
/// настроек в стыки не входят. `asking` — окно, которое спрашивает само: его
/// прямоугольник берётся и у спрятанного — страница грузится раньше, чем окно
/// выходит на экран, и без этого получала бы «все углы круглые».
fn shown_rects<R: Runtime>(app: &AppHandle<R>, asking: Option<&str>) -> Vec<(&'static str, Rect)> {
    widgets::shown()
        .filter(|widget| !widget.chrome)
        .filter_map(|widget| {
            let window = app.get_webview_window(widget.id)?;
            let hidden = !window.is_visible().unwrap_or(false) || window.is_minimized().unwrap_or(false);
            if hidden && asking != Some(widget.id) {
                return None;
            }
            let at = window.outer_position().ok()?;
            let size = window.outer_size().ok()?;
            Some((widget.id, Rect {
                left: at.x,
                top: at.y,
                right: at.x + size.width as i32,
                bottom: at.y + size.height as i32,
            }))
        })
        .collect()
}

fn of(id: &str, rects: &[(&'static str, Rect)]) -> Square {
    let Some((_, me)) = rects.iter().find(|(other, _)| *other == id) else {
        return [false; 4];
    };
    let others: Vec<Rect> = rects.iter().filter(|(other, _)| *other != id).map(|(_, rect)| *rect).collect();
    square(*me, &others)
}

/// Пересчитать всех и разослать изменившееся. Из главного потока: зовут его
/// события окон.
pub fn refresh<R: Runtime>(app: &AppHandle<R>) {
    let rects = shown_rects(app, None);
    let mut sent = SENT.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let sent = sent.get_or_insert_with(HashMap::new);
    for widget in widgets::shown().filter(|widget| !widget.chrome) {
        let now = of(widget.id, &rects);
        if sent.get(widget.id) == Some(&now) {
            continue;
        }
        // Адресно: `emit` в Tauri 2 — рассылка всем окнам
        if app.emit_to(widget.id, "corners", now).is_ok() {
            sent.insert(widget.id.to_owned(), now);
        }
    }
}

/// Прямые углы окна, которое спрашивает, — на его первом кадре и после того,
/// как оно начало слушать `corners`.
///
/// ВАЖНО: **ответ записывается как разосланный.** Иначе событие, ушедшее до
/// подписки страницы, числилось бы доставленным, повторно не шло, и окно
/// «забывало» соседа до следующего сдвига (2 ноября 2026).
#[tauri::command]
pub fn window_corners<R: Runtime>(app: AppHandle<R>, window: tauri::WebviewWindow<R>) -> Square {
    let label = window.label();
    let now = of(label, &shown_rects(&app, Some(label)));
    let mut sent = SENT.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    sent.get_or_insert_with(HashMap::new).insert(label.to_owned(), now);
    now
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect { left, top, right, bottom }
    }

    #[test]
    fn a_lone_window_keeps_every_corner_round() {
        assert_eq!(square(rect(0, 0, 100, 100), &[]), [false; 4]);
        // Рядом, но не вплотную
        assert_eq!(square(rect(0, 0, 100, 100), &[rect(110, 0, 200, 100)]), [false; 4]);
    }

    #[test]
    fn side_by_side_windows_square_the_inner_corners() {
        let cpu = rect(0, 100, 180, 400);
        let gpu = rect(180, 100, 360, 400);
        assert_eq!(square(cpu, &[gpu]), [false, true, true, false]);
        assert_eq!(square(gpu, &[cpu]), [true, false, false, true]);
    }

    #[test]
    fn a_wide_neighbour_above_squares_both_top_corners() {
        let fps = rect(0, 0, 360, 100);
        let cpu = rect(0, 101, 180, 400);
        let gpu = rect(180, 101, 360, 400);
        assert_eq!(square(cpu, &[fps, gpu]), [true, true, true, false]);
        // У широкого сверху прямые оба нижних: соседи доходят до обоих концов
        assert_eq!(square(fps, &[cpu, gpu]), [false, false, true, true]);
    }

    #[test]
    fn a_neighbour_short_of_the_corner_leaves_it_round() {
        // Сосед справа начинается заметно ниже верха — верхний правый угол
        // торчит над пустотой и остаётся круглым
        let me = rect(0, 0, 180, 300);
        let low = rect(180, 100, 360, 300);
        assert_eq!(square(me, &[low]), [false, false, true, false]);
    }
}
