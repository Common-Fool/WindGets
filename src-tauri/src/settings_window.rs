//! Окно настроек и его команды.
//!
//! Пришло на смену галочкам в трее и всплывающему окну ползунка. Меню трея
//! собирает `muda`, а на Windows это `CreatePopupMenu` — нативный `HMENU`,
//! куда кладут только пункты, галочки и подменю. Половина нынешних настроек —
//! ползунки и поля, и места им там нет вовсе.
//!
//! Окно обычное: его двигают, тянут за края, оно помнит своё место и
//! закрывается крестиком или Esc. В реестре виджетов его нет и быть не должно
//! — иначе оно получило бы строку в собственном списке и открывалось бы при
//! старте.
//!
//! Непрозрачное, в отличие от виджетов: стекло нужно тому, что живёт на
//! обоях, а настройки живут поверх работы.
//!
//! ВАЖНО: **окно одно, и настройки виджета — страница внутри него.** Второе
//! окно (`widget-settings`) прожило один этап и слито сюда: настройки виджета
//! — это раздел общих, а не вторая сущность. У слияния была цена, и её
//! пришлось заплатить целиком, см. ниже.
//!
//! ВАЖНО: **окно собирается при старте и прячется, а по требованию не
//! строится.** Это условие пришло от настроек виджета и распространилось на всё
//! окно: их открывают нажатием ВНУТРИ webview — шестерёнкой в списке и пунктом
//! меню виджета, — то есть посреди вызова IPC. Собранное в такой момент окно
//! вешает приём IPC во всём приложении: шестерёнка не делает ничего, а после
//! пары нажатий перестают закрываться и двигаться сами настройки, а за ними и
//! все виджеты. Поймано живьём на прошлой редакции.
//!
//! Пока окно звали только из трея — вне вызова IPC, — строить его по требованию
//! было безопасно. Слитое окно зовут из обоих мест, и слабейшее условие
//! проигрывает: строим заранее, показываем и прячем. Плата та же, что у
//! выключенных виджетов, — webview спрятанного окна остаётся в памяти.

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_window_state::{StateFlags, WindowExt};

use crate::settings::{self, SettingsState};
use crate::weather;
use crate::widgets;
use crate::window_effects;
use crate::windows::{self, keep_frameless};

/// Метка окна, она же `?widget=` в адресе.
pub const LABEL: &str = "settings";

/// Событие, которым вид настроек расходится по окнам. Слушает его одно окно,
/// но менять настройки могут и трей, и крестик на отладочном окне.
const SETTINGS_EVENT: &str = "settings";

/// Какого виджета показывать страницей. Пустая строка — никакого, то есть
/// общие настройки.
///
/// Событием, а не адресом: страница у окна одна на все виджеты и живёт от
/// старта до выхода, перезагружать её ради другого виджета незачем.
const TARGET_EVENT: &str = "settings-target";

/// Показано ли окно настроек: спрятанное снимает страницу целиком, и вместе с
/// ней — подписки на телеметрию и прочие потоки. Иначе окно, закрытое на
/// «Виджетах» или «Процессоре», круглые сутки разбирало бы снимок датчиков
/// каждую секунду, никому его не показывая.
const SHOWN_EVENT: &str = "settings-shown";

/// Размер окна постоянный (24 октября, слово пользователя): тянуть его
/// незачем, а суженное окно делало карточки раздела «Виджеты» нечитаемыми.
/// Колонка разделов с деревом — 216 точек (замер испанского «Generador de
/// contraseñas», `.settings__nav` в settings.css), справа карточки виджетов
/// в два столбца и живое превью выбранной. Что не влезает — сокращается или
/// уходит в подсказку, а не растит окно.
const WIDTH: f64 = 940.0;
const HEIGHT: f64 = 900.0;
const MIN_WIDTH: f64 = WIDTH;
const MIN_HEIGHT: f64 = HEIGHT;

/// Предел парой — для прилипания при растягивании (`windows::min_logical`).
pub const MIN_SIZE: (f64, f64) = (MIN_WIDTH, MIN_HEIGHT);

// Проверки времени сборки, а не теста: все четыре числа известны компилятору,
// и тест из них не мог бы упасть ни при какой правке кода — он лишь повторял
// бы то, что здесь и так видно. Промах ломает сборку, а не выхлоп `cargo test`.
const _: () = {
    // Окно тянут мышью, и предел должен помещаться внутрь размера по умолчанию
    assert!(MIN_WIDTH <= WIDTH && MIN_HEIGHT <= HEIGHT);
    // Колонка разделов и карточки виджетов рядом требуют места: общий предел
    // виджетов тут заведомо мал, и брать его нельзя
    assert!(MIN_WIDTH > widgets::MIN_WIDTH && MIN_HEIGHT > widgets::MIN_HEIGHT);
};

/// Строка списка виджетов.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetRow {
    pub id: &'static str,
    /// Название на выбранном языке.
    pub title: String,
    /// Служебное окно (отладка, витрина): галочки «Все» его не трогают, а
    /// в таблице оно стоит отдельным блоком.
    pub service: bool,
    pub enabled: bool,
    pub always_on_top: bool,
    /// Окно заперто поштучно. Общий рубильник этого флага не меняет.
    pub locked: bool,
    /// Номер вида: по нему карточка решает, чем себя рисовать.
    pub view: usize,
    /// Имена видов на выбранном языке. Пусто или одна запись — переключать
    /// нечего.
    pub views: Vec<String>,
    pub layout: usize,
    pub layouts: Vec<String>,
    /// Номер способа показать обложку.
    pub cover: usize,
    /// Имена способов. Пусто — обложки у виджета нет вовсе.
    pub covers: Vec<String>,
    /// Номер готового размера.
    pub size: usize,
    /// Имена готовых размеров — «Полный», «Без истории», «Только полоса».
    pub sizes: Vec<String>,
    pub size_indices: Vec<usize>,
    /// Окно тянется мышью, готовых размеров у него нет вовсе.
    pub flexible: bool,
    /// Своя плотность фона. `None` — как у всех.
    pub card_opacity: Option<f64>,
    /// Свой масштаб. `None` — как у всех.
    pub scale: Option<f64>,
    /// Размер окна при 100 % в нынешнем дизайне, виде, размере и компоновке:
    /// под него собирается живое превью в разделе «Виджеты». У карточек с
    /// высотой по содержимому это стартовая высота, точную сообщает превью.
    pub preview: (f64, f64),
}

