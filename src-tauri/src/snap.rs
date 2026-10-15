//! Прилипание окон друг к другу и к краям экрана.
//!
//! Перетаскивание за шапку — это `WM_NCLBUTTONDOWN` с `HTCAPTION`, который tao
//! посылает окну сам, а дальше работает модальный цикл `DefWindowProc`. Внутри
//! этого цикла окно получает `WM_MOVING` с предлагаемым местом, и его прямо
//! разрешено поправить. Это и есть штатный способ прилипания на Windows: окно
//! не догоняет мышь рывками, как было бы с `set_position` по событию, и
//! обратной связи «подвинули → событие → подвинули» не возникает.
//!
//! Край ловится с восьми точек, а отпускает на десяти: чуть больше, чем
//! ловит, — иначе окно дрожит между двумя порогами. Клавиши-отмены нет и не
//! нужно: прилипшее окно просто уводят дальше.
//!
//! То же и при растягивании за грань (`WM_SIZING`, этап 53): грань ловится
//! гранями соседей, краем экрана и ровным размером соседа над или под окном.
//! Тянутся мышью только текстовые окна, остальные стоят готовыми размерами.
//!
//! Вешаемся подклассом на готовый HWND. Свой подкласс встаёт перед подклассом
//! tao, и всё, кроме шести нужных сообщений, уходит в `DefSubclassProc` без
//! изменений.
//!
//! Заодно отсюда идёт запись позиций на диск: конец перетаскивания — это
//! единственный момент, когда точно известно, что окно доехало.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, WebviewWindow};

/// Прилипание включено. Выключатель стоит в окне настроек.
///
/// Статический флаг, а не поход в настройки: читает его оконная процедура
/// внутри модального цикла перетаскивания, а событийный цикл в этот момент
/// стоит — спросить Tauri оттуда значит подвесить окно под мышью.
static SNAPPING: AtomicBool = AtomicBool::new(true);

/// Раздаёт оконной процедуре то, что переключили в настройках.
pub fn set_snapping(snapping: bool) {
    SNAPPING.store(snapping, Ordering::Relaxed);
}

fn snapping() -> bool {
    SNAPPING.load(Ordering::Relaxed)
}

/// Взять окно мышью.
///
/// ВАЖНО: перед отправкой сообщения проверяем, нажата ли ещё кнопка. Иначе
/// приложение вешает само себя: `start_dragging` шлёт `WM_NCLBUTTONDOWN`, тот
/// уводит главный поток в модальный цикл `DefWindowProc`, а цикл ждёт
/// отпускания кнопки. Между нажатием во фронте и приходом сообщения проходит
/// целый вызов через IPC, и короткого щелчка хватает, чтобы отпускание
/// случилось раньше, — цикл остаётся ждать того, чего уже не будет.
///
/// Снаружи это выглядит обманчиво: окно рисуется, наведение работает, а
/// нажатия будто проваливаются — приём IPC у WebView2 идёт тем же потоком.
/// Проверка стоит здесь, а не во фронте, потому что только здесь между ней и
/// сообщением не остаётся зазора.
#[tauri::command]
pub fn begin_drag(window: WebviewWindow) {
    #[cfg(windows)]
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};

        // Старший бит — «кнопка нажата прямо сейчас»
        let held = unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } as u16 & 0x8000 != 0;
        if !held {
            return;
        }
    }

    let _ = window.start_dragging();
}

/// Отпустить окно.
///
/// Страховка на случай, если модальный цикл всё-таки начался после отпускания
/// кнопки: `WM_CANCELMODE` завершает его. Во время настоящего перетаскивания
/// сюда не заходят вовсе — пока цикл идёт, мышь принадлежит Windows, и
/// отпускания фронт не видит.
#[tauri::command]
pub fn end_drag(window: WebviewWindow) {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CANCELMODE};

        if let Ok(handle) = window.hwnd() {
            // Именно `Post`, а не `Send`: главный поток может быть внутри
            // цикла, и ждать от него ответа значит встать вместе с ним
            let _ = unsafe {
                PostMessageW(Some(handle), WM_CANCELMODE, WPARAM(0), LPARAM(0))
            };
        }
    }

    #[cfg(not(windows))]
    let _ = window;
}

/// Расстояние, с которого окно притягивается к чужому краю.
///
/// Подобрано живой проверкой: магнит должен помогать попасть, а не решать за
/// пользователя, куда он ведёт окно.
const ATTRACT: i32 = 8;

/// Насколько надо оттянуть прилипшее окно, чтобы оно отлипло.
///
/// ВАЖНО: обязано быть больше притяжения. Меньшее удержание даёт полосу, где
/// окно каждое сообщение то отпускается (вышло за удержание), то ловится
/// снова (ещё в притяжении), — и это видно как дрожь под мышью. Разница между
/// числами и есть вся вязкость: чем она больше, тем сильнее окно упирается.
const RELEASE: i32 = 10;

