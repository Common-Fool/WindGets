//! Окна виджетов: сборка и доводка, позиции между запусками, «поверх всех»
//! и прозрачность для мыши.
//!
//! В `tauri.conf.json` окон нет вовсе — там их пришлось бы описывать
//! заранее, а окно есть только у включённого модуля (`modules.rs`).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_window_state::{AppHandleExt, StateFlags, WindowExt};

use crate::settings::{self, SettingsState, WidgetSettings};
use crate::snap;
use crate::widgets::{self, Widget};
use crate::window_effects;

mod sound_panel;

/// Масштаб, при котором снят нынешний размер окна, по меткам окон.
///
/// В `settings.json` ему не место: там прямо записано, что позиции и размеры
/// окон туда не попадают — их ведёт `tauri-plugin-window-state` в своём файле.
/// А на диске это число и не нужно: масштаб меняют ползунком при работающем
/// приложении, и в момент правки обе величины — прежняя и новая — известны.
///
/// ВАЖНО: **по числу на окно, а не одно на всех.** С тех пор как масштаб бывает
/// свой у виджета, общего «прежнего масштаба» не существует: подвинув общий
/// ползунок, надо пересчитать одних и не тронуть других — тех, у кого своё
/// число и кого общий ползунок не касается вовсе.
///
/// Цена — правка `scale` руками в закрытом приложении разъедется с
/// размерами окон. Лечится кнопкой «Сбросить положение и размер».
static LAYOUT_SCALE: Mutex<BTreeMap<String, f64>> = Mutex::new(BTreeMap::new());

/// На сколько точек окно поднято ради раскрытой панели, по меткам окон.
///
/// Помнить обязательно: подняли мы его сами, и вернуть на место при закрытии
/// панели больше некому. Без этого окно после каждого раскрытия оставалось бы
/// выше, чем стояло, и за вечер уползало бы к верхнему краю.
static LIFTED: Mutex<BTreeMap<String, Lift>> = Mutex::new(BTreeMap::new());

/// Долг подъёма и место, куда мы окно этим подъёмом поставили.
///
/// ВАЖНО: **место здесь не для отчётности, а чтобы отличить своё поднятие от
/// чужой руки.** Долг годится, только пока окно стоит там, где мы его
/// оставили: раскрытое окно, перенесённое мышью на середину экрана, при
/// сворачивании панели опускалось на весь прежний долг — то есть уезжало вниз
/// из-под руки. Перетаскивание об этой таблице не знает и знать не должно:
/// оконная процедура работает внутри модального цикла, и ходить ей сюда
/// незачем — расхождение места видно и отсюда.
struct Lift {
    owed: i32,
    y: i32,
}

/// Помним положение и размер. Развёрнутость и видимость не помним: для
/// виджета это бессмыслица, а восстановленный «максимум» его ломает.
fn state_flags() -> StateFlags {
    StateFlags::POSITION | StateFlags::SIZE
}

/// Пишет положение и размер всех окон на диск.
///
/// Плагин копит их в памяти, а на диск они уходят только отсюда. Зовётся не
/// только на выходе, но и по концу каждого перетаскивания ([`snap`]): в
/// разработке приложение не выходит, а погибает вместе с пересборкой, и
/// сохранения на выходе не случается вовсе.
///
/// Об отказе сообщаем: молчание тут однажды стоило полдня расставленных
/// заново окон.
pub fn save_places(app: &AppHandle) {
    if let Err(error) = app.save_window_state(state_flags()) {
        eprintln!("позиции окон не сохранились: {error}");

        // Заодно называем виновника: отказ приходит общий, а молчит одно окно
        for (label, window) in app.webview_windows() {
            match window.outer_position() {
                Ok(place) => eprintln!("[окна] {label}: отвечает, {},{}", place.x, place.y),
                Err(error) => eprintln!("[окна] {label}: МОЛЧИТ — {error}"),
            }
        }
    } else {
        sound_panel::save_anchor(app);
    }
}

/// Идёт выход из приложения.
///
/// Закрытие окна крестиком означает «выключить виджет», но при выходе окна
/// закрываются тоже — и без этого признака приложение забывало бы, что было
/// включено, ровно один раз: в момент собственного завершения.
#[derive(Default)]
pub struct Shutdown(AtomicBool);