/// Один пункт списка выбора — тема, стекло или язык.
#[derive(Debug, Clone, Serialize)]
pub struct ChoiceRow {
    pub id: &'static str,
    pub title: String,
}

/// Имена из реестра — ключи каталога — на выбранном языке.
fn names(keys: &[&str]) -> Vec<String> {
    keys.iter().map(|key| crate::i18n::text(key)).collect()
}

/// Темы и стёкла: имя на выбранном языке берётся из каталога по `id`.
fn choices(prefix: &str, ids: &[&'static str]) -> Vec<ChoiceRow> {
    ids.iter()
        .map(|id| ChoiceRow {
            id,
            title: crate::i18n::text(&format!("{prefix}.{id}")),
        })
        .collect()
}

/// Всё, что показывает окно настроек, одним снимком.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub widgets: Vec<WidgetRow>,
    pub card_opacity: f64,
    pub scale: f64,
    /// Все темы: имена для карточек берутся отсюда, а не пишутся во фронте.
    /// Выбранную окно знает из события вида, как и все остальные окна.
    pub palettes: Vec<ChoiceRow>,
    /// Все стёкла — по той же причине.
    pub glasses: Vec<ChoiceRow>,
    /// Яркость цветов.
    pub vivid: f64,
    pub autostart: bool,
    pub hidden_all: bool,
    pub locked: bool,
    pub snapping: bool,
    pub poll_seconds: u64,
    /// Город виджета погоды. `None` — не выбран.
    pub weather_place: Option<settings::Place>,
    /// Явное сворачивание приложений в мини-микшере звука.
    pub sound_mixer_collapsed: bool,
    pub design: settings::Design,
    /// Выбранный язык. `None` — «как в Windows».
    pub language: Option<String>,
    /// Все языки — самоназваниями, они не переводятся.
    pub languages: Vec<ChoiceRow>,
    /// Язык Windows: подпись «Как в Windows · Русский» говорит, что именно
    /// выберется, а не отсылает человека выяснять это самому.
    pub system_language: &'static str,
    /// Накопители, скрытые в карточке. Снимок настроек слушают и окна
    /// виджетов, так что карточка узнаёт о снятой галочке тем же событием.
    pub hidden_disks: Vec<String>,
    /// Снятые ряды карточек процессора и видеокарты, по виджету (этап 58).
    /// Едут тем же снимком, что и диски: карточка слушает его сама.
    pub hidden_metrics: std::collections::BTreeMap<String, Vec<String>>,
    /// Ручной выбор датчиков платы: место → `off` или путь без имени машины.
    pub sensor_picks: std::collections::BTreeMap<String, String>,
    /// Версия приложения — из `Cargo.toml`, единственного её источника:
    /// в `tauri.conf.json` и `package.json` её нет вовсе.
    pub version: String,
    /// «Напоминать реже» у напоминания о донатах (`support_reminder.rs`).
    pub support_rare: bool,
    /// Последний снимок датчиков сказал, что дискретной видеокарты нет.
    /// Живой снимок у окна пропадает, когда опрос засыпает вместе с модулями
    /// железа, а Rust помнит (`poller::lacks_graphics_card`): без этого
    /// галочка видеокарты оживала бы ровно тогда, когда «Включить все» её
    /// пропускает.
    pub no_gpu: bool,
    /// `settings.json` не прочитался при запуске, правки не сохраняются.
    pub unsaved: bool,
}

/// Размер окна виджета для превью. У гибких узких пресетов своей высоты нет —
/// окно держит ту, что натянули руками, — и превью берёт высоту полного
/// размера того же вида и компоновки.
fn preview_size(widget: &widgets::Widget, design: settings::Design, own: &settings::WidgetSettings) -> (f64, f64) {
    let (width, height) = widget.design_size(design, own.view, own.size, own.layout);
    let height = height
        .or_else(|| widget.design_size(design, own.view, 0, own.layout).1)
        .or(widget.full().height)
        .unwrap_or(widgets::MIN_HEIGHT);
    (width, height)
}