/// Полоса у края окна, за которую его тянут.
///
/// У tao она шириной `SM_CXFRAME` — около четырёх точек. Для окна с рамкой
/// этого хватает, а карточка виджета своей рамки не рисует, и попасть в
/// четыре точки на глаз не выходит. Отступы у карточки 12–16 точек, так что
/// полоса ложится на пустое место.
const GRIP: i32 = 6;

/// Прямоугольник окна в физических точках экрана.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

/// Прилипло ли окно сейчас — своя память у каждой оси.
///
/// Windows считает предлагаемое место от курсора, а не от прошлого ответа:
/// сколько бы мы окно ни держали, в следующем `WM_MOVING` придёт настоящее
/// положение мыши. Отсюда и получается сопротивление — мы знаем, насколько
/// далеко её уже увели.
#[derive(Clone, Copy, Default, Debug)]
struct Stuck {
    across: bool,
    along: bool,
}

impl Stuck {
    const NONE: Self = Self {
        across: false,
        along: false,
    };
}

/// Сдвиг, притягивающий окно к чужим краям.
///
/// Оси считаются порознь: окно может прилипнуть боком к соседу и при этом
/// свободно ездить вверх-вниз.
fn offset(moving: Rect, work: Option<Rect>, others: &[Rect], stuck: &mut Stuck) -> (i32, i32) {
    let mut across = Vec::new();
    let mut along = Vec::new();

    if let Some(work) = work {
        // Край экрана — только вровень: за него окно не ставят
        across.extend([work.left - moving.left, work.right - moving.right]);
        along.extend([work.top - moving.top, work.bottom - moving.bottom]);
    }

    for other in others {
        // Окно из другого ряда тянуть к себе не должно: иначе виджет из
        // дальнего угла экрана ловил бы движение поперёк себя
        if overlap(moving.top, moving.bottom, other.top, other.bottom) {
            across.extend([
                other.left - moving.left,   // вровень по левому краю
                other.right - moving.right, // вровень по правому
                other.left - moving.right,  // вплотную слева от соседа
                other.right - moving.left,  // вплотную справа от него
            ]);
        }

        if overlap(moving.left, moving.right, other.left, other.right) {
            along.extend([
                other.top - moving.top,
                other.bottom - moving.bottom,
                other.top - moving.bottom,
                other.bottom - moving.top,
            ]);
        }
    }

    (
        nearest(across, &mut stuck.across),
        nearest(along, &mut stuck.along),
    )
}

/// Наименьший сдвиг из предложенных — и признак того, что окно им держится.
///
/// Порог зависит от того, держится ли окно уже: свободное ловится с
/// [`ATTRACT`], прилипшее держится до [`RELEASE`].
fn nearest(deltas: Vec<i32>, stuck: &mut bool) -> i32 {
    let limit = if *stuck { RELEASE } else { ATTRACT };

    let best = deltas
        .into_iter()
        .filter(|delta| delta.abs() <= limit)
        .min_by_key(|delta| delta.abs());

    *stuck = best.is_some();
    best.unwrap_or(0)
}

/// Пересекаются ли отрезки — с тем же допуском, что и притяжение: у окон,
/// поставленных вплотную, края лишь касаются, а до касания есть ещё зазор.
fn overlap(near: i32, far: i32, other_near: i32, other_far: i32) -> bool {
    near <= other_far + ATTRACT && other_near <= far + ATTRACT
}

/// Какая грань тянется по одной оси: ни одна, ближняя (левая или верхняя) или
/// дальняя (правая или нижняя).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Pull {
    Still,
    Near,
    Far,
}

/// Тянущиеся грани окна — по оси на каждую сторону.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Edges {
    x: Pull,
    y: Pull,
}

impl Edges {
    /// Из `wparam` сообщения `WM_SIZING`: `WMSZ_LEFT` = 1 … `WMSZ_BOTTOMRIGHT`
    /// = 8. Числа из документации Windows, а не из крейта: так их видно тесту.
    fn from_wmsz(code: u32) -> Self {
        let x = match code {
            1 | 4 | 7 => Pull::Near,
            2 | 5 | 8 => Pull::Far,
            _ => Pull::Still,
        };
        let y = match code {
            3..=5 => Pull::Near,
            6..=8 => Pull::Far,
            _ => Pull::Still,
        };
        Self { x, y }
    }
}

/// Прямоугольник, растянутый за грани на сдвиг мыши: неподвижные грани стоят
/// на месте.
fn stretch(rect: Rect, edges: Edges, dx: i32, dy: i32) -> Rect {
    let mut out = rect;
    match edges.x {
        Pull::Near => out.left += dx,
        Pull::Far => out.right += dx,
        Pull::Still => {}
    }
    match edges.y {
        Pull::Near => out.top += dy,
        Pull::Far => out.bottom += dy,
        Pull::Still => {}
    }
    out
}