impl Shutdown {
    pub fn begin(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn started(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Собирает окна включённых модулей. Зовётся один раз при старте, из главного
/// потока до событийного цикла.
///
/// Выключенный модуль окна не получает вовсе: webview — самое дорогое, что у
/// виджета есть, и держать его ради спрятанного окна незачем. Прежде собирались
/// все окна разом, а выключенные прятались: сборка на ходу вешала приём IPC.
/// Вешала она только из синхронной команды (см. [`create`]); на ходу окна
/// теперь собирает `modules::set` из пула, и это двести миллисекунд.
pub fn prepare_all(app: &AppHandle) {
    let settings = app.state::<SettingsState>().get();

    for widget in widgets::shown() {
        // Выключенный модуль не грузится вовсе: ни окна, ни webview в памяти
        if !settings.widget(widget).enabled {
            continue;
        }

        if let Err(error) = build_window(app, widget, !settings.hidden_all) {
            // Одно упавшее окно не повод не собирать остальные
            eprintln!("{error}");
        }
    }
}

/// Масштаб, при котором в последний раз ставили размер этому окну.
///
/// Неизвестное окно — то, которого мы ещё не мерили: отвечаем нынешним
/// значением, чтобы пересчёт его не тронул.
fn layout_scale(id: &str, now: f64) -> f64 {
    LAYOUT_SCALE
        .lock()
        .ok()
        .and_then(|table| table.get(id).copied())
        .unwrap_or(now)
}

fn set_layout_scale(id: &str, value: f64) {
    if let Ok(mut table) = LAYOUT_SCALE.lock() {
        table.insert(id.to_owned(), value);
    }
}

/// Нижний предел окна в логических точках — тот же, что стоит у него
/// `min_inner_size`.
///
/// Нужен прилипанию при растягивании (`snap.rs`): оно само ставит грань и
/// обязано держать предел, не двигая противоположную. Читает его оконная
/// процедура внутри модального цикла, поэтому только реестр и статическая
/// таблица масштабов — к Tauri отсюда не ходим.
pub fn min_logical(id: &str) -> (f64, f64) {
    if id == crate::settings_window::LABEL {
        return crate::settings_window::MIN_SIZE;
    }

    match widgets::find(id) {
        Some(widget) => {
            let floor = widget.min_size(layout_scale(id, 1.0));
            (floor.width, floor.height)
        }
        None => (widgets::MIN_WIDTH, widgets::MIN_HEIGHT),
    }
}

/// Пересчитывает долг подъёма вслед за масштабом.
///
/// Долг записан в физических точках, и после смены масштаба он мерит уже не то:
/// окно поднимали под панель прежнего размера, а опускать придётся из-под
/// панели нового.
fn restretch_lift(id: &str, ratio: f64) {
    let Ok(mut lifted) = LIFTED.lock() else {
        return;
    };
    if let Some(lift) = lifted.get_mut(id) {
        lift.owed = (f64::from(lift.owed) * ratio).round() as i32;
    }
}

/// Пересчитывает размеры окон вслед за кеглем.
///
/// Множится накрученный мышью размер, а не реестровый: подобранный руками
/// размер терять нельзя. То же правило, по которому пульт Spotify растит окно
/// под список плейлистов от нынешней высоты, а не от числа.
///
/// Берём все собранные окна показываемых виджетов, **не глядя на видимость**:
/// спрятанный рубильником виджет не должен вернуться в прежнем размере. Тем
/// и отличается от [`reset_places`], который трогает только видимые — там
/// невидимым нечего занимать место в сетке.
pub fn rescale(app: &AppHandle) {
    let settings = app.state::<SettingsState>().get();
    let mut moved = false;

    for widget in widgets::shown() {
        let to = settings.scale_of(widget);
        let from = layout_scale(widget.id, to);
        if (to - from).abs() < f64::EPSILON {
            continue;
        }

        let Some(window) = app.get_webview_window(widget.id) else {
            continue;
        };

        // Вертикальный звук растёт в обе стороны: обычный путь ниже вернул
        // бы только реестровую ширину и отрезал раскрытую боковую панель.
        if widget.id == "sound" && settings.widget(widget).view == 2
            && sound_panel::rescale(&window, to) {
            let _ = window.set_min_size(Some(widget.min_size(to)));
            set_layout_scale(widget.id, to);
            moved = true;
            continue;
        }

        let (Ok(size), Ok(factor)) = (window.inner_size(), window.scale_factor()) else {
            continue;
        };

        let now = size.to_logical::<f64>(factor);
        let ratio = to / from;

        if widget.flexible {
            let _ = window.set_size(LogicalSize::new(now.width * ratio, now.height * ratio));
        } else {
            // ВАЖНО: **высоту множим, а не берём из реестра.** Ставить сюда
            // готовый размер казалось честнее — ширину мы так и ставим, — но
            // высота бывает не только реестровой: у звука, пульта и погоды
            // часть её доросла под раскрытую панель, и подменённая числом она
            // стирала бы панель, не сказав об этом фронту. Тот про подмену не
            // узнаёт (`widget-size` отсюда не уходит), держит свой долг и по
            // закрытию панели вычитал его из УЖЕ уменьшенного окна — высота
            // упиралась в единицу, и виджет превращался в полоску.
            //
            // Заодно чинится второе: у размера без своей высоты
            // (`height: None` — «Без температур» у накопителей) она не
            // менялась вовсе, пока содержимое ехало за масштабом, и карточка
            // резалась снизу.
            //
            // Реестровое число тут ничего и не теряет: окно нерастягиваемое,
            // высота у него и так реестровая, помноженная на прежний масштаб,
            // — умножение на отношение даёт ровно то же самое.
            let own = settings.widget(widget);
            let (width, _) = widget.design_size(settings.design, own.view, own.size, own.layout);
            let _ = window.set_size(LogicalSize::new(width * to, now.height * ratio));
        }

        // Долг подъёма записан в точках прежнего масштаба: не растянув его,
        // сворачивание панели опустило бы окно не на столько, на сколько его
        // подняли
        restretch_lift(widget.id, ratio);

        let _ = window.set_min_size(Some(widget.min_size(to)));

        set_layout_scale(widget.id, to);
        moved = true;
    }

    // Записи позиций стоят денег на диске, а ползунок отпускают и вхолостую:
    // не тронув ни одного окна, писать нечего
    if moved {
        save_places(app);
    }
}

/// Ставит окну готовый размер из реестра.
///
/// ВАЖНО: **высоту трогаем, только если она у размера своя.** У большинства
/// карточек содержимое растягивается под окно — замер показал, что сеть,
/// спидтест и пароли при окне в 900 точек требуют ровно 900, — и высота
/// у них та, какую оставили мышью или сбросом. Своя высота нашлась лишь у
/// видеокарты и процессора: там пара показателей на узкой карточке переносит
/// второй столбец под первый, и места нужно БОЛЬШЕ, чем при полной ширине.
pub fn apply_preset(
    window: &WebviewWindow,
    widget: &Widget,
    index: usize,
    view: usize,
    scale: f64,
) {
    let settings = window.app_handle().state::<SettingsState>().get();
    let own = settings.widget(widget);
    let (width, own_height) = widget.design_size(settings.design, view, index, own.layout);

    let (Ok(factor), Ok(size)) = (window.scale_factor(), window.inner_size()) else {
        return;
    };
    let now = size.to_logical::<f64>(factor);

    // Вид уже учтён в `preset_size`: у одних он поправляет высоту, у других
    // приносит свой набор размеров целиком
    let height = match own_height {
        Some(height) => height * scale,
        None => now.height,
    };

    let requested = LogicalSize::new(width * scale, height);
    if let Err(error) = window.set_size(requested) {
        eprintln!("{}: применение пресета: {error}", widget.id);
    }
}

/// Компоновка меняет основу без события размера: открытые панели остаются.
/// Свободные текстовые окна сохраняют выбранные руками ширину и высоту.
pub fn set_layout(app: &AppHandle, id: &str) {
    let Some(widget) = widgets::find(id) else { return; };
    if widget.flexible { return; }
    let Some(window) = app.get_webview_window(id) else { return; };
    let settings = app.state::<SettingsState>().get();
    let own = settings.widget(widget);
    let scale = settings.scale_of(widget);
    apply_preset(&window, widget, own.size, own.view, scale);
    set_layout_scale(id, scale);
    save_places(app);
}

/// Окну поставили высоту числом (`useWindowResized` во фронте): раскрытые
/// панели сворачиваются, хук роста забывает свой долг.
const SIZE_EVENT: &str = "widget-size";
/// Сброс раскладки поставил окну новую основу (`useWindowReset`): панели
/// остаются раскрытыми и заново меряют высоту от неё.
const RESET_EVENT: &str = "widget-reset";

/// Событие одному окну, а не всем.
///
/// ВАЖНО: **`emit_to`, а не `emit`.** `emit` у окна в Tauri 2 — рассылка всем
/// окнам, и до слушателя с адресом своего окна (`getCurrentWebviewWindow().listen`)
/// она доходит всё равно: без фильтра Tauri отдаёт событие каждому слушателю.
/// Смена размера у одного виджета сворачивала раскрытые панели у всех остальных.
fn tell<S: serde::Serialize + Clone>(window: &WebviewWindow, event: &str, payload: S) {
    let _ = window.emit_to(window.label(), event, payload);
}

/// Ставит размер по номеру всему, что открыто под этой меткой.
pub fn set_size(app: &AppHandle, id: &str, index: usize) {
    let Some(widget) = widgets::find(id) else {
        return;
    };
    let Some(window) = app.get_webview_window(id) else {
        return;
    };

    let settings = app.state::<SettingsState>().get();
    let view = settings.widget(widget).view;
    let scale = settings.scale_of(widget);
    let was_side_panel = id == "sound" && sound_panel::restore_base(&window);
    apply_preset(&window, widget, index, view, scale);
    // Смена вида тоже может убрать раскрытую панель. Возвращаем её подъём,
    // насколько новый размер помещается снизу, тем же расчётом, что у роста.
    if id == "sound" {
        if let (Ok(place), Ok(size)) = (window.outer_position(), window.inner_size()) {
            settle(&window, place, size.height);
        }
    }
    if was_side_panel || (id == "sound" && view == 2) {
        sound_panel::clamp_base(&window);
    }
    if id == "sound" && view == 2 {
        // Теперь место ведёт боковая панель; долг прежнего роста вниз не
        // должен примениться второй раз при следующем переходе между видами.
        LIFTED.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(id);
    }

    // Размер поставлен на нынешнем масштабе — пересчёту знать об этом
    set_layout_scale(widget.id, scale);

    // Окну надо знать, что высоту ему поставили числом: раскрытая панель
    // сворачивается, а хук роста забывает свой долг. Иначе следующее закрытие
    // панели отнимет высоту у уже нового размера — живьём это выглядело как
    // застрявшая плашка микшера
    tell(&window, SIZE_EVENT, index);

    save_places(app);
}

/// Растит или сжимает окно на столько логических точек — под раскрытую панель.
///
/// ВАЖНО: **окно растёт вниз, а виджеты чаще всего прижаты к нижнему краю
/// экрана.** У такого окна раскрытая панель уезжала под панель задач и за край
/// монитора: список плейлистов и список устройств было не увидеть вовсе.
/// Поэтому вместе с высотой правим и место.
///
/// Считает это Rust, а не фронт: рабочая область — это край без панели задач, и
/// у монитора её знает только система.
pub fn grow(window: &WebviewWindow, by: f64) {
    let (Ok(factor), Ok(size), Ok(place)) = (
        window.scale_factor(),
        window.inner_size(),
        window.outer_position(),
    ) else {
        return;
    };

    let step = (by * factor).round() as i32;
    if step == 0 {
        return;
    }

    let height = (size.height as i32 + step).max(1) as u32;
    let _ = window.set_size(PhysicalSize::new(size.width, height));
    settle(window, place, height);
}

/// Ставит окно так, чтобы выросшее влезло в рабочую область, а сжавшееся
/// вернулось на своё место.
///
/// ВАЖНО: **это один расчёт на оба случая, а не «вырос — подняли, сжался —
/// опустили».** Панели в окне сменяют друг друга: с высокого микшера на список
/// из одной строки окно сжимается, но остаётся раскрытым. Возврат всего
/// поднятия разом уводил его вниз, и новая панель оказывалась за краем экрана —
/// ровно то, от чего поднятие и заводили.
///
/// Поэтому смотрим не на знак шага, а на место: не влезло снизу — поднимаем на
/// недостающее, освободилось снизу — опускаем, но не ниже того, откуда подняли.
fn settle(window: &WebviewWindow, place: PhysicalPosition<i32>, height: u32) {
    let Ok(Some(monitor)) = window.current_monitor() else {
        return;
    };
    let area = monitor.work_area();

    let top = area.position.y;
    let bottom = area.position.y + area.size.height as i32;

    let mut lifted = LIFTED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // Окно с тех пор двигали руками — долг больше не наш; забываем его сразу,
    // иначе он ожил бы, случись окну вернуться на прежнее место
    let raised = match lifted.get(window.label()) {
        Some(lift) if lift.y == place.y => lift.owed,
        Some(_) => {
            lifted.remove(window.label());
            0
        }
        None => 0,
    };

    let (shift, owed) = placement(top, bottom, place.y, height as i32, raised);
    if shift == 0 {
        return;
    }

    let settled = place.y + shift;
    let _ = window.set_position(PhysicalPosition::new(place.x, settled));

    if owed > 0 {
        lifted.insert(window.label().to_owned(), Lift { owed, y: settled });
    } else {
        lifted.remove(window.label());
    }
}

/// Куда подвинуть окно и сколько подъёма за ним останется.
///
/// Отдельной функцией — ради теста: живьём такое проверяется только мышью у
/// самого края экрана, а разойтись знаками тут проще простого.
fn placement(top: i32, bottom: i32, y: i32, height: i32, raised: i32) -> (i32, i32) {
    // Сколько не влезло снизу; отрицательное — столько там ещё свободно
    let over = y + height - bottom;

    let shift = if over > 0 {
        // Выше верхнего края не уходим: там окно так же не увидеть
        -over.min((y - top).max(0))
    } else {
        // Опускаем ровно на то, на сколько поднимали, и не больше запаса
        raised.min(-over).max(0)
    };

    (shift, raised - shift)
}

#[cfg(test)]
mod tests {
    use super::{placement, resizes};
    use crate::settings::{Settings, WidgetSettings};
    use crate::widgets;

    /// Своя блокировка запирает окно, но отпереть запертое общим рубильником
    /// не может. Проверяется на текстовом виджете: у остальных размер и так
    /// заперт готовыми размерами, и разницы было бы не видно.
    #[test]
    fn the_switch_outranks_the_widgets_own_lock() {
        let notes = widgets::find("notes").expect("заметки в реестре");
        assert!(notes.flexible, "заметки тянутся мышью — на них это и видно");

        let free = Settings::default();
        let mut own = WidgetSettings::default();
        assert!(resizes(notes, &free, own), "без запретов окно тянется");

        own.locked = true;
        assert!(!resizes(notes, &free, own), "свой замок запирает");

        let all = Settings {
            locked: true,
            ..Settings::default()
        };
        assert!(!resizes(notes, &all, own), "оба замка — тем более заперто");

        // Главное: снятый свой замок не отпирает запертое рубильником
        own.locked = false;
        assert!(!resizes(notes, &all, own), "свой замок не отменяет общего");
    }

    /// Нерастягиваемому виджету замок ничего не меняет: у него размер держат
    /// готовые размеры, и мышью он не тянулся никогда.
    #[test]
    fn a_preset_sized_widget_never_stretches() {
        let gpu = widgets::find("gpu").expect("видеокарта в реестре");
        assert!(!resizes(gpu, &Settings::default(), WidgetSettings::default()));
    }

    /// Рабочая область вертикального монитора: сверху ноль, снизу тысяча.
    const TOP: i32 = 0;
    const BOTTOM: i32 = 1000;

    /// Окно у нижнего края выросло — поднимаем ровно на недостающее.
    #[test]
    fn a_grown_window_moves_up_by_what_is_missing() {
        let (shift, owed) = placement(TOP, BOTTOM, 900, 250, 0);

        assert_eq!(shift, -150, "низ окна должен лечь на край области");
        assert_eq!(owed, 150, "столько мы ему и должны");
    }

    /// Панель закрыли — окно возвращается ровно туда, откуда его подняли.
    #[test]
    fn a_closed_panel_puts_the_window_back() {
        let (shift, owed) = placement(TOP, BOTTOM, 750, 100, 150);

        assert_eq!(shift, 150);
        assert_eq!(owed, 0, "долга не остаётся");
    }

    /// ВАЖНО: панели сменяют друг друга. С высокого микшера на список из одной
    /// строки окно сжимается, но раскрытым быть не перестаёт — и возврат всего
    /// подъёма разом увёл бы новую панель за нижний край.
    #[test]
    fn switching_panels_returns_only_the_freed_room() {
        // Было: поднято на 150, высота 250, низ ровно на краю.
        // Стало: высота 200 — освободилось пятьдесят, их и возвращаем
        let (shift, owed) = placement(TOP, BOTTOM, 750, 200, 150);

        assert_eq!(shift, 50);
        assert_eq!(owed, 100, "остальное окно должно держать за собой");
    }

    /// Окно посреди экрана не трогаем вовсе: двигать его незачем.
    #[test]
    fn a_window_with_room_below_stays_where_it_is() {
        assert_eq!(placement(TOP, BOTTOM, 100, 200, 0), (0, 0));
    }

    /// Не влезает даже подъёмом до верха — поднимаем до упора и не выше.
    #[test]
    fn a_window_taller_than_the_screen_stops_at_the_top() {
        let (shift, owed) = placement(TOP, BOTTOM, 200, 1500, 0);

        assert_eq!(shift, -200, "выше верхнего края уходить некуда");
        assert_eq!(owed, 200);
    }

    /// Тот же расчёт на мониторе с отрицательными координатами: второй экран
    /// слева от основного — ровно этот случай.
    #[test]
    fn negative_coordinates_work_the_same() {
        let (shift, owed) = placement(-400, 1520, 1400, 250, 0);

        assert_eq!(shift, -130);
        assert_eq!(owed, 130);
    }
}

/// Ставит работу с окнами в очередь главного потока.
///
/// ВАЖНО: **из синхронной команды это не очередь.** Синхронная команда Tauri
/// сама выполняется в главном потоке, прямо в обработчике IPC, и
/// `run_on_main_thread` оттуда выполняет работу на месте. Стилям и видимости
/// это не мешает, а сборке окна — мешает: она ждёт главный поток, то есть сама
/// себя. Так и зависало «выключил — включил видеокарту». Окна на ходу собирает
/// только `modules::set`, из пула, а сюда носят их доводку и разборку.
///
/// Ответа не ждём: команда возвращается сразу.
pub fn on_main<F>(app: &AppHandle, job: F)
where
    F: FnOnce(AppHandle) + Send + 'static,
{
    let handle = app.clone();
    if let Err(error) = app.run_on_main_thread(move || job(handle)) {
        eprintln!("окна не дождались главного потока: {error}");
    }
}

/// Собирает окно целиком — при старте, из главного потока до событийного цикла.
fn build_window(app: &AppHandle, widget: &'static Widget, visible: bool) -> Result<(), String> {
    let window = create(app, widget)?;
    finish(app, &window, widget, visible);
    Ok(())
}

/// Ключи движка для каждого окна приложения — виджетов, настроек, напоминания.
///
/// `--process-per-site` сводит процессы отрисовки всех окон в один: страницы у
/// нас одного сайта, а без ключа WebView2 держал свой процесс на каждое окно —
/// 30–85 МБ на виджет, около гигабайта на двадцать окон (замер 11 октября).
/// Цена — один поток JS на всех: упавший процесс гасит все окна разом.
///
/// ВАЖНО: **заданные ключи заменяют умолчания wry, а не дополняют их**, поэтому
/// его `--disable-features` повторён здесь. И ключи обязаны быть одинаковыми у
/// всех окон: окружение WebView2 одно на папку данных, и окно с другими ключами
/// не соберётся вовсе.
pub const BROWSER_ARGS: &str =
    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --process-per-site";

/// Первая половина сборки: сам webview, спрятанным.
///
/// ВАЖНО: **на ходу — только из потока, который не держит главный.** Сборка
/// ждёт главный поток, а синхронная команда Tauri на нём и выполняется, прямо в
/// обработчике IPC: `build()` оттуда ждёт сам себя, и вместе с ним встаёт
/// приём IPC во всём приложении. Так и зависало «выключил — включил
/// видеокарту» — из синхронной `set_widget_option`. Документация Tauri
/// говорит о том же (wry#583): окна на ходу собирают из async-команд.
pub fn create(app: &AppHandle, widget: &'static Widget) -> Result<WebviewWindow, String> {
    let settings = app.state::<SettingsState>().get();
    let own = settings.widget(widget);
    let scale = settings.scale_of(widget);
    let (width, height) = widget.design_size(settings.design, own.view, own.size, own.layout);
    let size = LogicalSize::new(width * scale, height.unwrap_or_else(|| widget.full().height.unwrap()) * scale);
    let floor = widget.min_size(scale);
    WebviewWindowBuilder::new(app, widget.id, WebviewUrl::App(widget.url().into()))
        .title(widget.title())
        .additional_browser_args(BROWSER_ARGS)
        .inner_size(size.width, size.height)
        // Пол, а не размер по содержимому: узкую карточку выбирают сами, и
        // сохранённый размер бывает вдвое меньше реестрового
        .min_inner_size(floor.width, floor.height)
        .decorations(widget.chrome)
        .transparent(!widget.chrome)
        .shadow(widget.chrome)
        .skip_taskbar(!widget.chrome)
        // Растягивается только текстовое, и запирают его оба рубильника —
        // общий и свой. У остальных размер задают пороги раскладки:
        // подобранная под них ширина и есть та сетка, ради которой всё
        // затевалось
        .resizable(resizes(widget, &settings, own))
        // Прячем до восстановления позиции: иначе окно мигнёт посреди экрана
        .visible(false)
        .build()
        .map_err(|error| format!("окно «{}» не создалось: {error}", widget.id))
}

/// Вторая половина сборки: рамка, место, размер, флаги, показ.
///
/// ВАЖНО: **только из главного потока.** Подклассы `native_frame` и `snap`
/// ставит `SetWindowSubclass`, а он вешается лишь на окно своего потока — из
/// чужого откажет, и виджет останется с заголовком и без прилипания.
pub fn finish(app: &AppHandle, window: &WebviewWindow, widget: &'static Widget, visible: bool) {
    let settings = app.state::<SettingsState>().get();
    let own = settings.widget(widget);
    let scale = settings.scale_of(widget);

    // Окно встаёт под свой масштаб — с него и отсчёт для пересчёта
    set_layout_scale(widget.id, scale);

    // ВАЖНО: **первым, пока окно спрятано, а не последним.** Защита правит
    // каждую запись стиля, а не снимает нарисованную рамку задним числом:
    // поставленная тут, она ловит и восстановление места, и флаги из настроек,
    // и первый показ
    if !widget.chrome {
        keep_frameless(window);
    }

    if widget.id == "converter" {
        // Прежние фиксированные min/max больше не ограничивают ручной resize.
        if let Err(error) = window.set_max_size(None::<LogicalSize<f64>>) {
            eprintln!("converter: снятие верхнего предела размера: {error}");
        }
        let _ = window.set_min_size(Some(widget.min_size(scale)));
    }
    let _ = window.restore_state(state_flags());
    sound_panel::restore_anchor(&window);
    if widget.id == "sound" {
        // Модуль мог разобрать прежнее окно с раскрытой панелью. Новое
        // начинает с основы и не наследует долг её старого роста вниз.
        LIFTED.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(widget.id);
    }

    // ВАЖНО: **после восстановления, а не вместо него.** Плагин окон вернёт
    // размер, накрученный мышью до того, как виджет перевели на готовые
    // размеры, — у нерастягиваемой карточки его надо перебить. У текстовых
    // виджетов правило обратное: там сохранённый размер и есть выбор человека.
    if !widget.flexible {
        apply_preset(window, widget, own.size, own.view, scale);
    }
    if widget.id == "sound" { sound_panel::clamp_base(&window); }

    apply(window, own, resizes(widget, &settings, own));

    if let Err(error) = window_effects::apply_to(window) {
        // Вид испортится, но виджет останется рабочим
        eprintln!("подложка у «{}» не снялась: {error}", widget.id);
    }

    if !widget.chrome {
        let _ = window.set_decorations(false);
        let _ = window.set_skip_taskbar(true);
    }

    watch_closing(app, window, widget.id);
    snap::attach(app, window, widget.id, widget.chrome);

    // Рубильник «спрятать все» поднят — окно собрано, но на экран не выходит
    if visible {
        let _ = window.show();
    }
}

/// Держит окно без системной рамки всю его жизнь — как и почему, написано в
/// `native_frame.rs`. Зовётся один раз, сразу после сборки окна.
///
/// Прежде рамку снимали вслед за каждым обращением к окну, и правило «снятие
/// последним» стояло в полудюжине мест. Подкласс его отменил: заголовок больше
/// не попадает в стиль вовсе, так что снимать нечего.
#[cfg(windows)]
pub fn keep_frameless(window: &WebviewWindow) {
    // Окно при отказе останется рабочим, но с заголовком под полупрозрачной
    // карточкой — молча это проходить не должно
    if let Err(error) = crate::native_frame::install(window) {
        eprintln!("окно «{}» осталось с системной рамкой: {error}", window.label());
    }
}

#[cfg(not(windows))]
pub fn keep_frameless(_window: &WebviewWindow) {}

/// Прячет окна всех виджетов, не трогая список включённых.
///
/// Рубильник «спрятать все». Окна именно прячутся, а не выгружаются: рубильник
/// — это «убрать с глаз на минуту», а не «выключить модули». Выгрузка — дело
/// галочки «Вкл.».
///
/// Опрос Спотифая от этого не страдает: он смотрит не на наличие окна, а на
/// его видимость, и на спрятанном молчит ровно так же.
pub fn hide_all(app: &AppHandle) {
    // Сохраняем разом: спрятанное окно ещё помнит место, но лучше записать
    save_places(app);

    for widget in widgets::shown() {
        if let Some(window) = app.get_webview_window(widget.id) {
            let _ = window.hide();
        }
    }
}

/// Заголовки окон виджетов — на нынешнем языке. Зовётся после его смены.
///
/// Заголовок не виден на экране — рамки у виджетов нет, — но его читают
/// Alt+Tab, экранный диктор и `scripts/shot.ps1`, так что отставать от языка
/// ему незачем.
pub fn retitle(app: &AppHandle) {
    for widget in widgets::shown() {
        if let Some(window) = app.get_webview_window(widget.id) {
            let _ = window.set_title(&widget.title());
        }
    }
}

/// Возвращает на экран то, что спрятал рубильник.
///
/// Показывает только собранные окна: окно есть ровно у включённых модулей, а
/// включённый при поднятом рубильнике модуль собрался спрятанным и ждёт здесь.
///
/// Фокус при этом не трогаем: восемь окон подряд перетягивали бы его друг у
/// друга, и последнее отобрало бы его у окна настроек, где рубильник и
/// опустили.
pub fn show_all(app: &AppHandle) {
    for widget in widgets::shown() {
        if let Some(window) = app.get_webview_window(widget.id) {
            let _ = window.show();
        }
    }
}

/// Запирает или отпускает размер окон.
///
/// ВАЖНО: размер запирается снятием `WS_THICKFRAME` (`set_resizable(false)`),
/// а не ответом `HTCLIENT` на `WM_NCHITTEST`. Перехват в оконной процедуре
/// проверку живьём не прошёл — окна продолжали тянуться за края, — а стиль
/// работает наверняка: без него `DefWindowProc` не пускает `SC_SIZE`.
///
/// Перетаскивание запирает фронт: `useWindowDrag` не начинает его ни при
/// поднятом рубильнике, ни при своём флаге виджета.
///
/// Числом сюда ничего не передаётся: запертость у каждого окна своя, и
/// собрать её можно только из настроек целиком.
pub fn apply_lock(app: &AppHandle) {
    let settings = app.state::<SettingsState>().get();

    for widget in widgets::shown() {
        let Some(window) = app.get_webview_window(widget.id) else {
            continue;
        };

        // ВАЖНО: снятие рубильника не возвращает мышь тому, кто её и не имел,
        // — иначе блокировка положения тихо отменяла бы готовые размеры
        let _ = window.set_resizable(resizes(widget, &settings, settings.widget(widget)));
    }
}

fn reset_size(widget: &Widget, settings: &settings::Settings) -> LogicalSize<f64> {
    let own = settings.widget(widget);
    let scale = settings.scale_of(widget);
    let (width, height) = widget.design_size(settings.design, own.view, own.size, own.layout);
    // У узких текстовых пресетов высота ручная. Сброс берёт начальную высоту
    // этого же вида и компоновки, сохраняя выбранную узкую ширину.
    let height = height.or_else(|| widget.design_size(settings.design, own.view, 0, own.layout).1)
        .unwrap_or_else(|| widget.full().height.expect("у полного размера высота своя"));
    LogicalSize::new(hardware_width(settings, widget.id).unwrap_or(width * scale), height * scale)
}

// Открытая панель повторно нарастит основу после widget-reset. Резервируем
// её прежнюю занятую высоту в сетке, чтобы не поставить туда следующее окно.
// Ручные окна сбрасывают геометрию, а звук закрывает панель — резерва нет.
fn reset_spacing_height(widget: &Widget, base: f64, previous: Option<f64>) -> f64 {
    if widget.flexible || widget.id == "sound" { return base; }
    previous.filter(|height| height.is_finite()).unwrap_or(base).max(base)
}

/// Расставляет открытые окна заново: размер из реестра, место — сеткой по
/// рабочей области основного монитора.
///
/// Спасение на случай, когда окно уехало за край экрана: мышью его оттуда уже
/// не достать, а сохранённая позиция переживает перезапуск. Выключенные
/// модули не трогаем — окна у них нет, а их запись перепишется тем же
/// сбросом, когда модуль включат.
pub fn reset_places(app: &AppHandle) {
    /// Зазор между окнами и до края рабочей области.
    const GAP: i32 = 12;

    // Только то, что видно: спрятанные рубильником окна занимать место в
    // сетке не должны
    let open: Vec<(&'static Widget, WebviewWindow)> = widgets::shown()
        .filter_map(|widget| {
            app.get_webview_window(widget.id)
                .filter(|window| window.is_visible().unwrap_or(false))
                .map(|window| (widget, window))
        })
        .collect();

    let Some((_, sample)) = open.first() else {
        return;
    };

    let Ok(Some(monitor)) = sample.primary_monitor() else {
        return;
    };
    let area = monitor.work_area();

    // Размеры в реестре логические, а места на экране считаются в физических.
    // Масштаб берём у монитора, на который расставляем, а не у окна: оно могло
    // стоять на соседнем и с другим масштабом, и сетка вышла бы в чужих точках
    let factor = monitor.scale_factor();

    let left = area.position.x + GAP;
    let top = area.position.y + GAP;
    let bottom = area.position.y + area.size.height as i32 - GAP;

    let mut x = left;
    let mut y = top;
    // Ширина самого широкого окна в нынешней колонке
    let mut column = 0;

    // Размеры возвращаем реестровые, но на нынешнем масштабе: при 130 % сброс
    // ставил бы окна в сотню процентов и резал бы им содержимое. У кого масштаб
    // свой — по своему: сброс расставляет окна, а не отнимает настройки
    let settings = app.state::<SettingsState>().get();

    for (widget, window) in &open {
        let scale = settings.scale_of(widget);
        let size = reset_size(widget, &settings);
        if widget.id == "sound" { sound_panel::forget(); }
        // Новый якорь не должен наследовать подъём старой раскрытой панели.
        LIFTED.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(widget.id);
        let previous_height = match (window.scale_factor(), window.inner_size()) {
            (Ok(previous_factor), Ok(previous)) => Some(previous.to_logical::<f64>(previous_factor).height),
            _ => None,
        };
        let width = (size.width * factor).round() as i32;
        let height = (reset_spacing_height(widget, size.height, previous_height) * factor).round() as i32;

        // Не влезло по высоте — начинаем новую колонку. Условие про `y > top`
        // спасает окно выше монитора: иначе оно уезжало бы вправо без конца
        if y + height > bottom && y > top {
            x += column + GAP;
            y = top;
            column = 0;
        }

        let _ = window.set_size(size);
        let _ = window.set_min_size(Some(widget.min_size(scale)));
        let _ = window.set_position(PhysicalPosition::new(x, y));
        set_layout_scale(widget.id, scale);
        if widget.id == "sound" {
            // Прежний договор звука: сброс закрывает микшер и боковую панель.
            tell(window, SIZE_EVENT, settings.widget(widget).size);
        } else {
            // Сброс основы не меняет состояние карточки. Открытые панели
            // забывают прежний долг и повторно измеряются от новой основы.
            tell(window, RESET_EVENT, ());
        }

        y += height + GAP;
        column = column.max(width);
    }

    save_places(app);
}

/// Просьба фронта вырасти под раскрытую панель или сжаться обратно.
///
/// Размерами и местом окон ведает главный поток, поэтому команда только ставит
/// работу в его очередь и возвращается.
#[tauri::command]
pub fn grow_window(window: WebviewWindow, by: f64, preset: Option<PresetPanel>) {
    let app = window.app_handle().clone();
    let label = window.label().to_owned();

    on_main(&app, move |app| {
        if let Some(window) = app.get_webview_window(&label) {
            if let Some(request) = preset {
                let Some(widget) = widgets::find(&label) else { return; };
                let settings = app.state::<SettingsState>().get();
                let Some(height) = panel_height(widget, settings.widget(widget), &request, settings.scale_of(widget), settings.design) else {
                    return;
                };
                if let (Ok(factor), Ok(size)) = (window.scale_factor(), window.inner_size()) {
                    grow(&window, height - size.to_logical::<f64>(factor).height);
                }
            } else {
                grow(&window, by);
            }
        }
    });
}

/// Смена дизайна восстанавливает основу до appearance. Панели не закрываются:
/// их React-состояние сохранено, нужную высоту они измерят заново.
pub fn sync_design_geometry(app: &AppHandle) {
    let settings = app.state::<SettingsState>().get();
    for id in ["disks", "network", "speedtest", "weather", "media", "spotify", "password", "downloader"] {
        let Some(widget) = widgets::find(id) else { continue; };
        let Some(window) = app.get_webview_window(id) else { continue; };
        let own = settings.widget(widget);
        let scale = settings.scale_of(widget);
        apply_preset(&window, widget, own.size, own.view, scale);
        set_layout_scale(id, scale);
    }
}

/// Замер снят другим дизайном, чем тот, которым окно рисует сейчас. Без метки
/// (старый фронт) замер принимается, как прежде.
fn fit_is_stale(settings: &settings::Settings, label: &str, sent: Option<settings::Design>) -> bool {
    sent.is_some_and(|sent| sent != window_effects::effective_design(settings, label))
}

/// Ширина, которую подгонка по содержимому держит у нерастягиваемой карточки:
/// та же, что ставит готовый размер (`design_size`), на её масштабе.
/// Индекс размера не меняется.
fn hardware_width(settings: &settings::Settings, label: &str) -> Option<f64> {
    if !matches!(label, "cpu" | "gpu" | "disks" | "network" | "speedtest" | "weather" | "media" | "downloader") { return None; }
    let widget = widgets::find(label)?;
    let own = settings.widget(widget);
    let width = widget.design_size(settings.design, own.view, own.size, own.layout).0;
    Some(width * settings.scale_of(widget))
}

/// Просьба фронта встать ровно такой высоты — в логических точках.
///
/// Нужна карточке, чья высота следует из содержимого, а не из реестра: у
/// накопителей строк столько, сколько дисков показано. Число абсолютное, а не
/// прирост, и шаг считается здесь, в главном потоке, от высоты, которая уже
/// стоит: прирост, посчитанный во фронте, ошибся бы на вызове внахлёст — оба
/// прочли бы одну и ту же высоту (см. очередь в `grow.ts`). Дальше та же
/// дорога, что у панелей: окно у нижнего края поднимается и не уходит под
/// панель задач.
///
/// ВАЖНО: **замер помечен дизайном, которым его сняли.** Смена дизайна ставит
/// окну новую основу, и от этого `ResizeObserver` прежней карточки успевает
/// прислать ещё один замер — уже не той раскладки. Принятый, он поставил бы
/// карточке Claude высоту GPT, а своей подгонки у неё (сеть, погода, медиа)
/// нет, и исправить было бы нечем. Сверка идёт здесь, в главном потоке:
/// выбор дизайна к этому моменту уже записан.
#[tauri::command]
pub fn fit_window_height(window: WebviewWindow, height: f64, design: Option<settings::Design>) {
    // Конвертер сохраняет выбранную руками геометрию в обоих дизайнах.
    if window.label() == "converter" { return; }
    let app = window.app_handle().clone();
    let label = window.label().to_owned();

    on_main(&app, move |app| {
        let Some(window) = app.get_webview_window(&label) else {
            return;
        };
        if fit_is_stale(&app.state::<SettingsState>().get(), &label, design) {
            return;
        }
        let (Ok(size), Ok(factor)) = (window.inner_size(), window.scale_factor()) else {
            return;
        };

        // Ширина подгоняется вместе с высотой, в том числе если высота та же.
        // Чужие виджеты и выбранный индекс размера не затрагиваются.
        if let Some(width) = hardware_width(&app.state::<SettingsState>().get(), &label) {
            let physical_width = (width * factor).round() as u32;
            if size.width.abs_diff(physical_width) >= 2 {
                let _ = window.set_size(PhysicalSize::new(physical_width, size.height));
            }
        }
        let now = f64::from(size.height) / factor;
        grow(&window, height - now);
    });
}

/// Измеренная боковая панель вертикального звука. Ответ приходит после
/// размещения на главном потоке: фронт получает выбранную системой сторону.
#[tauri::command]
pub async fn fit_sound_panel(window: WebviewWindow, request: sound_panel::Request) -> Result<sound_panel::Reply, String> {
    sound_panel::fit(window, request).await
}

/// Панель нерастягиваемого виджета: измеренная прибавка к его готовой основе.
/// Номер вида и размера отбрасывает запоздавший запрос прежней раскладки.
#[derive(serde::Deserialize, Default)]
pub struct PresetPanel {
    view: usize,
    size: usize,
    extra: f64,
    #[serde(default)]
    design: Option<settings::Design>,
    #[serde(default)]
    layout: Option<usize>,
    #[serde(default)]
    scale: Option<f64>,
}

fn panel_height(widget: &Widget, own: WidgetSettings, request: &PresetPanel, scale: f64, design: settings::Design) -> Option<f64> {
    if widget.flexible || request.view != own.view || request.size != own.size
        || request.design.is_some_and(|sent| sent != design)
        || request.layout.is_some_and(|sent| sent != own.layout)
        || request.scale.is_some_and(|sent| !sent.is_finite() || (sent - scale).abs() > 0.0001)
        || (widget.id == "sound" && own.view == 2)
        || !request.extra.is_finite() {
        return None;
    }
    // Отрицательная прибавка бывает только у пульта: вертикальный GPT встаёт
    // высотой по содержимому, когда у устройства нет громкости
    // (`referenceSlack` во фронте). Ужать окно больше чем вдвое она не может —
    // такое число пришло бы только по ошибке
    let base = widget.design_size(design, own.view, own.size, own.layout).1? * scale;
    let floor = if widget.id == "spotify" { -base / 2.0 } else { 0.0 };
    (request.extra >= floor).then_some(base + request.extra)
}

/// Тянется ли окно мышью за края.
///
/// Три условия, и все три обязательны: растягивается только текстовый виджет,
/// общий рубильник запирает всех разом, свой флаг — этого одного.
///
/// ВАЖНО: **общий рубильник главнее своего.** Свой запирает, но отпереть
/// запертое общим не может: иначе рубильник, ради которого лезут в трей,
/// обходился бы галочкой в списке — и «заблокировать положение» оставляло бы
/// половину окон на свободе.
fn resizes(widget: &Widget, settings: &settings::Settings, own: WidgetSettings) -> bool {
    widget.flexible && !settings.locked && !own.locked
}

pub fn apply(window: &WebviewWindow, settings: WidgetSettings, resizable: bool) {
    let _ = window.set_always_on_top(settings.always_on_top);
    let _ = window.set_ignore_cursor_events(settings.click_through);
    let _ = window.set_resizable(resizable);
}

/// Применяет настройки к уже открытому окну, если оно открыто.
pub fn apply_by_id(app: &AppHandle, id: &str, own: WidgetSettings) {
    let Some(window) = app.get_webview_window(id) else {
        return;
    };
    let Some(widget) = widgets::find(id) else {
        return;
    };
    let settings = app.state::<SettingsState>().get();

    apply(&window, own, resizes(widget, &settings, own));
}

/// Крестик на окне с рамкой (отладка, витрина) и Alt+F4 означают «выключить
/// модуль», а не «спрятать до перезапуска»: иначе галочка в окне настроек
/// осталась бы врать.
///
/// Закрытие отменяем и отдаём модулю: он запишет настройку и разберёт окно
/// той же дорогой, что и галочка. Сам обработчик живёт в главном потоке, а
/// разборка ждёт его же — поэтому через пул (`modules::set_later`).
fn watch_closing(app: &AppHandle, window: &WebviewWindow, id: &'static str) {
    let handle = app.clone();

    window.on_window_event(move |event| {
        let WindowEvent::CloseRequested { api, .. } = event else {
            return;
        };
        if handle.state::<Shutdown>().started() {
            return;
        }

        api.prevent_close();

        if let Some(widget) = widgets::find(id) {
            crate::modules::set_later(&handle, widget, false);
        }
    });
}

/// Сохраняет положение всех окон и завершает приложение.
pub fn quit(app: &AppHandle) {
    app.state::<Shutdown>().begin();
    save_places(app);
    app.exit(0);
}

#[cfg(test)]
mod panel_tests {
    use super::*;

    #[test]
    fn sound_panel_height_always_starts_at_the_current_base() {
        let sound = widgets::find("sound").expect("звук в реестре");
        for view in 0..2 {
            for size in 0..sound.presets_for(view).len() {
                let own = WidgetSettings { view, size, ..WidgetSettings::default() };
                let base = sound.preset_size(view, size).1.expect("высота основы");
                for scale in [0.9, 1.0, 1.3] {
                    // Большая панель, меньшая и закрытие не зависят от
                    // того, какой была предыдущая прибавка.
                    for extra in [420.0, 65.0, 0.0] {
                        let request = PresetPanel { view, size, extra, ..PresetPanel::default() };
                        assert_eq!(panel_height(sound, own, &request, scale, settings::Design::App), Some(base * scale + extra));
                    }
                }
            }
        }
    }

    #[test]
    fn stale_or_invalid_panel_sizes_are_ignored() {
        let sound = widgets::find("sound").expect("звук в реестре");
        let own = WidgetSettings { view: 0, size: 0, ..WidgetSettings::default() };
        for request in [
            PresetPanel { view: 1, size: 0, extra: 80.0, ..PresetPanel::default() },
            PresetPanel { view: 0, size: 1, extra: 80.0, ..PresetPanel::default() },
            PresetPanel { view: 0, size: 0, extra: -1.0, ..PresetPanel::default() },
            PresetPanel { view: 0, size: 0, extra: f64::NAN, ..PresetPanel::default() },
        ] {
            assert_eq!(panel_height(sound, own, &request, 1.0, settings::Design::App), None);
        }
        let own = WidgetSettings { view: 2, size: 0, ..WidgetSettings::default() };
        assert_eq!(panel_height(sound, own, &PresetPanel { view: 2, size: 0, extra: 80.0, ..PresetPanel::default() }, 1.0, settings::Design::App), None,
            "вертикальный вид размещает только своя боковая панель");
    }

    #[test]
    fn playlist_growth_uses_design_base_and_rejects_old_design_layout_or_scale() {
        let widget = widgets::find("spotify").unwrap();
        let own = WidgetSettings { view: 0, size: 0, layout: 1, ..WidgetSettings::default() };
        for design in [settings::Design::App, settings::Design::Reference] {
            for scale in [0.9, 1.0, 1.3] {
                for extra in [188.0 * scale, 0.0] {
                    let request = PresetPanel { view: 0, size: 0, extra, design: Some(design), layout: Some(1), scale: Some(scale) };
                    let base = widget.design_size(design, 0, 0, 1).1.unwrap();
                    assert_eq!(panel_height(widget, own, &request, scale, design), Some(base * scale + extra));
                }
            }
        }
        let request = PresetPanel { design: Some(settings::Design::Reference), layout: Some(1), scale: Some(1.0), ..PresetPanel::default() };
        assert_eq!(panel_height(widget, own, &request, 1.0, settings::Design::App), None);
        assert_eq!(panel_height(widget, WidgetSettings { layout: 0, ..own }, &request, 1.0, settings::Design::Reference), None);
        assert_eq!(panel_height(widget, own, &request, 1.3, settings::Design::Reference), None);
    }

    #[test]
    fn only_the_player_may_shrink_below_its_base() {
        let widget = widgets::find("spotify").unwrap();
        let own = WidgetSettings { view: 1, size: 0, ..WidgetSettings::default() };
        let base = widget.design_size(settings::Design::Reference, 1, 0, 0).1.unwrap();
        let shrink = PresetPanel { view: 1, size: 0, extra: -60.0, ..PresetPanel::default() };
        assert_eq!(panel_height(widget, own, &shrink, 1.0, settings::Design::Reference), Some(base - 60.0));
        let absurd = PresetPanel { view: 1, size: 0, extra: -base, ..PresetPanel::default() };
        assert_eq!(panel_height(widget, own, &absurd, 1.0, settings::Design::Reference), None,
            "ужать окно больше чем вдвое прибавка не может");
    }

}

#[cfg(test)]
mod reset_size_tests {
    use super::*;

    #[test]
    fn reset_keeps_selected_format_size_layout_design_and_scale() {
        let spotify = widgets::find("spotify").unwrap();
        for scale in [0.9, 1.0, 1.3] {
            for (design, view, size, width, height) in [
                (settings::Design::Reference, 0, 0, 464.0, 152.0),
                (settings::Design::Reference, 0, 3, 300.0, 66.0),
                (settings::Design::Reference, 1, 1, 240.0, 454.0),
                (settings::Design::Reference, 2, 2, 240.0, 240.0),
                (settings::Design::App, 0, 0, 520.0, 184.0),
            ] {
                let mut settings = settings::Settings { design, scale, ..settings::Settings::default() };
                settings.set_widget("spotify", WidgetSettings { view, size, ..WidgetSettings::default() });
                let actual = reset_size(spotify, &settings);
                assert!((actual.width - width * scale).abs() < 0.001);
                assert!((actual.height - height * scale).abs() < 0.001);
                assert_eq!(settings.widget(spotify).view, view);
                assert_eq!(settings.widget(spotify).size, size);
            }
            let totp = widgets::find("totp").unwrap();
            let mut settings = settings::Settings { scale, ..settings::Settings::default() };
            settings.set_widget("totp", WidgetSettings { size: 2, layout: 1, ..WidgetSettings::default() });
            let actual = reset_size(totp, &settings);
            assert!((actual.width - 220.0 * scale).abs() < 0.001);
            assert!((actual.height - 210.0 * scale).abs() < 0.001, "узкая B получает высоту B, а не A");
            for label in ["cpu", "gpu"] {
                let widget = widgets::find(label).unwrap();
                settings.set_widget(label, WidgetSettings { size: 1, view: 1, scale: Some(1.3), ..WidgetSettings::default() });
                assert!((reset_size(widget, &settings).width - 180.0 * 1.3).abs() < 0.001,
                    "сброс использует плотную ширину и собственный масштаб");
            }
        }
    }

    #[test]
    fn reset_grid_reserves_open_panels_but_not_old_manual_or_sound_size() {
        let spotify = widgets::find("spotify").unwrap();
        for scale in [0.9, 1.0, 1.3] {
            let settings = settings::Settings { scale, ..settings::Settings::default() };
            let base = reset_size(spotify, &settings).height;
            assert!((reset_spacing_height(spotify, base, Some(340.0 * scale)) - 340.0 * scale).abs() < 0.001,
                "плейлисты не перекрывают следующее окно после повторного роста");
            assert_eq!(reset_spacing_height(spotify, base, Some(60.0 * scale)), base);
            assert_eq!(reset_spacing_height(spotify, base, None), base);
            for label in ["notes", "sound"] {
                let widget = widgets::find(label).unwrap();
                let base = reset_size(widget, &settings).height;
                assert_eq!(reset_spacing_height(widget, base, Some(900.0 * scale)), base,
                    "сброшенное ручное окно и закрытый микшер не оставляют прежнюю дыру");
            }
        }
    }

    #[test]
    fn reset_has_finite_base_for_every_widget_in_both_designs() {
        for design in [settings::Design::App, settings::Design::Reference] {
            let settings = settings::Settings { design, ..settings::Settings::default() };
            for widget in widgets::shown() {
                let size = reset_size(widget, &settings);
                assert!(size.width.is_finite() && size.width > 0.0, "{}", widget.id);
                assert!(size.height.is_finite() && size.height > 0.0, "{}", widget.id);
            }
        }
    }
}

#[cfg(test)]
mod hardware_width_tests {
    use super::*;

    #[test]
    fn hardware_width_keeps_size_and_scale_and_restores_claude() {
        for label in ["cpu", "gpu"] {
            let widget = widgets::find(label).unwrap();
            for scale in [0.9, 1.0, 1.3] {
                for size in 0..3 {
                    let mut settings = settings::Settings { scale, ..settings::Settings::default() };
                    settings.set_widget(label, WidgetSettings { size, view: 1, ..WidgetSettings::default() });
                    let expected = if size == 0 { 244.0 } else { 180.0 };
                    assert!((hardware_width(&settings, label).unwrap() - expected * scale).abs() < 0.001);
                    settings.design = settings::Design::App;
                    assert!((hardware_width(&settings, label).unwrap() - widget.preset(1, size).width * scale).abs() < 0.001);
                    assert_eq!(settings.widget(widget).size, size);
                    assert_eq!(settings.widget(widget).view, 1);
                }
            }
        }
        assert!(hardware_width(&settings::Settings::default(), "drives").is_none());
    }

    /// Готовый размер и подгонка ставят окну одну и ту же ширину: прежде GPT
    /// вставал по реестровым 264 и сужался до 244 уже после — скачком.
    #[test]
    fn a_preset_and_the_fit_agree_on_the_pair_width() {
        for label in ["cpu", "gpu"] {
            let widget = widgets::find(label).unwrap();
            for design in [settings::Design::Reference, settings::Design::App] {
                for view in 0..2 {
                    for size in 0..3 {
                        let mut settings = settings::Settings { design, ..settings::Settings::default() };
                        settings.set_widget(label, WidgetSettings { view, size, ..WidgetSettings::default() });
                        let preset = widget.design_size(design, view, size, 0).0;
                        assert!((hardware_width(&settings, label).unwrap() - preset).abs() < 0.001, "{label} {design:?} {view}/{size}");
                        assert_eq!(widget.design_size(design, view, size, 0).1, widget.preset_size(view, size).1,
                            "высоту по-прежнему задаёт реестр, а доводит замер");
                    }
                }
            }
        }
    }

    /// Замер, снятый прежним дизайном, после смены не принимается.
    #[test]
    fn a_fit_measured_by_the_other_design_is_dropped() {
        for chosen in [settings::Design::Reference, settings::Design::App] {
            let settings = settings::Settings { design: chosen, ..settings::Settings::default() };
            let other = if chosen == settings::Design::App { settings::Design::Reference } else { settings::Design::App };
            for label in ["network", "weather", "media", "cpu", "disks"] {
                assert!(!fit_is_stale(&settings, label, Some(chosen)), "{label}: свой замер принимается");
                assert!(fit_is_stale(&settings, label, Some(other)), "{label}: чужой замер отбрасывается");
                assert!(!fit_is_stale(&settings, label, None), "{label}: замер без метки — как прежде");
            }
        }
        // Неготовое окно рисует Claude даже при выбранном GPT — с ним и сверка
        let settings = settings::Settings { design: settings::Design::Reference, ..settings::Settings::default() };
        assert!(!fit_is_stale(&settings, "debug", Some(settings::Design::App)));
        assert!(fit_is_stale(&settings, "debug", Some(settings::Design::Reference)));
    }
}