fn view(app: &AppHandle) -> SettingsView {
    let settings = app.state::<SettingsState>().get();

    let widgets = widgets::shown()
        .map(|widget| {
            let own = settings.widget(widget);
            WidgetRow {
                id: widget.id,
                title: widget.title(),
                service: widget.chrome,
                enabled: own.enabled,
                always_on_top: own.always_on_top,
                locked: own.locked,
                view: own.view,
                views: if widget.has_reference_views() && settings.design == settings::Design::App { Vec::new() } else { names(widget.views) },
                layout: own.layout,
                layouts: names(widget.layout_choices(settings.design, own.view)),
                size: own.size,
                size_indices: widget.size_choices(settings.design, own.view, own.layout).iter().map(|(index, _)| *index).collect(),
                sizes: widget
                    .size_choices(settings.design, own.view, own.layout)
                    .iter()
                    .map(|(_, preset)| crate::i18n::text(widget.size_name(settings.design, preset)))
                    .collect(),
                cover: own.cover,
                // Пусто там, где обложку при этом виде не настраивают вовсе
                covers: if widget.switches_covers(own.view) {
                    names(widget.covers)
                } else {
                    Vec::new()
                },
                flexible: widget.flexible,
                card_opacity: own.card_opacity,
                scale: own.scale,
                preview: preview_size(widget, settings.design, &own),
            }
        })
        .collect();

    SettingsView {
        widgets,
        card_opacity: settings.card_opacity,
        scale: settings.scale,
        palettes: choices("palette", &settings::PALETTES.iter().copied().filter(|id| *id != "gpt" || settings.design == settings::Design::Reference).collect::<Vec<_>>()),
        glasses: choices("glass", settings::GLASSES),
        vivid: settings.vivid,
        autostart: settings.autostart,
        hidden_all: settings.hidden_all,
        locked: settings.locked,
        snapping: settings.snapping,
        poll_seconds: settings.poll_seconds,
        weather_place: settings.weather,
        sound_mixer_collapsed: settings.sound_mixer_collapsed,
        design: settings.design,
        language: settings.language.clone(),
        languages: crate::i18n::LANGUAGES
            .iter()
            .map(|language| ChoiceRow {
                id: language.code,
                title: language.name.to_owned(),
            })
            .collect(),
        system_language: crate::i18n::system(),
        hidden_disks: settings.hidden_disks.clone(),
        hidden_metrics: settings.hidden_metrics.clone(),
        sensor_picks: settings.sensor_picks.clone(),
        version: app.package_info().version.to_string(),
        support_rare: settings.support_reminder.rare,
        no_gpu: crate::poller::lacks_graphics_card(),
        unsaved: settings::unsaved(),
    }
}

/// Заголовок окна настроек — на нынешнем языке: его видно в панели задач.
pub fn retitle(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.set_title(&tr!("window.settings"));
    }
}

/// Рассылает нынешний вид настроек. Зовётся после любой правки — откуда бы
/// она ни пришла.
pub fn emit(app: &AppHandle) {
    let _ = app.emit(SETTINGS_EVENT, view(app));
}

/// Собирает окно спрятанным. Зовётся один раз при старте, из главного потока.
///
/// Почему заранее, а не по требованию, — в заголовке модуля.
pub fn prepare(app: &AppHandle) {
    if let Err(error) = build(app) {
        eprintln!("окно настроек не собралось: {error}");
    }
}

/// Показывает общие настройки — пункт трея.
///
/// Зовётся из обработчика меню трея, то есть из главного потока: видимостью и
/// стилями окон ведает он (см. [`windows::on_main`]).
pub fn show(app: &AppHandle) {
    // Пустая цель — «покажи общие»: окно, оставшееся на странице виджета,
    // иначе открылось бы ею же в ответ на пункт «Настройки…» в трее
    target(app, "");
    reveal(app);
}

/// Показывает настройки одного виджета страницей внутри того же окна.
///
/// Зовётся из главного потока — из обработчика меню виджета и из команды
/// шестерёнки.
pub fn show_for(app: &AppHandle, id: &str) {
    // Настройки бывают только у виджета: чужая метка окна сюда не доедет
    let Some(widget) = page_of(id) else {
        return;
    };

    // ВАЖНО: **двигаем окно, только пока его не видно.** Виджеты живут на
    // втором мониторе, и настройки, оставшиеся на первом, открылись бы мимо
    // взгляда; но окно, на которое человек прямо сейчас смотрит, переезжать не
    // должно — это хуже любого промаха с монитором
    if !visible(app) {
        center_on_widget(app, widget.id);
    }

    target(app, id);
    reveal(app);
}

/// Цель «раздел „Поддержать“» — её шлёт карточка напоминания о донатах.
///
/// Раздел, а не страница виджета, поэтому с приставкой: имя раздела иначе
/// могло бы совпасть с меткой виджета. Фронт (`useTarget`) знает приставку и
/// переключает раздел колонки.
pub const SUPPORT_TARGET: &str = "section:support";

/// Показывает настройки на разделе «Поддержать». Из главного потока, как
/// [`show`]; окно не двигается — общий раздел ничьему виджету не принадлежит.
pub fn show_support(app: &AppHandle) {
    target(app, SUPPORT_TARGET);
    reveal(app);
}

/// Чья это страница: виджет из реестра, а у почты — ещё и её ящик.
///
/// Цель бывает составной (этап 53): `mail/<ключ ящика>` и `mail/new` —
/// страница одного ящика в дереве под почтой. Вторая часть — ключ из шестнадцати
/// шестнадцатеричных знаков или `new`; ничего другого окну не шлём, хоть оно и
/// своё.
fn page_of(target: &str) -> Option<&'static widgets::Widget> {
    let (id, sub) = match target.split_once('/') {
        Some((id, sub)) => (id, Some(sub)),
        None => (target, None),
    };

    let sane = |sub: &str| sub == "new" || (sub.len() == 16 && sub.bytes().all(|byte| byte.is_ascii_hexdigit()));
    match sub {
        None => widgets::find(id),
        Some(sub) if id == "mail" && sane(sub) => widgets::find(id),
        Some(_) => None,
    }
}

/// Говорит окну, чью страницу показывать. Пустая строка — общие настройки.
fn target(app: &AppHandle, id: &str) {
    let _ = app.emit_to(LABEL, TARGET_EVENT, id);
}

/// Видно ли окно человеку. Свёрнутое Windows считает видимым, а человек его не
/// видит: такое окно и центрировать можно, и разворачивать нужно.
fn visible(app: &AppHandle) -> bool {
    app.get_webview_window(LABEL)
        .is_some_and(|window| window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(false))
}