/// Растянутое окно, притянутое к чужим граням и размерам.
///
/// Тянущаяся грань ловится, как в Photoshop: гранями соседей (ровняет края),
/// краем рабочей области и **ровным размером** — неподвижная грань плюс
/// размер соседа, стоящего над или под окном для ширины и слева или справа для
/// высоты. Фильтра «один ряд», как у переноса, тут нет: окна, расставленные
/// сбросом, стоят с зазором в 12 точек, и тот фильтр их не видел бы, а
/// выровнять грань с окном над собой и есть главная просьба.
///
/// `min` — нижний предел окна в физических точках. Держим его сами и с
/// неподвижной противоположной гранью: предел, наложенный системой позже,
/// двигал бы на левой и верхней грани правую и нижнюю.
fn resize(free: Rect, edges: Edges, work: Option<Rect>, others: &[Rect], min: (i32, i32), stuck: &mut Stuck) -> Rect {
    let (left, right) = pull_edge(
        edges.x,
        (free.left, free.right),
        work.map(|work| (work.left, work.right)),
        others.iter().map(|other| (other.left, other.right)),
        min.0,
        &mut stuck.across,
    );
    let (top, bottom) = pull_edge(
        edges.y,
        (free.top, free.bottom),
        work.map(|work| (work.top, work.bottom)),
        others.iter().map(|other| (other.top, other.bottom)),
        min.1,
        &mut stuck.along,
    );

    Rect {
        left,
        top,
        right,
        bottom,
    }
}

/// Одна ось растягивания: куда встать тянущейся грани отрезка `(near, far)`.
fn pull_edge(
    pull: Pull,
    (near, far): (i32, i32),
    work: Option<(i32, i32)>,
    others: impl Iterator<Item = (i32, i32)>,
    min: i32,
    stuck: &mut bool,
) -> (i32, i32) {
    let (edge, fixed) = match pull {
        Pull::Still => return (near, far),
        Pull::Near => (near, far),
        Pull::Far => (far, near),
    };

    // Где встала бы грань при таком размере окна
    let at = |size: i32| if pull == Pull::Near { fixed - size } else { fixed + size };
    // Размер окна, если грань встанет туда
    let size_at = |place: i32| if pull == Pull::Near { fixed - place } else { place - fixed };

    let mut targets = Vec::new();

    if let Some((work_near, work_far)) = work {
        // Край экрана — только своей стороной: за него окно не тянут
        targets.push(if pull == Pull::Near { work_near } else { work_far });
    }

    for (other_near, other_far) in others {
        // Грань вровень с любой гранью соседа — и вплотную к нему тоже
        targets.extend([other_near, other_far]);

        // Ровный размер — только у того, кто стоит в одну колонку с окном
        // (или в один ряд, если тянут высоту): у соседа сбоку ширина
        // случайна, и ловить её значило бы цепляться за пустоту
        if overlap(near.min(far), near.max(far), other_near, other_far) {
            targets.push(at(other_far - other_near));
        }
    }

    // Цель, после которой окно вышло бы меньше предела, не годится вовсе
    let deltas = targets
        .into_iter()
        .filter(|&place| size_at(place) >= min)
        .map(|place| place - edge)
        .collect();

    let snapped = edge + nearest(deltas, stuck);
    // Предел держится и без прилипания: мышь могла уйти за него
    let edge = if size_at(snapped) < min { at(min) } else { snapped };

    if pull == Pull::Near {
        (edge, fixed)
    } else {
        (fixed, edge)
    }
}

#[cfg(windows)]
mod imp {
    use std::sync::{Mutex, OnceLock};

    use tauri::{AppHandle, WebviewWindow};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromRect, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, GetWindowRect, IsWindow, IsWindowVisible, HTBOTTOM, HTBOTTOMLEFT,
        HTBOTTOMRIGHT, HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, WM_ENTERSIZEMOVE,
        WM_EXITSIZEMOVE, WM_MOVING, WM_NCDESTROY, WM_NCHITTEST, WM_SIZING, WINDOWPOS,
        SWP_NOMOVE, SWP_NOSIZE, WM_DPICHANGED, WM_WINDOWPOSCHANGING,
    };

    use super::{offset, resize, snapping, stretch, Edges, Rect, Stuck, GRIP};

    /// Своё значение подкласса: по нему он потом и снимается.
    const SUBCLASS: usize = 0x736e_6170;

    /// Окна, к которым виджет прилипает, — сырой HWND и метка окна.
    ///
    /// Держим сырые HWND, а не окна Tauri: оконная процедура работает внутри
    /// модального цикла перетаскивания, и ходить оттуда к приложению за
    /// списком окон значит ждать событийный цикл, который в этот момент стоит.
    /// Метка нужна растягиванию: по ней из реестра берётся нижний предел окна.
    static TRACKED: Mutex<Vec<(isize, String)>> = Mutex::new(Vec::new());

    /// Размер окна, которое тянут за карточку, и когда стеречь его перестать.
    ///
    /// Перенос за карточку — системный цикл заголовка, и Windows по нему
    /// включает Snap: у края экрана растягивает окно на половину или весь
    /// экран, как обычное. Виджету это не нужно никогда, поэтому смену размера
    /// после переноса мы не пускаем (`WM_WINDOWPOSCHANGING`). Растягивание
    /// само Windows делает уже после `WM_EXITSIZEMOVE`, отсюда секунда запаса.
    struct Moved {
        window: isize,
        size: (i32, i32),
        until: Option<std::time::Instant>,
    }

    static MOVED: Mutex<Option<Moved>> = Mutex::new(None);

    /// Снимает стражу размера с окна: его размер сейчас ставим мы сами.
    pub fn release(window: &WebviewWindow) {
        let Ok(hwnd) = window.hwnd() else {
            return;
        };
        let mut moved = MOVED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if moved.as_ref().is_some_and(|m| m.window == hwnd.0 as isize) {
            *moved = None;
        }
    }

    /// Всё, что живёт ровно одно перетаскивание.
    ///
    /// Тянут всегда одно окно, так что состояние общее: два окна сразу под
    /// мышью не окажутся.
    static DRAG: Mutex<Drag> = Mutex::new(Drag::IDLE);

    struct Drag {
        /// Чужие прямоугольники: во время перетаскивания они не двигаются.
        targets: Vec<Rect>,
        /// За что окно уже держится.
        stuck: Stuck,
        /// С чего перетаскивание началось.
        anchor: Option<Anchor>,
    }

    impl Drag {
        const IDLE: Self = Self {
            targets: Vec::new(),
            stuck: Stuck::NONE,
            anchor: None,
        };
    }

    /// Курсор и окно в момент, когда окно взяли.
    ///
    /// По ним и считается место окна, а не по тому, что предлагает Windows:
    /// предложенное она отмеряет от нынешнего положения окна, а его мы только
    /// что придержали. Удержание тогда складывалось бы само с собой, и окно
    /// отставало бы от мыши всё сильнее — до половины экрана.
    struct Anchor {
        cursor: POINT,
        rect: Rect,
    }

    static APP: OnceLock<AppHandle> = OnceLock::new();

    impl From<RECT> for Rect {
        fn from(rect: RECT) -> Self {
            Self {
                left: rect.left,
                top: rect.top,
                right: rect.right,
                bottom: rect.bottom,
            }
        }
    }

    pub fn attach(app: &AppHandle, window: &WebviewWindow, id: &str, chrome: bool) {
        let Ok(handle) = window.hwnd() else {
            return;
        };
        let _ = APP.set(app.clone());

        // Расширенная полоса захвата нужна только окнам без рамки: у окон
        // с рамкой её рисует Windows
        let grip = usize::from(!chrome);

        unsafe {
            if !SetWindowSubclass(handle, Some(procedure), SUBCLASS, grip).as_bool() {
                eprintln!("окно «{id}» осталось без прилипания");
                return;
            }
        }

        TRACKED.lock().unwrap().push((handle.0 as isize, id.to_owned()));
    }

    /// # Safety
    ///
    /// Зовёт только Windows, отдавая свои же сообщения. У `WM_MOVING` в
    /// `lparam` лежит `RECT`, и правка его на месте — документированный ответ
    /// на это сообщение.
    unsafe extern "system" fn procedure(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _subclass: usize,
        grip: usize,
    ) -> LRESULT {
        match message {
            WM_ENTERSIZEMOVE => {
                *MOVED.lock().unwrap() = None;
                remember_neighbours(window);
            }
            WM_MOVING | WM_DPICHANGED | WM_EXITSIZEMOVE | WM_WINDOWPOSCHANGING => guard_size(window, message, lparam),
            _ => {}
        }

        match message {
            WM_ENTERSIZEMOVE => {}

            // Прилипание выключено — окно едет ровно за мышью, и правка
            // предложенного прямоугольника ему только помешала бы
            WM_MOVING if snapping() => {
                let proposed = &mut *(lparam.0 as *mut RECT);
                let drag = &mut *DRAG.lock().unwrap();

                let free = free_place(drag).unwrap_or_else(|| Rect::from(*proposed));
                let (dx, dy) = offset(free, work_area(free), &drag.targets, &mut drag.stuck);

                *proposed = RECT {
                    left: free.left + dx,
                    top: free.top + dy,
                    right: free.right + dx,
                    bottom: free.bottom + dy,
                };
            }

            // Растягивание за грань. Прямоугольник Windows тоже отмеряет от уже
            // поправленного, поэтому грани ставятся от курсора, как при переносе.
            // Сюда приходят только окна, которые вообще тянутся: у запертых и у
            // окон с готовыми размерами нет `WS_THICKFRAME`, и цикла размера
            // `DefWindowProc` им не начинает
            WM_SIZING if snapping() => {
                let proposed = &mut *(lparam.0 as *mut RECT);
                let edges = Edges::from_wmsz(wparam.0 as u32);
                // Предел — до замка перетаскивания: он берёт замок списка окон,
                // а `remember_neighbours` берёт их в обратном порядке
                let min = floor_of(window);
                let drag = &mut *DRAG.lock().unwrap();

                if let Some(free) = free_size(drag, edges) {
                    let rect = resize(free, edges, work_area(free), &drag.targets, min, &mut drag.stuck);
                    *proposed = RECT {
                        left: rect.left,
                        top: rect.top,
                        right: rect.right,
                        bottom: rect.bottom,
                    };
                }
            }

            WM_EXITSIZEMOVE => {
                *DRAG.lock().unwrap() = Drag::IDLE;
                remember_places();
            }

            // Полоса захвата: сообщение приходит на каждое движение мыши,
            // поэтому дальше собственного прямоугольника тут ничего не спрашиваем
            WM_NCHITTEST if grip == 1 => {
                if let Some(hit) = grip_at(window, lparam) {
                    return LRESULT(hit as isize);
                }
            }

            WM_NCDESTROY => forget(window),

            _ => {}
        }

        DefSubclassProc(window, message, wparam, lparam)
    }

    /// Не даёт Snap растянуть перенесённое окно — см. [`Moved`].
    unsafe fn guard_size(window: HWND, message: u32, lparam: LPARAM) {
        let mut moved = MOVED.lock().unwrap();
        let ours = moved.as_ref().is_some_and(|m| m.window == window.0 as isize);
        match message {
            WM_MOVING if !ours => {
                let mut rect = RECT::default();
                if GetWindowRect(window, &mut rect).is_ok() {
                    *moved = Some(Moved {
                        window: window.0 as isize,
                        size: (rect.right - rect.left, rect.bottom - rect.top),
                        until: None,
                    });
                }
            }
            // Переезд на монитор с другим масштабом меняет размер законно
            WM_DPICHANGED if ours => {
                let rect = &*(lparam.0 as *const RECT);
                if let Some(m) = moved.as_mut() {
                    m.size = (rect.right - rect.left, rect.bottom - rect.top);
                }
            }
            WM_EXITSIZEMOVE if ours => {
                if let Some(m) = moved.as_mut() {
                    m.until = Some(std::time::Instant::now() + std::time::Duration::from_secs(1));
                }
            }
            WM_WINDOWPOSCHANGING if ours => {
                let m = moved.as_ref().unwrap();
                if m.until.is_some_and(|until| std::time::Instant::now() > until) {
                    *moved = None;
                    return;
                }
                let pos = &mut *(lparam.0 as *mut WINDOWPOS);
                if !pos.flags.contains(SWP_NOSIZE) && (pos.cx, pos.cy) != m.size {
                    pos.flags |= SWP_NOSIZE | SWP_NOMOVE;
                }
            }
            _ => {}
        }
    }

    /// Запоминает, где стоят остальные окна.
    ///
    /// Во время перетаскивания они не двигаются, а `WM_MOVING` приходит
    /// десятки раз в секунду — спрашивать систему на каждый шаг мыши незачем.
    unsafe fn remember_neighbours(moving: HWND) {
        let mut targets = Vec::new();
        let anchor = anchor_of(moving);

        let mut tracked = TRACKED.lock().unwrap();
        // Выключенный из трея виджет своё окно уничтожает: чистим на месте,
        // чтобы не считать чужой прямоугольник по переиспользованному HWND
        tracked.retain(|(raw, _)| IsWindow(Some(hwnd(*raw))).as_bool());

        for (raw, _) in tracked.iter() {
            let other = hwnd(*raw);
            if other == moving || !IsWindowVisible(other).as_bool() {
                continue;
            }

            let mut rect = RECT::default();
            if GetWindowRect(other, &mut rect).is_ok() {
                targets.push(Rect::from(rect));
            }
        }

        // Новое перетаскивание начинается свободным: окно, стоящее вплотную,
        // прилипнет обратно тут же, но уже с нуля
        *DRAG.lock().unwrap() = Drag {
            targets,
            stuck: Stuck::NONE,
            anchor,
        };
    }

    unsafe fn anchor_of(window: HWND) -> Option<Anchor> {
        let mut cursor = POINT::default();
        GetCursorPos(&mut cursor).ok()?;

        let mut rect = RECT::default();
        GetWindowRect(window, &mut rect).ok()?;

        Some(Anchor {
            cursor,
            rect: Rect::from(rect),
        })
    }

    /// Куда окно уехало бы, если бы ни за что не цеплялось.
    unsafe fn free_place(drag: &Drag) -> Option<Rect> {
        let anchor = drag.anchor.as_ref()?;

        let mut cursor = POINT::default();
        GetCursorPos(&mut cursor).ok()?;

        let dx = cursor.x - anchor.cursor.x;
        let dy = cursor.y - anchor.cursor.y;

        Some(Rect {
            left: anchor.rect.left + dx,
            top: anchor.rect.top + dy,
            right: anchor.rect.right + dx,
            bottom: anchor.rect.bottom + dy,
        })
    }

    /// Каким окно растянулось бы, ни за что не цепляясь: тянущиеся грани
    /// уехали за мышью, остальные стоят, где стояли.
    unsafe fn free_size(drag: &Drag, edges: Edges) -> Option<Rect> {
        let anchor = drag.anchor.as_ref()?;

        let mut cursor = POINT::default();
        GetCursorPos(&mut cursor).ok()?;

        Some(stretch(
            anchor.rect,
            edges,
            cursor.x - anchor.cursor.x,
            cursor.y - anchor.cursor.y,
        ))
    }

    /// Нижний предел окна в физических точках: логический из реестра, помноженный
    /// на масштаб монитора, где окно стоит. Округляем вверх — предел, недобранный
    /// на полточки, Windows всё равно добрала бы, двинув противоположную грань.
    unsafe fn floor_of(window: HWND) -> (i32, i32) {
        let raw = window.0 as isize;
        let id = TRACKED
            .lock()
            .unwrap()
            .iter()
            .find(|(kept, _)| *kept == raw)
            .map(|(_, id)| id.clone())
            .unwrap_or_default();

        let (width, height) = crate::windows::min_logical(&id);
        let dpi = match GetDpiForWindow(window) {
            0 => 1.0,
            dpi => f64::from(dpi) / 96.0,
        };

        ((width * dpi).ceil() as i32, (height * dpi).ceil() as i32)
    }

    /// Рабочая область монитора, на котором окно оказалось: край экрана —
    /// это край без панели задач, поставленный на неё виджет был бы наполовину
    /// закрыт.
    unsafe fn work_area(place: Rect) -> Option<Rect> {
        let rect = RECT {
            left: place.left,
            top: place.top,
            right: place.right,
            bottom: place.bottom,
        };
        let monitor = MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST);

        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };

        GetMonitorInfoW(monitor, &mut info)
            .as_bool()
            .then(|| Rect::from(info.rcWork))
    }

    /// Куда попал курсор у края окна. Курсоры Windows подставит сама: для
    /// неклиентской части tao отдаёт `WM_SETCURSOR` в `DefWindowProc`.
    unsafe fn grip_at(window: HWND, lparam: LPARAM) -> Option<u32> {
        let mut rect = RECT::default();
        GetWindowRect(window, &mut rect).ok()?;

        // Координаты в `lparam` экранные и знаковые: на втором мониторе слева
        // они отрицательные, и без приведения через i16 ушли бы в миллионы
        let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
        let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;

        if x < rect.left || x >= rect.right || y < rect.top || y >= rect.bottom {
            return None;
        }

        let left = x < rect.left + GRIP;
        let right = x >= rect.right - GRIP;
        let top = y < rect.top + GRIP;
        let bottom = y >= rect.bottom - GRIP;

        Some(match (left, right, top, bottom) {
            (true, _, true, _) => HTTOPLEFT,
            (_, true, true, _) => HTTOPRIGHT,
            (true, _, _, true) => HTBOTTOMLEFT,
            (_, true, _, true) => HTBOTTOMRIGHT,
            (true, ..) => HTLEFT,
            (_, true, ..) => HTRIGHT,
            (_, _, true, _) => HTTOP,
            (_, _, _, true) => HTBOTTOM,
            // Середина окна — дело содержимого, а не рамки
            _ => return None,
        })
    }

    /// Пишет позиции на диск.
    ///
    /// Не на месте: изнутри оконной процедуры это значило бы файловый
    /// ввод-вывод и опрос окон, не выходя из модального цикла. Работа встаёт в
    /// очередь главного потока, и к её исполнению цикл уже закончится.
    fn remember_places() {
        let Some(app) = APP.get().cloned() else {
            return;
        };

        // ВАЖНО: **главным потоком, а не из пула.** Плагин window-state держит
        // свой замок, пока спрашивает окна о месте, а спрошенное из пула окно
        // ждёт главный поток. Главный же поток, разбирая `Moved` того же окна,
        // ждёт этот замок — и оба стоят навсегда. Очередь главного потока
        // дойдёт до записи уже после модального цикла перетаскивания
        crate::windows::on_main(&app, |app| crate::windows::save_places(&app));
    }

    unsafe fn forget(window: HWND) {
        let raw = window.0 as isize;
        TRACKED.lock().unwrap().retain(|(kept, _)| *kept != raw);
        let _ = RemoveWindowSubclass(window, Some(procedure), SUBCLASS);
    }

    /// Обратно из хранимого числа. Держать сам `HWND` в статике нельзя:
    /// он не `Send`, а список общий на все окна.
    fn hwnd(raw: isize) -> HWND {
        HWND(raw as *mut std::ffi::c_void)
    }
}