fn reveal(app: &AppHandle) {
    let Some(window) = app.get_webview_window(LABEL) else {
        // Сюда попадаем только после беды: окно собирается при старте
        eprintln!("окна настроек нет — оно не собралось при старте");
        return;
    };

    // До показа: страница успевает смонтироваться, пока окно выходит на экран
    let _ = app.emit_to(LABEL, SHOWN_EVENT, true);
    // Свёрнутое в панель задач окно `show` не трогает, а `set_focus` у tao
    // свёрнутые пропускает: трей, шестерёнка и второй запуск молчали бы
    if window.is_minimized().unwrap_or(false) {
        let _ = window.unminimize();
    }
    let _ = window.show();
    let _ = window.set_focus();
}

/// Ставит окно по центру окна виджета, прижимая в рабочую область того
/// монитора, где виджет стоит. Нет окна виджета — окно остаётся, где стояло.
fn center_on_widget(app: &AppHandle, id: &str) {
    let (Some(window), Some(widget_window)) =
        (app.get_webview_window(LABEL), app.get_webview_window(id))
    else {
        return;
    };

    if let (Ok(widget_pos), Ok(widget_size), Ok(settings_size), Ok(Some(monitor))) = (
        widget_window.outer_position(),
        widget_window.outer_size(),
        window.outer_size(),
        widget_window.current_monitor(),
    ) {
        let area = monitor.work_area();
        let pos = center_and_clamp(
            widget_pos,
            widget_size,
            settings_size,
            area.position,
            area.size,
        );
        let _ = window.set_position(pos);
    }
}

/// Вычисляет положение окна настроек: по центру окна виджета с прижатием
/// к границам рабочей области монитора, на котором стоит виджет.
fn center_and_clamp(
    widget_pos: PhysicalPosition<i32>,
    widget_size: PhysicalSize<u32>,
    settings_size: PhysicalSize<u32>,
    work_pos: PhysicalPosition<i32>,
    work_size: PhysicalSize<u32>,
) -> PhysicalPosition<i32> {
    let center_x = widget_pos.x + (widget_size.width as i32) / 2;
    let center_y = widget_pos.y + (widget_size.height as i32) / 2;

    let target_x = center_x - (settings_size.width as i32) / 2;
    let target_y = center_y - (settings_size.height as i32) / 2;

    let min_x = work_pos.x;
    let max_x = work_pos.x + work_size.width as i32 - settings_size.width as i32;
    let min_y = work_pos.y;
    let max_y = work_pos.y + work_size.height as i32 - settings_size.height as i32;

    // Окно шире монитора: прижимаем к началу рабочей области, иначе `clamp`
    // получил бы перевёрнутый отрезок и запаниковал
    let x = if max_x < min_x {
        min_x
    } else {
        target_x.clamp(min_x, max_x)
    };

    let y = if max_y < min_y {
        min_y
    } else {
        target_y.clamp(min_y, max_y)
    };

    PhysicalPosition::new(x, y)
}

fn build(app: &AppHandle) -> Result<WebviewWindow, String> {
    let url = format!("index.html?widget={LABEL}");
    // Размер один на оба дизайна и не меняется
    let (width, height) = (WIDTH, HEIGHT);

    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App(url.into()))
        .title(tr!("window.settings"))
        .additional_browser_args(crate::windows::BROWSER_ARGS)
        .inner_size(width, height)
        .min_inner_size(MIN_WIDTH, MIN_HEIGHT)
        .decorations(false)
        // Стекло тут не нужно: фон карточка рисует сплошным
        .transparent(false)
        // ВАЖНО: тень у окна без рамки tao делает через
        // `DwmExtendFrameIntoClientArea`, и вместе с ней по краям остаётся
        // светлая линия — та самая рамка с трёх сторон. У виджетов её не
        // видно только потому, что они прозрачные
        .shadow(false)
        // ВАЖНО: **кнопка в панели задач у настроек обязана быть.** Поверх
        // всех окно больше не поднимается вовсе, а значит проваливается за
        // чужие окна — и без кнопки в панели и без Alt+Tab доставать его
        // пришлось бы из трея каждый раз.
        .skip_taskbar(false)
        .resizable(false)
        // Поверх всех настройки не держим — ни насовсем, ни пока они в фокусе.
        // Плата известна и принята: виджеты сами поверх всех, поэтому окно
        // настроек будет уходить под них, и виджет придётся отодвинуть или
        // снять ему галочку «поверх»
        .always_on_top(false)
        // Прячем до восстановления позиции: иначе окно мигнёт посреди экрана
        .visible(false)
        .center()
        .build()
        .map_err(|error| error.to_string())?;

    // Та же защита, что у виджетов, и так же первой: окну без рамки tao точно
    // так же возвращает заголовок при каждом показе
    keep_frameless(&window);
    crate::windows::lock_browser(&window);

    // Место и размер помним наравне с виджетами: настройки открывают часто,
    // и каждый раз ловить окно посреди экрана — то ещё удовольствие
    // Только место: сохранённый прежде размер растянутого окна не возвращается
    let _ = window.restore_state(StateFlags::POSITION);

    // Прилипает как все окна, и оттуда же идёт запись позиций по концу
    // перетаскивания. Блокировка положения его не касается: в нём стоит её
    // собственный выключатель
    crate::snap::attach(app, &window, LABEL, false);

    // ВАЖНО: **закрытие мимо крестика тоже обязано прятать, а не уничтожать.**
    // Окно стоит в панели задач и Alt+F4 ловит, а уничтоженное собрать заново
    // нечем: сборка webview в рантайме подвешивает приём IPC во всём
    // приложении. Пока окно строилось по требованию, это было безобидно —
    // теперь нет.
    let handle = app.clone();
    window.on_window_event(move |event| {
        let WindowEvent::CloseRequested { api, .. } = event else {
            return;
        };
        // На выходе из приложения мешать нельзя: окнам положено закрыться
        if handle.state::<windows::Shutdown>().started() {
            return;
        }

        api.prevent_close();
        // Тем же путём, что и крестик: ползунки начисто, позиции на диск
        close_settings(handle.clone());
        // И в общий счёт закрытий: `taskkill` шлёт его всем окнам разом, и
        // с одним виджетом без этого счёта он выключил бы модуль
        windows::closing(&handle, "settings");
    });

    Ok(window)
}