/// Вешает прилипание на окно.
///
/// Берёт `id` и признак рамки, а не запись реестра: окно настроек виджетом не
/// является, а прилипать и запоминать своё место обязано наравне со всеми.
#[cfg(windows)]
pub fn attach(app: &AppHandle, window: &WebviewWindow, id: &str, chrome: bool) {
    imp::attach(app, window, id, chrome);
}

#[cfg(not(windows))]
pub fn attach(_app: &AppHandle, _window: &WebviewWindow, _id: &str, _chrome: bool) {}

/// Снимает стражу от Snap перед размером, который ставит само приложение.
///
/// Стража запрещает окну менять размер секунду после переноса, не разбирая,
/// кто его меняет, — и выбранный в меню размер в эту секунду молча
/// отменялся бы, хотя номер его уже записан.
#[cfg(windows)]
pub fn release_size_guard(window: &WebviewWindow) {
    imp::release(window);
}

#[cfg(not(windows))]
pub fn release_size_guard(_window: &WebviewWindow) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Свободное окно: так начинается каждое перетаскивание.
    fn free() -> Stuck {
        Stuck::default()
    }

    /// Окно 200×100 в точке (x, y).
    fn window(x: i32, y: i32) -> Rect {
        Rect {
            left: x,
            top: y,
            right: x + 200,
            bottom: y + 100,
        }
    }

    fn screen() -> Rect {
        Rect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        }
    }

    #[test]
    fn sticks_to_the_screen_edge() {
        // Три точки от левого верхнего угла — притягиваем к нулю
        let (dx, dy) = offset(window(3, 3), Some(screen()), &[], &mut free());
        assert_eq!((dx, dy), (-3, -3));

        // И к дальним краям тоже: считается правый край, а не левый
        let (dx, dy) = offset(window(1717, 939), Some(screen()), &[], &mut free());
        assert_eq!((dx, dy), (3, 1));
    }

    /// Панель задач — часть рабочей области, и виджет встаёт над ней.
    #[test]
    fn screen_edge_means_the_work_area() {
        let (_, dy) = offset(window(400, 938), Some(screen()), &[], &mut free());
        assert_eq!(dy, 2, "нижний край окна должен лечь на 1040");
    }

    #[test]
    fn sticks_side_by_side() {
        let neighbour = window(400, 300);

        // Подводим слева вплотную: правый край едет к левому краю соседа
        let (dx, dy) = offset(window(197, 300), None, &[neighbour], &mut free());
        assert_eq!((dx, dy), (3, 0));

        // И заодно ровняется по верхнему краю
        let (_, dy) = offset(window(197, 302), None, &[neighbour], &mut free());
        assert_eq!(dy, -2);
    }

    #[test]
    fn far_away_stays_where_put() {
        let neighbour = window(400, 300);

        // Тринадцать точек до чужого края — уже за порогом
        let far = offset(window(187, 300), None, &[neighbour], &mut free());
        assert_eq!(far, (0, 0));

        let across_the_screen = offset(window(500, 500), Some(screen()), &[neighbour], &mut free());
        assert_eq!(across_the_screen, (0, 0));
    }

    /// Держит чуть дальше, чем ловит. Полоса между порогами обязана иметь
    /// два разных исхода — от того, прилипло окно или нет: одинаковый исход
    /// на обоих порогах и есть та самая дрожь под мышью.
    #[test]
    fn holds_on_a_little_longer_than_it_catches() {
        let neighbour = window(400, 300);
        let mut stuck = free();

        // Встали вплотную
        let (dx, _) = offset(window(200, 300), None, &[neighbour], &mut stuck);
        assert_eq!(dx, 0);
        assert!(stuck.across);

        // Девять точек прилипшее ещё держит, хотя ловит только с восьми
        let (dx, _) = offset(window(191, 300), None, &[neighbour], &mut stuck);
        assert_eq!(dx, 9);

        // А на двенадцати отпустило, и окно поехало дальше за мышью
        let (dx, _) = offset(window(188, 300), None, &[neighbour], &mut stuck);
        assert_eq!(dx, 0);
        assert!(!stuck.across);

        // Обратно те же девять точек уже не притягивают: так и ставится отступ
        let (dx, _) = offset(window(191, 300), None, &[neighbour], &mut stuck);
        assert_eq!(dx, 0);
    }

    /// Нахлёст должен оставаться возможным: иначе одно окно на другое не
    /// положить вовсе.
    #[test]
    fn overlapping_is_still_possible() {
        let neighbour = window(400, 300);

        // Сдвинуто внутрь соседа на полсотни точек — ни один край не близко
        let inside = offset(window(450, 350), None, &[neighbour], &mut free());
        assert_eq!(inside, (0, 0));
    }

    /// Окно из другого ряда не должно тянуть к себе поперёк экрана.
    #[test]
    fn ignores_windows_from_another_row() {
        let far = window(400, 900);

        // По горизонтали край в трёх точках — притянуло бы, но по вертикали
        // окна не встречаются вовсе
        let apart = offset(window(197, 300), None, &[far], &mut free());
        assert_eq!(apart, (0, 0));
    }

    /// Из нескольких краёв рядом выбирается ближний.
    #[test]
    fn takes_the_closest_edge() {
        // Сосед на две точки уже: правые края сходятся ближе, чем левые
        let neighbour = Rect {
            left: 400,
            top: 300,
            right: 598,
            bottom: 400,
        };

        let (dx, _) = offset(window(397, 300), None, &[neighbour], &mut free());
        assert_eq!(dx, 1, "выбирается ближний край, а не первый попавшийся");
    }

    /// Предел окна в тестах растягивания: 140×64, общий предел виджетов.
    const MIN: (i32, i32) = (140, 64);

    fn right_edge() -> Edges {
        Edges::from_wmsz(2)
    }

    fn left_edge() -> Edges {
        Edges::from_wmsz(1)
    }

    /// Коды `WMSZ_*` раскладываются по осям: угол тянет обе, сторона — одну.
    #[test]
    fn sizing_codes_name_their_edges() {
        assert_eq!(Edges::from_wmsz(1), Edges { x: Pull::Near, y: Pull::Still });
        assert_eq!(Edges::from_wmsz(6), Edges { x: Pull::Still, y: Pull::Far });
        assert_eq!(Edges::from_wmsz(4), Edges { x: Pull::Near, y: Pull::Near });
        assert_eq!(Edges::from_wmsz(8), Edges { x: Pull::Far, y: Pull::Far });
    }

    /// Главная просьба: правая грань ловится правой гранью окна над собой, даже
    /// если окна стоят с зазором сброса в 12 точек, а не вплотную.
    #[test]
    fn a_stretched_edge_lines_up_with_the_window_above() {
        let above = Rect { left: 100, top: 100, right: 400, bottom: 200 };
        // Под ним с зазором 12, уже на сотню; тянем правую грань почти до 400
        let stretched = Rect { left: 100, top: 212, right: 395, bottom: 312 };

        let rect = resize(stretched, right_edge(), None, &[above], MIN, &mut free());
        assert_eq!(rect.right, 400);
        assert_eq!(rect.left, 100, "левая грань не тянется и стоять обязана");
    }

    /// Ровный размер: окно над собой, сдвинутое вбок, всё равно даёт свою
    /// ширину — «сравняться по размерам», а не только по краю.
    #[test]
    fn a_stretched_edge_matches_the_width_above() {
        // Сосед шириной 300, левее на 40: края не совпадут ни при какой ширине
        let above = Rect { left: 60, top: 100, right: 360, bottom: 200 };
        let stretched = Rect { left: 100, top: 212, right: 396, bottom: 312 };

        let rect = resize(stretched, right_edge(), None, &[above], MIN, &mut free());
        assert_eq!(rect.right - rect.left, 300);
    }

    /// Ширина соседа сбоку не ловится: он не над окном, и его ширина случайна.
    #[test]
    fn a_neighbour_on_the_side_lends_no_width() {
        let aside = Rect { left: 600, top: 212, right: 900, bottom: 312 };
        // Ширина 297 — в трёх точках от ширины соседа, но сосед стоит сбоку
        let stretched = Rect { left: 100, top: 212, right: 397, bottom: 312 };

        let rect = resize(stretched, right_edge(), None, &[aside], MIN, &mut free());
        assert_eq!(rect.right, 397);
    }

    /// Держит чуть дальше, чем ловит, — как и при переносе.
    #[test]
    fn a_stretched_edge_holds_a_little_longer_than_it_catches() {
        let above = Rect { left: 100, top: 100, right: 400, bottom: 200 };
        let at = |right: i32| Rect { left: 100, top: 212, right, bottom: 312 };
        let mut stuck = free();

        assert_eq!(resize(at(398), right_edge(), None, &[above], MIN, &mut stuck).right, 400);
        // Девять точек держит, хотя ловит только с восьми
        assert_eq!(resize(at(391), right_edge(), None, &[above], MIN, &mut stuck).right, 400);
        // На двенадцати отпустило
        assert_eq!(resize(at(388), right_edge(), None, &[above], MIN, &mut stuck).right, 388);
    }

    /// Левая грань ловится и краем экрана; правая при этом стоит на месте.
    #[test]
    fn the_left_edge_moves_alone() {
        let stretched = Rect { left: 5, top: 300, right: 400, bottom: 400 };

        let rect = resize(stretched, left_edge(), Some(screen()), &[], MIN, &mut free());
        assert_eq!((rect.left, rect.right), (0, 400));
    }

    /// Мышь ушла за предел — грань встаёт на предел, противоположная стоит.
    #[test]
    fn the_floor_holds_with_the_far_edge_in_place() {
        let squeezed = Rect { left: 330, top: 300, right: 400, bottom: 400 };

        let rect = resize(squeezed, left_edge(), None, &[], MIN, &mut free());
        assert_eq!((rect.left, rect.right), (260, 400));
    }

    /// Цель, которая сделала бы окно меньше предела, не притягивает вовсе.
    #[test]
    fn a_target_under_the_floor_is_ignored() {
        // Грань соседа в 100 точках от левой — окно вышло бы уже 140
        let neighbour = Rect { left: 0, top: 0, right: 200, bottom: 50 };
        let stretched = Rect { left: 100, top: 300, right: 205, bottom: 400 };

        let rect = resize(stretched, right_edge(), None, &[neighbour], MIN, &mut free());
        assert_eq!(rect.right, 240, "предел, а не грань соседа в двухстах");
    }
}