/// Снимок настроек при открытии окна.
#[tauri::command]
pub fn settings_view(app: AppHandle) -> SettingsView {
    view(&app)
}

/// Один флаг виджета: `top` или `lock` — кнопки в дереве окна настроек.
///
/// Включение модуля идёт не сюда, а в `modules::set_module`: он собирает и
/// разбирает окно, а синхронная команда выполняется в главном потоке, и
/// сборка оттуда вешала приём IPC во всём приложении.
#[tauri::command]
pub fn set_widget_option(app: AppHandle, id: String, option: String, value: bool) {
    let Some(widget) = widgets::find(&id) else {
        return;
    };

    let state = app.state::<SettingsState>();
    let Some(own) = settings::update_widget(&app, &state, widget, |_, own| {
        settings::apply_option(own, &option, value)
    }) else {
        return;
    };
    emit(&app);

    windows::on_main(&app, move |app| windows::apply_by_id(&app, &id, own));
}

/// Вид виджета: чем именно карточка себя рисует.
///
/// Отдельной командой, а не столбцом в [`set_widget_option`]: там значение
/// булево, а здесь номер в списке. Зовут её двое — пункт меню виджета и окно
/// его настроек, — и ветка правки у обоих одна, ровно как у галочек.
///
/// ВАЖНО: **окну размер после этого ставится заново.** Вид меняет высоту
/// содержимого — у «полос» железа замер дал 52 точки разницы с «кольцами», — и
/// окно, оставшееся прежним, показывает дыру у нижнего края. Поправка живёт в
/// реестре полем `view_heights`, а прикладывает её `windows::apply_preset`.
#[tauri::command]
pub fn set_widget_view(app: AppHandle, id: String, index: usize) {
    let Some(widget) = widgets::find(&id) else {
        return;
    };
    // Номер приходит от нашего же меню, но проверяем всё равно: испорченный
    // не должен молча ставить вид, которого у виджета нет
    if index >= widget.views.len() {
        return;
    }

    let state = app.state::<SettingsState>();
    let Some(own) = settings::update_widget(&app, &state, widget, |design, own| {
        if own.view == index {
            return false;
        }
        own.view = index;

        // ВАЖНО: **набор размеров у нового вида может быть короче.** У пульта
        // раскладка приносит свои четыре размера, и запись от версии с другим их
        // числом оставила бы номер за краем набора: окно встало бы по первому
        // размеру, а меню держало бы галочку на несуществующем пункте
        let sizes = widget.presets_in(design, index).len();
        if sizes > 0 && own.size >= sizes {
            own.size = 0;
        }
        true
    }) else {
        return;
    };

    // Тот же путь, что и у смены размера: окно узнаёт о новой высоте числом, а
    // фронт получает `widget-size` и сворачивает раскрытые панели
    if !widget.flexible {
        let id = widget.id;
        let size = own.size;
        windows::on_main(&app, move |app| windows::set_size(&app, id, size));
    }

    emit(&app);
}

/// Запоминает только явное «Показать / Скрыть» у мини-микшера.
#[tauri::command]
pub fn set_sound_mixer_collapsed(app: AppHandle, collapsed: bool) {
    let state = app.state::<SettingsState>();
    if state.get().sound_mixer_collapsed == collapsed {
        return;
    }
    settings::update(&app, &state, |settings| settings.sound_mixer_collapsed = collapsed);
    emit(&app);
}

/// Компоновка не трогает тип графика, размер, данные и текущий замер.
#[tauri::command]
pub fn set_widget_layout(app: AppHandle, id: String, index: usize) {
    let Some(widget) = widgets::find(&id) else { return; };
    let state = app.state::<SettingsState>();
    let changed = settings::update_widget(&app, &state, widget, |design, own| {
        if index >= widget.layout_choices(design, own.view).len() || own.layout == index { return false; }
        own.layout = index;
        true
    });
    if changed.is_none() { return; }
    let id = widget.id;
    windows::on_main(&app, move |app| windows::set_layout(&app, id));
    emit(&app);
}

/// Как показана обложка: картинкой, фоном, крупно или никак.
///
/// Отдельной командой, а не столбцом: значение — номер в списке, как у вида и
/// размера. Окно от неё не меняется вовсе — карточка подстраивается сама.
#[tauri::command]
pub fn set_widget_cover(app: AppHandle, id: String, index: usize) {
    let Some(widget) = widgets::find(&id) else {
        return;
    };
    if index >= widget.covers.len() {
        return;
    }

    let state = app.state::<SettingsState>();
    let changed = settings::update_widget(&app, &state, widget, |_, own| {
        if own.cover == index {
            return false;
        }
        own.cover = index;
        true
    });
    if changed.is_none() {
        return;
    }
    emit(&app);
}

/// Готовый размер окна: ставит его окну и запоминает номер.
#[tauri::command]
pub fn set_widget_size(app: AppHandle, id: String, index: usize) {
    let Some(widget) = widgets::find(&id) else {
        return;
    };
    let state = app.state::<SettingsState>();
    let settings = state.get();
    let own = settings.widget(widget);
    if !widget.size_choices(settings.design, own.view, own.layout).iter().any(|(available, _)| *available == index) {
        return;
    }
    if own.size == index && widget.id != "disks" {
        return;
    }
    // Повторный выбор восстанавливает геометрию этих карточек даже после
    // старого оконного состояния, не создавая лишнюю запись настроек.
    settings::update_widget(&app, &state, widget, |_, own| {
        if own.size == index {
            return false;
        }
        own.size = index;
        true
    });

    let id = widget.id;
    windows::on_main(&app, move |app| windows::set_size(&app, id, index));
    emit(&app);
}

/// Общий путь для рубильника в настройках и в трее: сперва состояние,
/// затем окна и меню в главном потоке, после — снимок настроек.
pub fn set_hidden_all(app: &AppHandle, value: bool) {
    let state = app.state::<SettingsState>();
    settings::update(app, &state, |settings| settings.hidden_all = value);
    windows::on_main(app, move |app| {
        if value {
            windows::hide_all(&app);
        } else {
            windows::show_all(&app);
        }
        crate::tray::refresh(&app);
    });
    emit(app);
}

pub fn set_locked(app: &AppHandle, value: bool) {
    let state = app.state::<SettingsState>();
    settings::update(app, &state, |settings| settings.locked = value);
    apply_flags(app);
    windows::on_main(app, |app| crate::tray::refresh(&app));
    emit(app);
}

/// Рубильники: `hidden_all`, `locked`, `snapping`, `autostart`.
///
/// Одной командой, а не четырьмя: у всех четырёх одна форма и один ответ —
/// разослать новый вид. Незнакомое имя не делает ничего.
#[tauri::command]
pub fn set_flag(app: AppHandle, name: String, value: bool) {
    let state = app.state::<SettingsState>();

    match name.as_str() {
        "hidden_all" => return set_hidden_all(&app, value),
        "locked" => return set_locked(&app, value),
        "snapping" => {
            settings::update(&app, &state, |settings| settings.snapping = value);
            apply_flags(&app);
        }
        "autostart" => {
            // ВАЖНО: **не здесь, а в пуле.** Задачу планировщика ставит
            // `schtasks`, и его ждут, а синхронная команда выполняется в главном
            // потоке — на это время встал бы приём IPC во всех окнах.
            //
            // Два быстрых нажатия — две работы в пуле, и порядок между ними
            // никто не обещает: поздняя «включить» могла пройти после «выключить».
            // Поэтому работа берёт не своё значение, а последнее попрошенное
            settings::want_autostart(value);
            let app = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                settings::apply_autostart(&app);
                emit(&app);
            });
            return;
        }
        _ => return,
    }

    emit(&app);
}

/// Раздаёт прилипание оконной процедуре, а блокировку — самим окнам.
///
/// Прилипание идёт статическим флагом: сама процедура в настройки не ходит —
/// она работает внутри модального цикла, где событийный цикл стоит. Блокировка
/// же снимает окнам `WS_THICKFRAME`, а стилями ведает главный поток.
pub fn apply_flags(app: &AppHandle) {
    let settings = app.state::<SettingsState>().get();
    crate::snap::set_snapping(settings.snapping);

    windows::on_main(app, |app| windows::apply_lock(&app));
}

/// Шаг опроса датчиков.
///
/// Адреса тут больше нет: железо читает свой сайдкар, ходить некуда.
#[tauri::command]
pub fn set_source(app: AppHandle, seconds: u64) {
    let state = app.state::<SettingsState>();

    settings::update(&app, &state, |settings| {
        settings.poll_seconds = settings::sane_poll_seconds(seconds);
    });

    emit(&app);
}

/// Город виджета погоды.
///
/// Одна команда на выбор и на сброс: `None` — это «убрать город», после чего
/// виджет в сеть не ходит вовсе. Опрос про смену узнаёт сам, сравнивая
/// настройки со своим прошлым городом, — будить его каналом незачем.
#[tauri::command]
pub fn set_weather_place(app: AppHandle, place: Option<settings::Place>) {
    let state = app.state::<SettingsState>();

    settings::update(&app, &state, |settings| {
        settings.weather = settings::sane_place(place);
    });

    // Будим опрос: город теперь выбирают и прямо в карточке, и ждать там пять
    // секунд неподвижной панели поиска — это выглядит как непрошедшее нажатие
    weather::nudge(&app);

    emit(&app);
}

/// Прячет накопитель в карточке или возвращает его.
///
/// Короткая правка под замком и запись файла — синхронной команде это можно.
/// Последний показанный диск прячет не эта команда, а галочка в настройках:
/// она погашена. Сюда же скрытие всех дисков пропускается — карточка на это
/// отвечает своей строкой, а не пустотой.
#[tauri::command]
pub fn set_disk_hidden(app: AppHandle, name: String, hidden: bool) {
    let state = app.state::<SettingsState>();

    settings::update(&app, &state, |settings| {
        let name = name.trim().to_owned();
        settings.hidden_disks.retain(|kept| *kept != name);
        if hidden && !name.is_empty() {
            settings.hidden_disks.push(name);
        }
    });

    emit(&app);
}

/// Снять ряд с карточки процессора или видеокарты либо вернуть его (этап 58).
///
/// Синхронная и короткая — правка под замком и запись файла. Чужой виджет и
/// незнакомый ряд молча отбрасываются: строки приходят из окна.
#[tauri::command]
pub fn set_metric_hidden(app: AppHandle, widget: String, metric: String, hidden: bool) {
    if !settings::METRIC_WIDGETS.contains(&widget.as_str()) || !settings::METRICS.contains(&metric.as_str()) {
        return;
    }
    let state = app.state::<SettingsState>();

    settings::update(&app, &state, |settings| {
        let list = settings.hidden_metrics.entry(widget.clone()).or_default();
        list.retain(|kept| *kept != metric);
        if hidden {
            list.push(metric);
        }
        if list.is_empty() {
            settings.hidden_metrics.remove(&widget);
        }
    });

    emit(&app);
}

/// Выбрать датчик платы руками (этап 58): `None` или пустая строка —
/// автоматика, `off` — не показывать, иначе путь датчика без имени машины.
///
/// Опрос читает выбор на каждом такте, так что карточка сменит число за
/// секунду, без перезапуска сайдкара.
#[tauri::command]
pub fn set_sensor_pick(app: AppHandle, slot: String, pick: Option<String>) {
    if !settings::SENSOR_SLOTS.contains(&slot.as_str()) {
        return;
    }
    let state = app.state::<SettingsState>();

    settings::update(&app, &state, |settings| {
        settings.sensor_picks.remove(&slot);
        let pick = pick.map(|pick| pick.trim().to_owned()).unwrap_or_default();
        if !pick.is_empty() {
            settings.sensor_picks.insert(slot, pick);
        }
        settings.sensor_picks = settings::sane_picks(std::mem::take(&mut settings.sensor_picks));
    });

    emit(&app);
}

#[tauri::command]
pub fn reset_widget_places(app: AppHandle) {
    windows::on_main(&app, |app| windows::reset_places(&app));
}

/// Открывает папку с настройками в проводнике.
///
/// Рядом с `settings.json` лежат учётные данные Spotify и история спидтеста —
/// заглянуть туда проще, чем вспоминать путь под `AppData`.
///
/// ВАЖНО: запускаем проводник отдельным процессом, а не через `opener`. Тот
/// на Windows идёт в `ShellExecute`, то есть в COM, — а на COM же держится и
/// WebView2. Один такой вызов из потока команды оставил без ответа все webview
/// разом: приложение осталось живым, но нажатия перестали доходить куда-либо.
/// Отдельный процесс своего COM не трогает вовсе.
#[tauri::command]
pub fn open_config_dir(app: AppHandle) {
    if let Ok(dir) = app.path().app_config_dir() {
        open_folder(&dir, "настроек");
    }
}

/// Открывает папку журнала ошибок (`journal.rs`) — его прикладывают к жалобе.
///
/// В разработке журнал не ведётся, и папка бывает пустой: вывод тогда читают
/// в терминале запуска.
#[tauri::command]
pub fn open_log_dir(app: AppHandle) {
    if let Some(dir) = crate::journal::dir(&app) {
        open_folder(&dir, "журнала");
    }
}

/// Показывает папку в проводнике, создав её, если её ещё нет.
fn open_folder(dir: &std::path::Path, what: &str) {
    // Каталог может ещё не существовать: настройки пишутся при первой правке,
    // а журнал — только в собранном приложении
    if let Err(error) = std::fs::create_dir_all(dir) {
        eprintln!("каталог {what} не создался: {error}");
        return;
    }

    #[cfg(windows)]
    {
        // Ответа не ждём — `spawn`, а не `status`: проводник живёт своей жизнью.
        // Путь полный, а не голое имя: см. `autostart::system_exe`
        let explorer = crate::autostart::system_exe("explorer.exe");
        if let Err(error) = std::process::Command::new(explorer).arg(dir).spawn() {
            eprintln!("папка {what} не открылась: {error}");
        }
    }
}

/// Открывает настройки виджета — шестерёнкой из общего списка.
///
/// Через главный поток: команда приходит своим потоком, а видимостью и местом
/// окон ведает главный.
#[tauri::command]
pub fn open_widget_settings(app: AppHandle, id: String) {
    windows::on_main(&app, move |app| show_for(&app, &id));
}

/// Закрывает окно настроек.
///
/// Через команду, а не `window.close()` с фронта: место окна пишется на диск
/// по концу перетаскивания и на выходе, и закрытое из JS окно не попало бы ни
/// туда, ни туда.
///
/// Именно прячет, а не уничтожает: собрать его заново значило бы собрать
/// webview в рантайме, а окно открывают в том числе изнутри webview — см.
/// заголовок модуля.
#[tauri::command]
pub fn close_settings(app: AppHandle) {
    // Ползунки вида двигали начерно: на диск значение попадает здесь. Закрытое
    // посреди движения ползунка окно иначе оставило бы виджет с несохранённым
    // видом
    window_effects::commit(&app);

    windows::on_main(&app, |app| {
        let Some(window) = app.get_webview_window(LABEL) else {
            return;
        };

        windows::save_places(&app);
        let _ = window.hide();
        let _ = app.emit_to(LABEL, SHOWN_EVENT, false);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRONTEND_REGISTRY: &str = include_str!("../../src/widgets/registry.ts");

    /// Размеры окон по умолчанию для листа снимков в README
    /// (`scripts/widget-sheet/`): тот же `preview_size`, что у живого превью,
    /// в обоих дизайнах. Не проверка, а выгрузка — запускается руками:
    /// `cargo test --lib widget_sheet_sizes -- --ignored`.
    #[test]
    #[ignore]
    fn widget_sheet_sizes() {
        let mut out = serde_json::Map::new();
        for (name, design) in [("reference", settings::Design::Reference), ("app", settings::Design::App)] {
            let settings = settings::Settings { design, ..settings::Settings::default() };
            let mut sizes = serde_json::Map::new();
            for widget in widgets::shown().filter(|widget| !widget.chrome) {
                let (width, height) = preview_size(widget, design, &settings.widget(widget));
                sizes.insert(widget.id.to_owned(), serde_json::json!([width, height]));
            }
            out.insert(name.to_owned(), serde_json::Value::Object(sizes));
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/widget-sheet/sizes.json");
        std::fs::create_dir_all(path.parent().expect("папка")).expect("папка листа");
        let text = serde_json::to_string_pretty(&serde_json::Value::Object(out)).expect("json");
        std::fs::write(&path, text + "
").expect("запись размеров");
    }

    /// Окна нет в реестре виджетов, но компонент ему всё равно нужен:
    /// без записи на фронте всплывёт «виджет не найден».
    #[test]
    fn settings_window_has_a_component_on_the_front() {
        assert!(FRONTEND_REGISTRY.contains(&format!("id: \"{LABEL}\"")));
    }

    /// И при этом виджетом оно не притворяется: иначе попало бы в собственный
    /// список и открывалось бы при старте.
    #[test]
    fn settings_window_is_not_a_widget() {
        assert!(widgets::find(LABEL).is_none());
    }

    /// Версия живёт в одном месте — `Cargo.toml`. Без поля в
    /// `tauri.conf.json` Tauri берёт её оттуда сам, а `package.json` у
    /// закрытого пакета без неё обходится. Вторая копия разошлась бы с первой
    /// на ближайшем выпуске, и проверка обновлений сверялась бы не с тем.
    #[test]
    fn the_version_lives_in_cargo_toml_only() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for file in ["tauri.conf.json", "../package.json"] {
            let text = std::fs::read_to_string(root.join(file)).expect("файл читается");
            let json: serde_json::Value = serde_json::from_str(&text).expect("это JSON");
            assert!(json.get("version").is_none(), "{file}: версия — только в Cargo.toml");
        }
        assert!(!env!("CARGO_PKG_VERSION").is_empty());
    }

    /// Составная цель — только у почты и только с ключом ящика или `new`.
    #[test]
    fn a_mailbox_page_is_the_only_nested_one() {
        let id = |target: &str| page_of(target).map(|widget| widget.id);

        assert_eq!(id("weather"), Some("weather"));
        assert_eq!(id("mail"), Some("mail"));
        assert_eq!(id("mail/new"), Some("mail"));
        assert_eq!(id("mail/0123456789abcdef"), Some("mail"));

        assert_eq!(id("mail/../../etc"), None);
        assert_eq!(id("mail/"), None);
        assert_eq!(id("weather/new"), None, "ящики бывают только у почты");
        assert_eq!(id("nothing"), None);
    }

    /// Второго окна настроек больше нет: настройки виджета — страница внутри
    /// этого. Оставленная на фронте запись подняла бы второе окно, которое
    /// некому показать и нечем закрыть.
    #[test]
    fn the_second_settings_window_is_gone() {
        assert!(!FRONTEND_REGISTRY.contains("widget-settings"));
    }

    #[test]
    fn centers_settings_on_widget_within_primary_monitor() {
        let widget_pos = PhysicalPosition::new(800, 400);
        let widget_size = PhysicalSize::new(320, 200);
        let settings_size = PhysicalSize::new(680, 520);
        let work_pos = PhysicalPosition::new(0, 0);
        let work_size = PhysicalSize::new(1920, 1080);

        // Центр виджета: (800 + 160, 400 + 100) = (960, 500)
        // Идеальное положение окна настроек: (960 - 340, 500 - 260) = (620, 240)
        let pos = center_and_clamp(widget_pos, widget_size, settings_size, work_pos, work_size);
        assert_eq!(pos, PhysicalPosition::new(620, 240));
    }

    #[test]
    fn handles_negative_coordinates_on_secondary_monitor() {
        // Второй монитор слева от основного: x от -1920 до 0
        let work_pos = PhysicalPosition::new(-1920, 0);
        let work_size = PhysicalSize::new(1920, 1080);
        let widget_pos = PhysicalPosition::new(-1000, 200);
        let widget_size = PhysicalSize::new(300, 200);
        let settings_size = PhysicalSize::new(680, 520);

        // Центр виджета: (-1000 + 150, 200 + 100) = (-850, 300)
        // Идеальное положение: (-850 - 340, 300 - 260) = (-1190, 40)
        let pos = center_and_clamp(widget_pos, widget_size, settings_size, work_pos, work_size);
        assert_eq!(pos, PhysicalPosition::new(-1190, 40));
    }

    #[test]
    fn clamps_to_screen_boundaries_at_edges() {
        let work_pos = PhysicalPosition::new(-1920, 0);
        let work_size = PhysicalSize::new(1920, 1080);
        let settings_size = PhysicalSize::new(680, 520);

        // Виджет у самого правого края второго монитора (x близко к 0)
        let widget_pos = PhysicalPosition::new(-100, 100);
        let widget_size = PhysicalSize::new(100, 100);
        let pos = center_and_clamp(widget_pos, widget_size, settings_size, work_pos, work_size);
        // Правый край монитора: 0; максимальный x для окна: 0 - 680 = -680
        assert_eq!(pos.x, -680);

        // Виджет у левого края второго монитора (x близко к -1920)
        let widget_pos = PhysicalPosition::new(-1910, 100);
        let pos = center_and_clamp(widget_pos, widget_size, settings_size, work_pos, work_size);
        // Минимальный x: -1920
        assert_eq!(pos.x, -1920);
    }

    /// Монитор уже окна — `clamp` получил бы перевёрнутый отрезок и запаниковал.
    /// Вертикальный второй монитор такое устраивает не в теории: 1080 точек
    /// ширины против окна в 680 запас невелик, а ползунок кегля его тратит.
    #[test]
    fn oversize_settings_window_does_not_panic() {
        let work_pos = PhysicalPosition::new(0, 0);
        let work_size = PhysicalSize::new(300, 300);
        let settings_size = PhysicalSize::new(680, 520);
        let widget_pos = PhysicalPosition::new(100, 100);
        let widget_size = PhysicalSize::new(50, 50);

        let pos = center_and_clamp(widget_pos, widget_size, settings_size, work_pos, work_size);
        assert_eq!(pos, PhysicalPosition::new(0, 0));
    }
}
