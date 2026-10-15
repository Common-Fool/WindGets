use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::widgets::Widget;

/// Плотная заливка — вид по умолчанию. Обои видны только по краям карточки.
pub const DEFAULT_CARD_OPACITY: f64 = 0.94;

/// Ниже фон начинает зависеть от обоев: настоящего размытия под окном нет, и
/// на светлых обоях мелкий текст и контролы полупрозрачной карточки перестают
/// читаться.
pub const MIN_CARD_OPACITY: f64 = 0.86;

/// Кегль как в токенах: ползунок начинает отсюда.
pub const DEFAULT_SCALE: f64 = 1.0;

/// Пределы масштаба. Ниже подписи перестают читаться, выше начинает страдать
/// раскладка внутри карточки: окна за масштабом подтягиваются, а вот
/// перекладываться под свой размер виджеты пока не умеют.
pub const MIN_SCALE: f64 = 0.9;
pub const MAX_SCALE: f64 = 1.3;

/// Цветовые темы — идентификаторы. Имя для окна настроек — в каталоге строк,
/// `palette.<id>`.
///
/// Тема задаёт базы — четыре у режимов и до шести у тем, — а роли данных,
/// состояний и интерфейса ссылаются на них в tokens.css. Стекло хранится
/// отдельно и при смене темы остаётся.
///
/// ВАЖНО: **первая — тема по умолчанию**, и в tokens.css она объявлена ещё и
/// для окна без атрибута `data-palette`: окно, до которого настройки ещё не
/// доехали, рисует ею, а не пустотой.
pub const PALETTES: &[&str] = &[
    "reference-classic",
    "reference-polar",
    "reference-indigo",
    "reference-mint",
    "reference-spectrum",
    "reference-a",
    "reference-b",
    "reference-c",
    "monochrome",
    "single",
    "custom",
    "gpt",
];

/// Стекло — фон карточек, выбирается независимо от темы: цвет подложки
/// заимствовать у акцента нельзя, насыщенный акцент на подложке того же
/// оттенка перестаёт читаться. По умолчанию Claude — obsidian, GPT — gpt;
/// порядок образцов не задаёт умолчание. Имена — `glass.<id>`.
pub const GLASSES: &[&str] = &["graphite", "obsidian", "warm", "midnight", "plum", "gpt"];

/// Цвет режима «Один акцент» по умолчанию — синий акцент темы Polar.
///
/// Фиолетовый акцент управления исключён самим сведением палитры, поэтому и
/// умолчание у режима не фиолетовое.
pub const DEFAULT_ACCENT_COLOR: &str = "#4CA5FF";

/// Четыре базы режима «Своя палитра» и их значения по умолчанию — базы Polar.
///
/// Отдельного цвета у каждого показателя нет намеренно: база меняет всех своих
/// потребителей разом, и одинаковый смысл остаётся одинаковым цветом.
///
/// ВАЖНО: те же числа стоят в tokens.css (`--single-accent`, `--custom-*`) и в
/// `look.ts`: ими окно рисует, пока настройки ещё не доехали. Правя тут,
/// править и там.
pub const CUSTOM_COLORS: &[(&str, &str)] = &[
    ("accent", DEFAULT_ACCENT_COLOR),
    ("positive", "#40D9A3"),
    ("caution", "#F0B857"),
    ("negative", "#F17882"),
];

/// Пределы яркости цветов.
///
/// Ползунок двигает светлоту и цветность всех баз темы разом, фон и обычный
/// текст не трогает. Поле в файле зовётся `vivid` по прежней насыщенности:
/// переименование ради нового смысла потеряло бы настройку при чтении старого
/// файла.
pub const DEFAULT_VIVID: f64 = 1.0;
pub const MIN_VIVID: f64 = 0.7;
pub const MAX_VIVID: f64 = 1.5;

/// Общий дизайн независимо от раскладки: Claude Design или GPT Design.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Design {
    App,
    #[default]
    Reference,
}

/// Один сохранённый набор оформления. Масштаб, плотность и яркость общие.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesignAppearance {
    pub palette: String,
    #[serde(deserialize_with = "deserialize_glass")]
    pub glass: String,
    #[serde(deserialize_with = "deserialize_accent_color")]
    pub accent_color: String,
    #[serde(deserialize_with = "deserialize_custom_colors")]
    pub custom_colors: BTreeMap<String, String>,
}

impl Default for DesignAppearance {
    fn default() -> Self { Self::for_design(Design::App) }
}

impl DesignAppearance {
    pub fn for_design(design: Design) -> Self {
        Self {
            palette: if design == Design::Reference { "gpt".to_owned() } else { default_palette() },
            glass: if design == Design::Reference { "gpt".to_owned() } else { default_glass() },
            accent_color: DEFAULT_ACCENT_COLOR.to_owned(),
            custom_colors: default_custom_colors(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesignAppearances {
    #[serde(deserialize_with = "deserialize_app_appearance")]
    pub app: DesignAppearance,
    #[serde(default = "default_reference_appearance", deserialize_with = "deserialize_reference_appearance")]
    pub reference: DesignAppearance,
}

impl Default for DesignAppearances {
    fn default() -> Self { Self { app: DesignAppearance::default(), reference: default_reference_appearance() } }
}

fn default_reference_appearance() -> DesignAppearance { DesignAppearance::for_design(Design::Reference) }

// Каждое поле восстанавливается отдельно: повреждённый профиль не сбрасывает город и окна.
fn read_appearance(value: serde_json::Value, design: Design) -> DesignAppearance {
    let mut appearance = DesignAppearance::for_design(design);
    if let Some(object) = value.as_object() {
        if let Some(name) = object.get("palette").and_then(serde_json::Value::as_str) {
            appearance.palette = palette_for_design(name.to_owned(), design);
        }
        if let Some(name) = object.get("glass").and_then(serde_json::Value::as_str) {
            if name == "slate" || GLASSES.contains(&name) { appearance.glass = sane_glass(name.to_owned()); }
        }
        if let Some(color) = object.get("accent_color").and_then(serde_json::Value::as_str) {
            appearance.accent_color = sane_color(color, DEFAULT_ACCENT_COLOR);
        }
        if let Some(colors) = object.get("custom_colors") {
            appearance.custom_colors = sane_custom_colors(colors.as_object().into_iter()
                .flat_map(|entries| entries.iter())
                .filter_map(|(role, color)| color.as_str().map(|color| (role.clone(), color.to_owned())))
                .collect());
        }
    }
    appearance
}

fn deserialize_app_appearance<'de, D: serde::Deserializer<'de>>(d: D) -> Result<DesignAppearance, D::Error> {
    Ok(read_appearance(serde_json::Value::deserialize(d)?, Design::App))
}
fn deserialize_reference_appearance<'de, D: serde::Deserializer<'de>>(d: D) -> Result<DesignAppearance, D::Error> {
    Ok(read_appearance(serde_json::Value::deserialize(d)?, Design::Reference))
}

/// GPT-палитра не доступна техническим окнам и Claude.
pub fn palette_for_design(name: String, design: Design) -> String {
    let name = sane_palette(name);
    if design == Design::App && name == "gpt" { default_palette() } else { name }
}

fn deserialize_design<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Design, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value.as_str() {
        Some("app") => Design::App,
        _ => Design::Reference,
    })
}

pub fn default_palette() -> String {
    PALETTES[0].to_owned()
}

/// Незнакомое имя — это правка файла руками или запись от версии, где тема
/// звалась иначе. Молча съезжать на первую правильнее, чем раздавать окнам
/// атрибут, под который в токенах ничего нет.
///
/// Прежние вариации переводятся в ближайшее по смыслу: «Один акцент» и
/// «Монохром» — в одноимённые режимы, «Многоцвет» — в тему по умолчанию.
pub fn sane_palette(value: String) -> String {
    match value.as_str() {
        "accent" => "single".to_owned(),
        "mono" => "monochrome".to_owned(),
        _ if PALETTES.contains(&value.as_str()) => value,
        _ => default_palette(),
    }
}

pub fn default_glass() -> String {
    "obsidian".to_owned()
}

/// Незнакомое стекло — то же, что незнакомая тема: съезжаем на первое.
pub fn sane_glass(value: String) -> String {
    if value == "slate" { return "gpt".to_owned(); }
    if GLASSES.contains(&value.as_str()) {
        value
    } else {
        default_glass()
    }
}

/// Цвет, пригодный для CSS: только шестизначный непрозрачный HEX.
///
/// Строка из файла уходит в окно переменной `--custom-*`, и всё прочее —
/// имена цветов, `var(...)`, произвольный текст — через эту границу не проходит
/// вовсе.
pub fn sane_color(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if value.len() == 7
        && value.starts_with('#')
        && value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
    {
        value.to_ascii_uppercase()
    } else {
        fallback.to_owned()
    }
}

pub fn default_custom_colors() -> BTreeMap<String, String> {
    CUSTOM_COLORS
        .iter()
        .map(|(role, color)| ((*role).to_owned(), (*color).to_owned()))
        .collect()
}

/// Ровно четыре базы, каждая пригодная для CSS: чужие ключи отбрасываются,
/// недостающие и испорченные берут значение по умолчанию.
pub fn sane_custom_colors(colors: BTreeMap<String, String>) -> BTreeMap<String, String> {
    CUSTOM_COLORS
        .iter()
        .map(|(role, fallback)| {
            let value = colors.get(*role).map(String::as_str).unwrap_or(fallback);
            ((*role).to_owned(), sane_color(value, fallback))
        })
        .collect()
}

/*
 * Разбор цветовых полей из файла.
 *
 * ВАЖНО: **испорченное поле не должно сбрасывать весь файл.** Строгий разбор на
 * `false` вместо строки уронил бы `from_str` целиком, и `unwrap_or_default`
 * вернул бы к умолчанию всё разом — город, автозапуск, окна. Поэтому эти поля
 * читаются как любое значение JSON и приводятся к годному на месте.
 */

fn deserialize_glass<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(sane_glass(value.as_str().unwrap_or_default().to_owned()))
}

fn deserialize_accent_color<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(sane_color(value.as_str().unwrap_or_default(), DEFAULT_ACCENT_COLOR))
}

fn deserialize_custom_colors<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, String>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    let colors = value
        .as_object()
        .into_iter()
        .flat_map(|entries| entries.iter())
        .filter_map(|(role, color)| color.as_str().map(|color| (role.clone(), color.to_owned())))
        .collect();
    Ok(sane_custom_colors(colors))
}

/// Яркость в рабочих пределах. Число ниже нынешнего предела могло остаться от
/// прежней насыщенности (там низ был 30 %) и зажимается здесь же: и чтение
/// файла, и живой показ видят один и тот же результат.
pub fn sane_vivid(value: f64) -> f64 {
    if value.is_nan() {
        return DEFAULT_VIVID;
    }
    value.clamp(MIN_VIVID, MAX_VIVID)
}

/// Шаг опроса датчиков в секундах.
///
/// Ниже секунды опрашивать нечего — датчики столько и обновляются, — а выше
/// пяти история столбиками растянулась бы на четверть часа и перестала
/// показывать «что было только что».
pub const DEFAULT_POLL_SECONDS: u64 = 1;
pub const MIN_POLL_SECONDS: u64 = 1;
pub const MAX_POLL_SECONDS: u64 = 5;

/// Приводит плотность фона к рабочим пределам.
///
/// Правка файла руками не должна делать окна невидимыми: значение вне
/// пределов — это опечатка, а не желание получить пустой экран.
pub fn sane_opacity(value: f64) -> f64 {
    if value.is_nan() {
        return DEFAULT_CARD_OPACITY;
    }
    value.clamp(MIN_CARD_OPACITY, 1.0)
}

/// То же для кегля: испорченное число не должно рассыпать все карточки разом.
pub fn sane_scale(value: f64) -> f64 {
    if value.is_nan() {
        return DEFAULT_SCALE;
    }
    value.clamp(MIN_SCALE, MAX_SCALE)
}

/// Шаг опроса. Ноль из файла означал бы запросы без передышки.
pub fn sane_poll_seconds(value: u64) -> u64 {
    value.clamp(MIN_POLL_SECONDS, MAX_POLL_SECONDS)
}

/// Город виджета погоды: имя для показа и координаты для запроса.
///
/// Имя приходит от геокодера Open-Meteo вместе с координатами и на карточке
/// стоит подписью под температурой. Сами координаты в карточку не попадают
/// никогда — по ним человеку не узнать, тот ли город он выбрал.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Place {
    /// «Грац».
    pub name: String,
    /// «Штирия, Австрия» — уточнение для списка находок.
    pub region: String,
    pub latitude: f64,
    pub longitude: f64,
}

/// Точность координат в знаках после запятой.
///
/// Четыре знака — это около одиннадцати метров, для погоды с запасом. Заодно
/// попадаем в кеш Open-Meteo: он считается по округлённым координатам, и
/// шестнадцать знаков от геокодера промахивались бы мимо него каждый раз.
const PLACE_PRECISION: f64 = 10_000.0;

/// Приводит город к пригодному для запроса виду.
///
/// Возвращает `None`, если города по сути нет: без имени его нечем подписать, а
/// с испорченными координатами запрос ушёл бы в никуда. Правка файла руками не
/// должна ронять опрос, поэтому зовётся и при чтении с диска, и в команде.
pub fn sane_place(place: Option<Place>) -> Option<Place> {
    let place = place?;

    let name = place.name.trim();
    if name.is_empty() {
        return None;
    }

    if place.latitude.is_nan() || place.longitude.is_nan() {
        return None;
    }

    Some(Place {
        name: name.to_owned(),
        region: place.region.trim().to_owned(),
        latitude: round_place(place.latitude.clamp(-90.0, 90.0)),
        longitude: round_place(place.longitude.clamp(-180.0, 180.0)),
    })
}

fn round_place(value: f64) -> f64 {
    (value * PLACE_PRECISION).round() / PLACE_PRECISION
}

/// Настройки приложения.
///
/// Позиции и размеры окон сюда не попадают: их ведёт
/// `tauri-plugin-window-state` в своём файле.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Плотность фона карточек, от `MIN_CARD_OPACITY` до единицы.
    ///
    /// Пришла на смену галочке «прозрачнее фон»: у той нижнее положение
    /// открывало заголовок окна под карточкой. Сам заголовок с тех пор убран
    /// (`native_frame.rs`), а предел держится ради контраста текста.
    pub card_opacity: f64,
    /// Масштаб карточки: буквы, отступы и толщины разом.
    ///
    /// Псевдоним поля обязателен: до переименования ползунок звался «размером
    /// шрифта», и в файлах у людей лежит `font_scale`. Без псевдонима правка
    /// молча вернула бы всех к сотне процентов — то есть выглядела бы как
    /// потерянная настройка, а не как несовпадение имён.
    #[serde(alias = "font_scale")]
    pub scale: f64,
    /// Независимые наборы Claude и GPT; прежние поля переносятся при чтении.
    pub appearance: DesignAppearances,
    /// Яркость цветов, от [`MIN_VIVID`] до [`MAX_VIVID`].
    pub vivid: f64,
    /// Запуск вместе с системой.
    pub autostart: bool,
    /// Рубильник «спрятать все»: окна закрыты, но список включённых цел.
    ///
    /// Именно отдельное поле, а не сброс `enabled` у каждого: опущенный
    /// рубильник обязан вернуть ровно то, что было до него.
    pub hidden_all: bool,
    /// Положение и размер окон заперты: ни подвинуть, ни растянуть.
    pub locked: bool,
    /// Прилипание к краям монитора и к соседним окнам.
    pub snapping: bool,
    /// Шаг опроса датчиков в секундах.
    pub poll_seconds: u64,
    /// Город виджета погоды.
    ///
    /// `None` — не выбран, и тогда в сеть не ходим вовсе. Именно `Option`, а не
    /// пара чисел с нулями: координаты 0,0 — это точка в Гвинейском заливе, и
    /// «города нет» она бы не выражала.
    pub weather: Option<Place>,
    /// Мини-микшер звука: явное «Скрыть» переживает смену вида и перезапуск.
    pub sound_mixer_collapsed: bool,
    /// Единственный общий выбор дизайна. У виджетов собственных значений нет.
    #[serde(deserialize_with = "deserialize_design")]
    pub design: Design,
    /// Без поля — прежние номера B/A/C; новая запись уже использует микшер/пульт/столбик.
    #[serde(default)]
    pub sound_layout_version: u8,
    /// Версия индексов размеров пары после удаления «Без истории».
    #[serde(default)]
    pub hardware_layout_version: u8,
    /// Настройки по виджетам, ключ — `id` из реестра. Отсутствующий виджет
    /// означает «настроек не было», и значения берутся из реестра.
    pub widgets: BTreeMap<String, WidgetSettings>,
    /// Свои папки вывода конвертера и загрузчика, ключ — `id` модуля.
    ///
    /// Нет ключа — папка по умолчанию на рабочем столе, и она не записывается
    /// сюда, а считается заново при каждом запросе: стол, перенесённый на
    /// другой диск или в OneDrive, найдётся сам. Карта, а не два поля: новый
    /// модуль не просит правки этой структуры.
    pub output_dirs: BTreeMap<String, String>,
    /// Язык интерфейса: код из [`crate::i18n::LANGUAGES`].
    ///
    /// `None` — «как в Windows», и язык системы смотрится на каждом запуске, а
    /// не записывается сюда однажды: сменивший язык Windows человек получит
    /// новый и в приложении. Та же логика `Option`, что у своей плотности
    /// виджета.
    pub language: Option<String>,
    /// Значок, выбранный для области уведомлений Windows.
    pub app_icon: String,
    /// Накопители, скрытые в карточке, — именами из выхлопа LHM (модель).
    ///
    /// ВАЖНО: **хранятся скрытые, а не показанные.** Новый диск появляется в
    /// карточке сам, как и до этапа 53: модели в коде не зашиты, и список
    /// показанных прятал бы любой купленный накопитель, пока его не отметят.
    pub hidden_disks: Vec<String>,
    /// Ряды, снятые с карточек процессора и видеокарты (этап 58): ключ — `cpu`
    /// или `gpu`, значения из [`METRICS`].
    ///
    /// Хранятся снятые, а не показанные, — как диски: ряд, которого прежде не
    /// было (напряжение ядра на этапе 57), появляется сам. Ключа нет — у
    /// виджета показано всё.
    pub hidden_metrics: BTreeMap<String, Vec<String>>,
    /// Ручной выбор датчиков платы (этап 58): место из [`SENSOR_SLOTS`] →
    /// [`PICK_OFF`] или путь датчика без имени машины.
    ///
    /// Нужен незнакомой плате: там вентиляторы зовутся «Fan #1…», и который
    /// из них процессорный, автоматика не угадает. Ключа нет — автоматика.
    /// Выбранный датчик пропал (сменили плату) — тоже автоматика, а не пусто.
    pub sensor_picks: BTreeMap<String, String>,
    /// Напоминание о донатах (`support_reminder.rs`): когда приложение
    /// впервые запустилось, когда карточка была в последний раз и просили
    /// ли напоминать реже.
    pub support_reminder: SupportReminder,
    /// Проверять обновления самим, раз в неделю (`update.rs`). Снято — в
    /// сеть за выпусками не ходим, остаётся «Проверить сейчас».
    pub update_auto: bool,
    /// Когда спрашивали GitHub о выпуске, секунды Unix. От неё и считается
    /// неделя: машину включают каждый день, и проверка «при запуске» была бы
    /// ежедневной.
    pub update_checked_at: Option<i64>,
}

/// Состояние напоминания о донатах. Сроки и правило — в `support_reminder.rs`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SupportReminder {
    /// Первый запуск, секунды Unix. Ноль — ещё не отмечен: отмечает его сам
    /// поток напоминания при первой проверке.
    pub first_run: u64,
    /// Когда карточка показывалась в последний раз.
    pub last_shown: Option<u64>,
    /// «Напоминать реже» — с карточки или галочкой в «Поддержать»: не чаще
    /// раза в полгода. Полного выключения нет — слово пользователя.
    pub rare: bool,
}

/// Ряды карточек процессора и видеокарты, которые можно снять. Загрузки
/// (кольца или полосы) не снимаются никогда — это слово пользователя.
pub const METRICS: &[&str] = &["temperature", "fan", "clock", "power", "voltage"];

/// Виджеты, у которых ряды снимаются.
pub const METRIC_WIDGETS: &[&str] = &["cpu", "gpu"];

/// Места, где датчик платы выбирают руками, — те же имена, что у фронта.
pub const SENSOR_SLOTS: &[&str] = &["cpuFan", "chassisFan", "boardTemp", "cpuVoltage"];

/// «Не показывать» в ручном выборе: путь датчика так выглядеть не может.
pub const PICK_OFF: &str = "off";

/// Значок, который виден в области уведомлений Windows.
pub const DEFAULT_APP_ICON: &str = "widget-mosaic";
pub const APP_ICON_IDS: &[&str] = &[
    "widget-mosaic",
    "mono-mosaic",
];

impl Default for Settings {
    fn default() -> Self {
        Self {
            card_opacity: DEFAULT_CARD_OPACITY,
            scale: DEFAULT_SCALE,
            appearance: DesignAppearances { app: DesignAppearance::default(), reference: default_reference_appearance() },
            vivid: DEFAULT_VIVID,
            autostart: false,
            hidden_all: false,
            locked: false,
            snapping: true,
            poll_seconds: DEFAULT_POLL_SECONDS,
            weather: None,
            sound_mixer_collapsed: false,
            design: Design::Reference,
            sound_layout_version: 1,
            hardware_layout_version: 1,
            widgets: BTreeMap::new(),
            output_dirs: BTreeMap::new(),
            language: None,
            app_icon: DEFAULT_APP_ICON.to_owned(),
            hidden_disks: Vec::new(),
            hidden_metrics: BTreeMap::new(),
            sensor_picks: BTreeMap::new(),
            support_reminder: SupportReminder::default(),
            update_auto: true,
            update_checked_at: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct WidgetSettings {
    /// Модуль включён: окно собрано, фоновые работы идут. Выключенный не
    /// грузится вовсе (`modules.rs`). Имя поля — с тех пор, когда оно
    /// значило «окно показано»: так старый `settings.json` читается как есть.
    pub enabled: bool,
    pub always_on_top: bool,
    /// Окно не ловит мышь: щелчки уходят на рабочий стол под ним.
    pub click_through: bool,
    /// Окно заперто поштучно: ни подвинуть мышью, ни растянуть.
    ///
    /// Общий рубильник `Settings::locked` от этого не отменяется и остаётся
    /// главнее: свой флаг запирает, но отпереть запертое общим не может.
    /// Иначе рубильник, ради которого лезут в трей, обходился бы галочкой.
    pub locked: bool,
    /// Номер вида в реестре. Ноль — первый, он же основной.
    ///
    /// Вид и размер — разные вещи, и путать их дорого: «только замер» у
    /// спидтеста ВЫШЕ обычного вида, а не ниже, и порогом по высоте от него не
    /// отличается никак. Три захода подряд это ловилось живьём.
    pub view: usize,
    /// Компоновка, независимая от вида истории (спидтест).
    pub layout: usize,
    /// Номер способа показать обложку. Ноль — первый.
    ///
    /// Третья ось помимо вида и размера, и заведена она ровно затем, чтобы не
    /// перемножать их: у пульта четыре обложки на три раскладки на четыре
    /// размера, и одним списком это сорок восемь пунктов меню.
    pub cover: usize,
    /// Номер готового размера в реестре. Ноль — полный.
    ///
    /// Именно номер, а не сами точки: размеры живут в реестре и меняются
    /// вместе с раскладкой карточки, а записанные в файл точки пережили бы
    /// правку порогов и разъехались бы с ней молча.
    pub size: usize,
    /// Своя плотность фона. `None` — та же, что у всех.
    ///
    /// Именно `Option`, а не копия общего числа: скопированное однажды, оно
    /// перестало бы слушать общий ползунок навсегда, и «как у всех» вернуть
    /// было бы нечем. Заводится только у того виджета, которому её тронули.
    pub card_opacity: Option<f64>,
    /// Свой масштаб. `None` — тот же, что у всех, и по той же причине `Option`.
    ///
    /// Запись «масштаб своим не бывает» держалась на том, что окна разъедутся:
    /// размеры окон считаются от него, и виджет со своим числом встал бы не в
    /// свой ряд. Довод ослаб, когда размеры стали готовыми: окно берёт число из
    /// реестра и множит на СВОЙ масштаб, а не на общий, — разъехаться тут
    /// нечему, пока каждое окно помнит, при каком масштабе его мерили.
    pub scale: Option<f64>,
}

impl Default for WidgetSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            always_on_top: false,
            click_through: false,
            locked: false,
            view: 0,
            layout: 0,
            cover: 0,
            size: 0,
            card_opacity: None,
            scale: None,
        }
    }
}

impl Settings {
    pub fn appearance_for(&self, design: Design) -> &DesignAppearance {
        match design { Design::App => &self.appearance.app, Design::Reference => &self.appearance.reference }
    }

    pub fn appearance_for_mut(&mut self, design: Design) -> &mut DesignAppearance {
        match design { Design::App => &mut self.appearance.app, Design::Reference => &mut self.appearance.reference }
    }

    /// Масштаб, по которому живёт окно этого виджета: свой, а нет своего —
    /// общий.
    ///
    /// Одна дорога на все вопросы про масштаб: по этому числу считаются и
    /// размер окна, и его нижний предел, и `--scale` в самой карточке. Дай им
    /// разойтись — и окно встанет по одному числу, а содержимое по другому.
    pub fn scale_of(&self, widget: &Widget) -> f64 {
        sane_scale(self.widget(widget).scale.unwrap_or(self.scale))
    }

    /// Настройки виджета: сохранённые, а если их не было — из реестра.
    ///
    /// «Сквозь мышь» у нажимаемого виджета гасится прямо здесь, при чтении.
    /// В окне настроек такой галочки у него нет вовсе, но в файле она могла
    /// остаться с тех пор, когда виджет ещё только смотрели: погода стала
    /// нажимаемой, когда у неё появился почасовой прогноз по правой кнопке.
    /// Сохранённый флаг доехал бы до `windows::apply` мимо всяких проверок, и
    /// окно молча перестало бы ловить мышь — выглядит это как поломка виджета,
    /// а не как забытая настройка.
    pub fn widget(&self, widget: &Widget) -> WidgetSettings {
        let mut own = self
            .widgets
            .get(widget.id)
            .copied()
            .unwrap_or(WidgetSettings {
                enabled: widget.enabled,
                // Поверх всех — только по выбору человека: по умолчанию виджет
                // не закрывает собой окна программ (12 октября, слово пользователя)
                always_on_top: false,
                click_through: false,
                locked: false,
                view: 0,
                layout: usize::from(widget.id == "notes"),
                cover: 0,
                size: 0,
                card_opacity: None,
                scale: None,
            });

        if widget.interactive {
            own.click_through = false;
        }

        // Пределы меняются между версиями: своя плотность в 50 % была годной,
        // пока нижний предел стоял на 50. Приводим тут, на общей дороге чтения,
        // — иначе карточка и подпись ползунка показывали бы разные числа
        own.card_opacity = own.card_opacity.map(sane_opacity);
        own.scale = own.scale.map(sane_scale);

        /*
         * ВАЖНО: **номера из файла могут указывать в пустоту.**
         *
         * Виды, обложки и размеры хранятся номерами, а списки в реестре
         * меняются между версиями: у пульта было шесть видов и три размера,
         * стало три раскладки и четыре. Уцелевшая запись с видом номер пять
         * доехала бы до карточки как есть — та не нашла бы такой раскладки и
         * нарисовала бы запасную, а меню держало бы галочку в никуда.
         *
         * Чиним при чтении, а не при записи: запись проверяет себя и так, а
         * испортить файл может и правка руками.
         */
        if !widget.views.is_empty() && own.view >= widget.views.len() {
            own.view = 0;
        }
        if own.layout >= widget.layouts().len() {
            own.layout = 0;
        }
        if own.cover >= widget.covers.len() {
            own.cover = 0;
        }
        // Размеры — те, что видит нынешний дизайн: в Claude новые виды GPT не
        // действуют, и их единственный размер не должен обнулять выбранный
        if own.size >= widget.presets_in(self.design, own.view).len() {
            own.size = 0;
        }

        own
    }

    pub fn set_widget(&mut self, id: &str, value: WidgetSettings) {
        self.widgets.insert(id.to_owned(), value);
    }
}

/// Правит одну настройку виджета по имени столбца в окне настроек.
///
/// Отдельной функцией — ради теста: разбор строки с той стороны IPC приходит
/// от фронта, и незнакомое имя должно ничего не менять, а не совпасть с
/// соседним полем.
///
/// Столбца «Вкл.» тут нет: модуль включается своей командой
/// (`modules::set_module`), потому что вместе с флагом собирает или разбирает
/// окно, а это — только из пула, не из синхронной команды.
///
/// «Сквозь мышь» у нажимаемого виджета игнорируется: окно перестало бы ловить
/// нажатия, то есть виджет перестал бы работать целиком. В окне настроек такой
/// галочки у него нет вовсе, но настройка могла остаться в файле с прошлых
/// версий — включить её обратно всё равно нельзя.
pub fn apply_option(own: &mut WidgetSettings, option: &str, value: bool, interactive: bool) -> bool {
    match option {
        "top" => own.always_on_top = value,
        "through" if !interactive => own.click_through = value,
        "lock" => own.locked = value,
        _ => return false,
    }
    true
}

pub struct SettingsState(Mutex<Settings>);

impl SettingsState {
    pub fn new(settings: Settings) -> Self {
        Self(Mutex::new(settings))
    }

    pub fn get(&self) -> Settings {
        // Отравленный мьютекс не повод падать: данные внутри осмысленные
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Меняет настройки, не отпуская замка, и отдаёт получившееся.
    ///
    /// ВАЖНО: **правки «взять — поменять — положить обратно» тут быть не
    /// должно.** Между чтением и записью замок отпущен, и соседняя правка,
    /// уложившаяся в этот промежуток, пропадает целиком: обратно ложится копия,
    /// снятая до неё. Настройки — один объект на всё приложение, и пишут в него
    /// из окна настроек, из трея, из крестика на окне и из выбора города.
    ///
    /// Поэтому наружу отдаётся только этот способ правки, а «положить обратно»
    /// нет вовсе.
    pub fn update_with<F>(&self, change: F) -> Settings
    where
        F: FnOnce(&mut Settings),
    {
        let mut settings = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        change(&mut settings);
        settings.clone()
    }
}

fn file_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("не найден каталог настроек: {error}"))?;
    Ok(dir.join("settings.json"))
}

/// Читает настройки с диска. Любая беда — берём значения по умолчанию:
/// из-за настроек приложение стартовать не откажется.
pub fn load(app: &AppHandle) -> Settings {
    let Ok(path) = file_path(app) else {
        return Settings::default();
    };
    let Ok(text) = fs::read_to_string(&path) else {
        return Settings::default();
    };

    parse_settings(&text)
}

/// Разбор текста файла — отдельно от чтения ради тестов: они проверяют ровно
/// ту дорогу, по которой идёт настоящий файл.
///
/// Стекло и цвета режимов приводятся к годному ещё при разборе, в
/// `deserialize_*`, — здесь только то, что разбор пропускает как есть.
fn parse_settings(text: &str) -> Settings {
    // Новый ключ главнее прежнего даже при ошибочном значении. Alias дал бы
    // duplicate field при двух ключах и сбросил бы соседние настройки.
    let mut value = serde_json::from_str::<serde_json::Value>(text).unwrap_or_default();
    if let Some(object) = value.as_object_mut() {
        let legacy = object.remove("sound_style");
        if !object.contains_key("design") {
            if let Some(legacy) = legacy { object.insert("design".to_owned(), legacy); }
        }
    }
    if let Some(object) = value.as_object_mut() {
        let mut legacy = serde_json::Map::new();
        for key in ["palette", "glass", "accent_color", "custom_colors"] {
            if let Some(value) = object.remove(key) { legacy.insert(key.to_owned(), value); }
        }
        let profiles = object.entry("appearance").or_insert_with(|| serde_json::json!({}));
        if let Some(profiles) = profiles.as_object_mut() {
            // Новый профиль целиком главнее старых полей, включая ручные цвета.
            profiles.entry("app").or_insert(serde_json::Value::Object(legacy));
        }
    }
    let mut settings: Settings = serde_json::from_value(value).unwrap_or_default();
    if settings.sound_layout_version == 0 {
        if let Some(sound) = settings.widgets.get_mut("sound") {
            sound.view = if sound.view == 1 { 1 } else { 0 };
            sound.size = 0;
        }
        settings.sound_layout_version = 1;
    }
    if settings.hardware_layout_version == 0 {
        for id in ["cpu", "gpu"] {
            if let Some(own) = settings.widgets.get_mut(id) {
                own.size = match own.size { 2 => 1, 3 => 2, _ => 0 };
            }
        }
        settings.hardware_layout_version = 1;
    }
    if let Some(own) = settings.widgets.get_mut("spotify") { own.layout = 0; }
    for id in ["notes", "clipboard", "todo", "converter"] {
        if let Some(own) = settings.widgets.get_mut(id) { own.size = 0; }
    }
    settings.card_opacity = sane_opacity(settings.card_opacity);
    settings.scale = sane_scale(settings.scale);
    settings.vivid = sane_vivid(settings.vivid);
    settings.poll_seconds = sane_poll_seconds(settings.poll_seconds);
    settings.weather = sane_place(settings.weather);
    settings.app_icon = sane_app_icon(&settings.app_icon).to_owned();
    settings.output_dirs = sane_output_dirs(settings.output_dirs);
    settings.language = crate::i18n::sane(settings.language);
    settings.hidden_disks = sane_disks(settings.hidden_disks);
    settings.hidden_metrics = sane_metrics(settings.hidden_metrics);
    settings.sensor_picks = sane_picks(settings.sensor_picks);
    settings
}

/// Незнакомый идентификатор из вручную исправленного файла заменяется выбором
/// по умолчанию.
pub fn sane_app_icon(id: &str) -> &'static str {
    // Остались только два одобренных значка. Прежний белый/чёрно-белый выбор
    // сохраняет монохром, цветные и неизвестные id получают оранжевые плитки.
    match id {
        "mono-mosaic" | "mono-window" | "white-windows" => "mono-mosaic",
        _ => DEFAULT_APP_ICON,
    }
}

/// Снятые ряды: только у процессора и видеокарты, только известные, без
/// повторов. Пустой список выбрасывается — он значил бы то же, что его
/// отсутствие, и только путал бы при сравнении.
pub fn sane_metrics(hidden: BTreeMap<String, Vec<String>>) -> BTreeMap<String, Vec<String>> {
    hidden
        .into_iter()
        .filter(|(widget, _)| METRIC_WIDGETS.contains(&widget.as_str()))
        .filter_map(|(widget, metrics)| {
            let mut kept: Vec<String> = Vec::new();
            for metric in metrics {
                if METRICS.contains(&metric.as_str()) && !kept.contains(&metric) {
                    kept.push(metric);
                }
            }
            (!kept.is_empty()).then_some((widget, kept))
        })
        .collect()
}

/// Ручной выбор: известные места и значения, похожие на дело, — «не
/// показывать» или путь с хотя бы одной косой (железо/группа/датчик).
pub fn sane_picks(picks: BTreeMap<String, String>) -> BTreeMap<String, String> {
    picks
        .into_iter()
        .filter(|(slot, _)| SENSOR_SLOTS.contains(&slot.as_str()))
        .map(|(slot, pick)| (slot, pick.trim().to_owned()))
        .filter(|(_, pick)| pick == PICK_OFF || pick.contains('/'))
        .collect()
}

/// Скрытые накопители: без пустых имён и без повторов, порядок как был.
///
/// Файл правят и руками, а повтор имени значил бы, что одна галочка снимает
/// одну копию из двух и диск остаётся скрытым.
pub fn sane_disks(names: Vec<String>) -> Vec<String> {
    let mut kept: Vec<String> = Vec::with_capacity(names.len());
    for name in names {
        let name = name.trim().to_owned();
        if !name.is_empty() && !kept.contains(&name) {
            kept.push(name);
        }
    }
    kept
}

/// Своя папка вывода — только у известного модуля и только полным путём.
///
/// Относительный путь считался бы от текущего каталога процесса, а он у
/// повышенного приложения — `System32`: правленный руками файл увёл бы видео
/// туда.
pub fn sane_output_dirs(dirs: BTreeMap<String, String>) -> BTreeMap<String, String> {
    dirs.into_iter()
        .filter(|(module, dir)| {
            crate::tools::Module::from_id(module).is_some()
                && std::path::Path::new(dir).is_absolute()
        })
        .collect()
}

pub fn save(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let path = file_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("не создать каталог настроек: {error}"))?;
    }
    let text = serde_json::to_string_pretty(settings)
        .map_err(|error| format!("не сериализовать настройки: {error}"))?;
    // Через временного соседа: обрыв записи оставил бы огрызок, а он читается
    // как «настроек нет» — то есть выглядел бы самовольным сбросом
    crate::disk::write_atomically(&path, &text)
        .map_err(|error| format!("не записать настройки: {error}"))
}

/// Меняет настройки и сразу пишет их на диск.
pub fn update<F>(app: &AppHandle, state: &SettingsState, change: F) -> Settings
where
    F: FnOnce(&mut Settings),
{
    let settings = state.update_with(change);

    // Не записались — в памяти всё равно верно, приложение работает дальше
    if let Err(error) = save(app, &settings) {
        eprintln!("настройки не сохранились: {error}");
    }

    settings
}

/// Автозапуск — задача планировщика с наивысшими правами.
///
/// Ветка `Run` тут больше не годится: приложение просит права манифестом, и
/// запущенное оттуда оно спрашивало бы UAC на каждом входе в систему.
/// Подробности — в [`crate::autostart`].
///
/// Настройку меняем только вслед за успешной записью: иначе переключатель
/// показывал бы то, чего в системе нет. Отдаём то, что вышло на самом деле.
pub fn set_autostart(app: &AppHandle, next: bool) -> bool {
    let state = app.state::<SettingsState>();

    let outcome = if next {
        crate::autostart::enable()
    } else {
        crate::autostart::disable()
    };

    match outcome {
        Ok(()) => {
            update(app, &state, |settings| settings.autostart = next);
            next
        }
        Err(error) => {
            eprintln!("автозапуск не переключился: {error}");
            state.get().autostart
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets;

    /// Файл с прошлых этапов знает только про размытие. Он должен читаться,
    /// а недостающее — доставаться из реестра.
    ///
    /// Галочки «прозрачнее фон» больше нет: незнакомое поле просто
    /// пропускается, а плотность фона берётся по умолчанию.
    #[test]
    fn old_file_still_reads() {
        let settings: Settings =
            serde_json::from_str(r#"{"acrylic":true}"#).expect("старый файл читается");

        assert_eq!(settings.card_opacity, DEFAULT_CARD_OPACITY);
        assert_eq!(settings.scale, DEFAULT_SCALE);
        assert!(!settings.autostart);
        assert!(settings.widgets.is_empty());

        // Рубильники опущены, прилипание включено: файл без этих полей —
        // это поведение прошлых версий, а там окна прилипали всегда
        assert!(!settings.hidden_all);
        assert!(!settings.locked);
        assert!(settings.snapping);
        assert_eq!(settings.poll_seconds, DEFAULT_POLL_SECONDS);

        let gpu = widgets::find("gpu").expect("видеокарта в реестре");
        assert!(settings.widget(gpu).enabled, "видеокарта должна открыться");

        let debug = widgets::find("debug").expect("отладка в реестре");
        assert!(!settings.widget(debug).enabled, "отладка сама не открывается");
    }

    #[test]
    fn stored_value_wins_over_registry() {
        let mut settings = Settings::default();
        let gpu = widgets::find("gpu").expect("видеокарта в реестре");

        settings.set_widget(
            "gpu",
            WidgetSettings {
                enabled: false,
                always_on_top: false,
                click_through: true,
                locked: false,
                view: 0,
                layout: 0,
                cover: 0,
                size: 0,
                card_opacity: None,
                scale: None,
            },
        );

        let stored = settings.widget(gpu);
        assert!(!stored.enabled);
        assert!(!stored.always_on_top);
        assert!(stored.click_through);
    }

    /// Виджеты без записи в файле поверх всех не встают: окна программ они
    /// по умолчанию не закрывают (12 октября, слово пользователя).
    #[test]
    fn nothing_is_on_top_by_default() {
        let settings = Settings::default();

        let gpu = widgets::find("gpu").expect("видеокарта в реестре");
        assert!(!settings.widget(gpu).always_on_top);

        let debug = widgets::find("debug").expect("отладка в реестре");
        assert!(!settings.widget(debug).always_on_top);
    }

    #[test]
    fn survives_round_trip() {
        let mut settings = Settings {
            card_opacity: 1.0,
            scale: 1.15,
            appearance: DesignAppearances {
                app: DesignAppearance {
            palette: "monochrome".to_owned(),
            glass: "plum".to_owned(),
            accent_color: "#33BBEE".to_owned(),
            custom_colors: BTreeMap::from([
                ("accent".to_owned(), "#C49AFF".to_owned()),
                ("positive".to_owned(), "#55D9A1".to_owned()),
                ("caution".to_owned(), "#ECC565".to_owned()),
                ("negative".to_owned(), "#EE7788".to_owned()),
            ]),
                },
                reference: default_reference_appearance(),
            },
            vivid: 1.2,
            autostart: true,
            hidden_all: true,
            locked: true,
            snapping: false,
            poll_seconds: 3,
            weather: Some(Place {
                name: "Грац".to_owned(),
                region: "Австрия".to_owned(),
                latitude: 47.0707,
                longitude: 15.4395,
            }),
            sound_mixer_collapsed: true,
            design: Design::App,
            sound_layout_version: 1,
            hardware_layout_version: 1,
            widgets: BTreeMap::new(),
            output_dirs: BTreeMap::from([("downloader".to_owned(), "D:\\Downloads".to_owned())]),
            language: Some("uk".to_owned()),
            app_icon: "mono-mosaic".to_owned(),
            hidden_disks: vec!["Samsung SSD 970 EVO Plus 2TB".to_owned()],
            hidden_metrics: BTreeMap::from([("gpu".to_owned(), vec!["fan".to_owned(), "voltage".to_owned()])]),
            sensor_picks: BTreeMap::from([("chassisFan".to_owned(), "Board/Chip/Fans/Fan #2".to_owned())]),
            support_reminder: SupportReminder { first_run: 1_700_000_000, last_shown: Some(1_710_000_000), rare: true },
            update_auto: false,
            update_checked_at: Some(1_720_000_000),
        };
        settings.set_widget("spotify", WidgetSettings::default());

        let text = serde_json::to_string(&settings).expect("сериализуется");
        let back: Settings = serde_json::from_str(&text).expect("читается обратно");

        assert_eq!(back.card_opacity, 1.0);
        assert_eq!(back.scale, 1.15);
        assert_eq!(back.app_icon, "mono-mosaic");
        assert_eq!(back.appearance.app.palette, "monochrome");
        assert_eq!(back.appearance.app.glass, "plum");
        assert_eq!(back.appearance.app.accent_color, "#33BBEE");
        assert_eq!(back.appearance.app.custom_colors, settings.appearance.app.custom_colors);
        assert_eq!(back.vivid, 1.2);
        assert!(back.autostart);
        assert!(back.hidden_all);
        assert!(back.locked);
        assert!(!back.snapping);
        assert_eq!(back.poll_seconds, 3);
        assert_eq!(back.widgets.len(), 1);
        assert!(back.sound_mixer_collapsed);
        assert_eq!(back.design, Design::App);
        assert_eq!(back.sound_layout_version, 1);
        assert_eq!(back.language.as_deref(), Some("uk"));
        assert_eq!(back.hidden_disks, settings.hidden_disks);
        assert_eq!(back.hidden_metrics, settings.hidden_metrics);
        assert_eq!(back.sensor_picks, settings.sensor_picks);
        assert_eq!(back.support_reminder, settings.support_reminder);
        assert!(!back.update_auto);
        assert_eq!(back.update_checked_at, Some(1_720_000_000));

        let city = back.weather.expect("город пережил запись");
        assert_eq!(city.name, "Грац");
        assert_eq!(city.latitude, 47.0707);
    }

    /// Прежние B и C становятся мини-микшером; компактный пульт сохраняется.
    #[test]
    fn old_sound_layouts_migrate_once_without_losing_preferences() {
        let sound = widgets::find("sound").expect("звук в реестре");
        for (old, current) in [(0, 0), (1, 1), (2, 0)] {
            let text = format!(r#"{{"widgets":{{"sound":{{"view":{old},"size":1,"enabled":true,"scale":1.15}}}},"sound_mixer_collapsed":true}}"#);
            let settings = parse_settings(&text);
            let own = settings.widget(sound);
            assert_eq!(own.view, current);
            assert_eq!(own.size, 0);
            assert!(own.enabled);
            assert_eq!(own.scale, Some(1.15));
            assert!(settings.sound_mixer_collapsed);
            assert_eq!(settings.sound_layout_version, 1);
            assert_eq!(settings.design, Design::Reference);
            let restored = parse_settings(&serde_json::to_string(&settings).expect("сериализуется"));
            assert_eq!(restored.widget(sound).view, current);
        }
        let old = parse_settings(r#"{"widgets":{"sound":{"view":0}}}"#);
        assert!(!old.sound_mixer_collapsed);
        assert_eq!(old.widget(sound).view, 0);
    }

    /// Миграция звука не меняет современные настройки остальных модулей.
    #[test]
    fn sound_migration_preserves_other_widget_and_disk_preferences() {
        let mut settings = Settings::default();
        settings.sound_layout_version = 0;
        settings.hidden_disks.push("disk-for-sound-migration-test".to_owned());
        for widget in widgets::shown() {
            let mut own = settings.widget(widget);
            own.enabled = !own.enabled;
            own.always_on_top = false;
            own.scale = Some(1.1);
            own.card_opacity = Some(0.92);
            settings.set_widget(widget.id, own);
        }
        let restored = parse_settings(&serde_json::to_string(&settings).expect("сериализация настроек"));
        assert_eq!(restored.hidden_disks, settings.hidden_disks);
        for widget in widgets::shown().filter(|widget| widget.id != "sound") {
            assert_eq!(serde_json::to_value(restored.widget(widget)).expect("настройки модуля"),
                serde_json::to_value(settings.widget(widget)).expect("исходные настройки модуля"),
                "миграция звука изменила «{}»", widget.id);
        }
    }

    /// Версия отличает новый столбик от прежнего C с тем же номером.
    #[test]
    fn current_sound_layouts_and_styles_survive_a_round_trip() {
        let sound = widgets::find("sound").expect("звук в реестре");
        for style in [Design::App, Design::Reference] {
            for view in 0..3 {
                let mut settings = Settings { design: style, sound_mixer_collapsed: true, ..Settings::default() };
                settings.set_widget("sound", WidgetSettings { view, ..WidgetSettings::default() });
                let restored = parse_settings(&serde_json::to_string(&settings).expect("сериализуется"));
                assert_eq!(restored.widget(sound).view, view);
                assert_eq!(restored.widget(sound).size, 0);
                assert_eq!(restored.design, style);
                assert!(restored.sound_mixer_collapsed);
            }
        }
    }

    /// Ошибка только в оформлении не должна сбрасывать соседние настройки.
    #[test]
    fn invalid_sound_styles_fall_back_without_resetting_settings() {
        for value in [r#""unknown""#, "null", "false", "17", "{}"] {
            let settings = parse_settings(&format!(r#"{{"sound_style":{value},"autostart":true,"sound_mixer_collapsed":true}}"#));
            assert_eq!(settings.design, Design::Reference);
            assert!(settings.autostart);
            assert!(settings.sound_mixer_collapsed);
        }
        assert_eq!(Settings::default().sound_layout_version, 1);
        assert_eq!(serde_json::from_str::<Settings>("{}").expect("старый файл").sound_layout_version, 0);
        assert!(serde_json::from_str::<Design>(r#""unknown""#).is_err());
        assert_eq!(serde_json::to_string(&Design::App).expect("стиль сериализуется"), r#""app""#);
        assert_eq!(serde_json::to_string(&Design::Reference).expect("стиль сериализуется"), r#""reference""#);
    }

    #[test]
    fn hardware_sizes_migrate_once_after_no_history_removal() {
        for (old, current) in [(0, 0), (1, 0), (2, 1), (3, 2)] {
            let input = format!(r#"{{"widgets":{{"cpu":{{"size":{old},"view":1}},"gpu":{{"size":{old},"view":0}}}}}}"#);
            let first = parse_settings(&input);
            let again = parse_settings(&serde_json::to_string(&first).unwrap());
            assert_eq!(first.hardware_layout_version, 1);
            for id in ["cpu", "gpu"] {
                let widget = widgets::find(id).unwrap();
                assert_eq!(first.widget(widget).size, current);
                assert_eq!(again.widget(widget).size, current);
                assert_eq!(first.widget(widget).view, again.widget(widget).view);
            }
        }
    }

    /// Общий дизайн переносит прежний выбор звука, не трогая соседние поля.
    #[test]
    fn design_migrates_once_and_new_key_wins() {
        for (legacy, expected) in [("app", Design::App), ("reference", Design::Reference)] {
            let text = format!(r#"{{"sound_style":"{legacy}","sound_layout_version":1,"sound_mixer_collapsed":true,"autostart":true,"widgets":{{"sound":{{"view":2}},"cpu":{{"view":1,"size":3,"scale":1.1,"card_opacity":0.97}}}},"hidden_metrics":{{"cpu":["fan"]}}}}"#);
            let settings = parse_settings(&text);
            assert_eq!(settings.design, expected);
            assert!(settings.autostart && settings.sound_mixer_collapsed);
            assert_eq!(settings.widget(widgets::find("sound").unwrap()).view, 2);
            let before = serde_json::to_value(&settings).unwrap();
            assert!(before.get("sound_style").is_none());
            let restored = parse_settings(&serde_json::to_string(&settings).unwrap());
            assert_eq!(serde_json::to_value(restored).unwrap(), before);
        }
        for value in [r#""app""#, r#""reference""#, "null", "false", "17", "{}", r#""unknown""#] {
            let settings = parse_settings(&format!(r#"{{"design":{value},"sound_style":"app","autostart":true}}"#));
            assert_eq!(settings.design, if value == r#""app""# { Design::App } else { Design::Reference });
            assert!(settings.autostart);
        }
        assert_eq!(parse_settings("{}").design, Design::Reference);
    }

    #[test]
    fn invalid_sound_layout_and_removed_size_use_the_full_mixer() {
        let settings = parse_settings(r#"{"sound_layout_version":1,"sound_style":"app","sound_mixer_collapsed":true,"widgets":{"sound":{"view":99,"size":1}}}"#);
        let sound = widgets::find("sound").expect("звук в реестре");
        assert_eq!(settings.widget(sound).view, 0);
        assert_eq!(settings.widget(sound).size, 0);
        assert_eq!(settings.design, Design::App);
        assert!(settings.sound_mixer_collapsed);
    }

    /// Выбор мини-микшера и явное сворачивание приложений переживают запись.
    #[test]
    fn sound_mixer_preference_survives_view_and_size_changes() {
        let sound = widgets::find("sound").expect("звук в реестре");
        let mut settings = Settings { sound_mixer_collapsed: true, ..Settings::default() };
        for view in [2, 0, 1, 2] {
            for size in 0..sound.presets_for(view).len() {
                let mut own = settings.widget(sound);
                own.view = view;
                own.size = size;
                settings.set_widget(sound.id, own);
                let text = serde_json::to_string(&settings).expect("сериализуется");
                settings = parse_settings(&text);
                assert_eq!(settings.widget(sound).view, view);
                assert_eq!(settings.widget(sound).size, size);
                assert!(settings.sound_mixer_collapsed);
            }
        }
        settings.sound_mixer_collapsed = false;
        let text = serde_json::to_string(&settings).expect("сериализуется");
        assert!(!parse_settings(&text).sound_mixer_collapsed);
    }

    /// Файл прошлых версий языка не знает — и читается как «как в Windows», а
    /// незнакомый язык съезжает туда же, а не на первый по списку.
    #[test]
    fn language_defaults_to_the_system_one() {
        assert_eq!(parse_settings("{}").language, None);
        assert_eq!(parse_settings(r#"{"language":"zh"}"#).language.as_deref(), Some("zh"));
        assert_eq!(parse_settings(r#"{"language":"de"}"#).language, None);
    }

    /// Файл без города читается как «город не выбран», а не как нули: по нулям
    /// виджет ходил бы в Гвинейский залив и показывал бы тамошнюю погоду.
    #[test]
    fn old_file_without_a_city_reads() {
        let settings: Settings = serde_json::from_str("{}").expect("пустой файл читается");
        assert_eq!(settings.weather, None);
    }

    /// Правка файла руками не должна ронять опрос: испорченные координаты
    /// означают «города нет», а не запрос за край глобуса.
    #[test]
    fn coordinates_never_leave_the_globe() {
        let at = |latitude: f64, longitude: f64| {
            sane_place(Some(Place {
                name: "Где-то".to_owned(),
                region: String::new(),
                latitude,
                longitude,
            }))
        };

        let north = at(91.0, -400.0).expect("город остаётся, координаты зажимаются");
        assert_eq!(north.latitude, 90.0);
        assert_eq!(north.longitude, -180.0);

        assert_eq!(at(f64::NAN, 20.0), None);
        assert_eq!(at(52.0, f64::NAN), None);

        // Шестнадцать знаков от геокодера промахиваются мимо кеша Open-Meteo
        let rounded = at(47.070_675_431, 15.439_480_9).expect("город на месте");
        assert_eq!(rounded.latitude, 47.0707);
        assert_eq!(rounded.longitude, 15.4395);

        // Города без имени на карточке нечем подписать
        assert_eq!(
            sane_place(Some(Place {
                name: "   ".to_owned(),
                region: "Австрия".to_owned(),
                latitude: 47.0,
                longitude: 15.0,
            })),
            None
        );

        assert_eq!(sane_place(None), None);
    }

    /// Своя папка вывода выживает, только если она у известного модуля и
    /// задана полным путём.
    #[test]
    fn output_dirs_keep_only_known_modules_and_absolute_paths() {
        let settings = parse_settings(
            r#"{"output_dirs":{
                "converter":"D:\\Video\\Out",
                "downloader":"relative\\dir",
                "notes":"C:\\Elsewhere"
            }}"#,
        );
        assert_eq!(settings.output_dirs.len(), 1);
        assert_eq!(settings.output_dirs.get("converter").map(String::as_str), Some("D:\\Video\\Out"));
    }

    /// Старый файл скрытых накопителей не знает: показаны все. Правленный
    /// руками — без пустых имён и повторов, иначе одна галочка снимала бы одну
    /// копию из двух и диск оставался скрытым.
    #[test]
    fn hidden_metrics_read_clean() {
        assert!(parse_settings("{}").hidden_metrics.is_empty());

        let settings = parse_settings(
            r#"{"hidden_metrics":{"gpu":["fan","fan","bogus","voltage"],"cpu":[],"disks":["clock"]}}"#,
        );
        // Повтор и незнакомое — долой, пустой список и чужой виджет — тоже
        assert_eq!(
            settings.hidden_metrics,
            BTreeMap::from([("gpu".to_owned(), vec!["fan".to_owned(), "voltage".to_owned()])])
        );
    }

    #[test]
    fn sensor_picks_read_clean() {
        assert!(parse_settings("{}").sensor_picks.is_empty());

        let settings = parse_settings(
            r#"{"sensor_picks":{"cpuFan":" off ","chassisFan":"Board/Chip/Fans/Fan #2","boardTemp":"nonsense","gpuFan":"off"}}"#,
        );
        assert_eq!(
            settings.sensor_picks,
            BTreeMap::from([
                ("chassisFan".to_owned(), "Board/Chip/Fans/Fan #2".to_owned()),
                ("cpuFan".to_owned(), PICK_OFF.to_owned()),
            ])
        );
    }

    #[test]
    fn hidden_disks_read_clean() {
        assert!(parse_settings("{}").hidden_disks.is_empty());

        let settings = parse_settings(
            r#"{"hidden_disks":["Samsung SSD 980 PRO 1TB", " ", "Samsung SSD 980 PRO 1TB ", "WD Blue"]}"#,
        );
        assert_eq!(settings.hidden_disks, ["Samsung SSD 980 PRO 1TB", "WD Blue"]);
    }

    /// Мусор в файле не должен ронять приложение — читаем как «настроек нет».
    #[test]
    fn broken_file_falls_back_to_defaults() {
        let settings: Settings = serde_json::from_str("не json").unwrap_or_default();
        assert_eq!(settings.card_opacity, DEFAULT_CARD_OPACITY);
        assert!(settings.widgets.is_empty());
    }

    /// Значение из файла применяется к окнам без проверки со стороны меню:
    /// ноль или двойка испортили бы вид, а понять почему было бы неоткуда.
    #[test]
    fn opacity_stays_within_working_range() {
        assert_eq!(sane_opacity(0.0), MIN_CARD_OPACITY);
        assert_eq!(sane_opacity(2.0), 1.0);
        assert_eq!(sane_opacity(f64::NAN), DEFAULT_CARD_OPACITY);

        // Годное значение остаётся ровно таким, каким пришло
        assert_eq!(sane_opacity(0.9), 0.9);
        assert_eq!(sane_opacity(DEFAULT_CARD_OPACITY), DEFAULT_CARD_OPACITY);
    }

    /// Кегль вне пределов рассыпал бы разом все карточки, а понять почему
    /// было бы неоткуда: своих чисел у виджетов нет.
    #[test]
    fn scale_stays_within_working_range() {
        assert_eq!(sane_scale(0.1), MIN_SCALE);
        assert_eq!(sane_scale(4.0), MAX_SCALE);
        assert_eq!(sane_scale(f64::NAN), DEFAULT_SCALE);
        assert_eq!(sane_scale(1.15), 1.15);
    }

    /// Ноль в шаге опроса — это запросы без передышки, а не «побыстрее».
    #[test]
    fn poll_step_stays_within_working_range() {
        assert_eq!(sane_poll_seconds(0), MIN_POLL_SECONDS);
        assert_eq!(sane_poll_seconds(60), MAX_POLL_SECONDS);
        assert_eq!(sane_poll_seconds(3), 3);
    }

    /// Файл прошлой версии помнит адрес LHM, которого больше нет.
    /// Лишнее поле обязано пройти молча, а не уронить чтение настроек.
    #[test]
    fn a_file_with_the_old_lhm_address_still_reads() {
        let text = r#"{"card_opacity":0.9,"lhm_url":"http://127.0.0.1:8085/data.json","poll_seconds":2}"#;
        let settings: Settings = serde_json::from_str(text).expect("читается");
        assert_eq!(settings.poll_seconds, 2);
    }

    /// Правки из разных потоков не должны затирать друг друга.
    ///
    /// Настройки — один объект на всё приложение, и правит его кто угодно:
    /// окно настроек, трей, крестик на окне виджета, выбор города. При правке
    /// «взять — поменять — положить обратно» соседняя правка, уложившаяся
    /// между чтением и записью, пропадала бы целиком.
    #[test]
    fn parallel_changes_all_survive() {
        use std::sync::Arc;

        const HANDS: usize = 4;
        const EACH: usize = 50;

        let state = Arc::new(SettingsState::new(Settings::default()));

        let hands: Vec<_> = (0..HANDS)
            .map(|hand| {
                let state = Arc::clone(&state);
                std::thread::spawn(move || {
                    for step in 0..EACH {
                        state.update_with(|settings| {
                            settings.set_widget(
                                &format!("виджет-{hand}-{step}"),
                                WidgetSettings::default(),
                            );
                        });
                    }
                })
            })
            .collect();

        for hand in hands {
            hand.join().expect("поток не упал");
        }

        assert_eq!(
            state.get().widgets.len(),
            HANDS * EACH,
            "часть правок потерялась по дороге"
        );
    }

    /// Разбор имени столбца приходит от фронта: незнакомое имя обязано
    /// ничего не менять, а не попасть в соседнее поле.
    #[test]
    fn unknown_option_changes_nothing() {
        let mut own = WidgetSettings::default();
        let before = own;

        assert!(!apply_option(&mut own, "нет такого", true, false));
        assert_eq!(own.enabled, before.enabled);
        assert_eq!(own.always_on_top, before.always_on_top);
        assert_eq!(own.click_through, before.click_through);
    }

    /// Прозрачность для мыши отключает приём нажатий целиком: нажимаемому
    /// виджету её не включить ни из окна, ни правкой файла руками.
    #[test]
    fn clickable_widgets_never_take_click_through() {
        let mut own = WidgetSettings::default();

        assert!(!apply_option(&mut own, "through", true, true));
        assert!(!own.click_through);

        assert!(apply_option(&mut own, "through", true, false));
        assert!(own.click_through);
    }

    #[test]
    fn known_options_reach_their_fields() {
        let mut own = WidgetSettings::default();

        // Модуль так не включается: у него своя команда
        assert!(!apply_option(&mut own, "show", true, true));
        assert!(!own.enabled);

        assert!(apply_option(&mut own, "top", false, true));
        assert!(!own.always_on_top);

        assert!(apply_option(&mut own, "lock", true, true));
        assert!(own.locked);
    }

    /// «Закрепить» есть и у нажимаемого виджета: мышь оно не отнимает, а
    /// только запрещает двигать окно. Проверка `interactive` тут ни при чём,
    /// и легко было бы приписать её заодно со «сквозь мышь».
    #[test]
    fn locking_is_offered_to_clickable_widgets_too() {
        let mut own = WidgetSettings::default();

        assert!(apply_option(&mut own, "lock", true, true));
        assert!(own.locked);
    }

    /// Виджет мог быть несмотрибельным раньше, а стать нажимаемым позже —
    /// ровно это случилось с погодой. Сохранённая тогда прозрачность для мыши
    /// не должна доехать до окна: `windows::apply` ставит её, ни на что не
    /// глядя, и виджет молча перестал бы отзываться на нажатия.
    #[test]
    fn a_stored_click_through_dies_at_the_door_of_a_clickable_widget() {
        let mut settings = Settings::default();
        settings.set_widget(
            "weather",
            WidgetSettings {
                enabled: true,
                always_on_top: true,
                click_through: true,
                locked: false,
                view: 0,
                layout: 0,
                cover: 0,
                size: 0,
                card_opacity: None,
                scale: None,
            },
        );

        let weather = widgets::find("weather").expect("погода в реестре");
        assert!(weather.interactive, "погоду нажимают: там правая кнопка");
        assert!(
            !settings.widget(weather).click_through,
            "прозрачность для мыши дожила до нажимаемого виджета"
        );

        // У того, на кого только смотрят, настройка работает по-прежнему
        settings.set_widget(
            "gpu",
            WidgetSettings {
                enabled: true,
                always_on_top: true,
                click_through: true,
                locked: false,
                view: 0,
                layout: 0,
                cover: 0,
                size: 0,
                card_opacity: None,
                scale: None,
            },
        );
        let gpu = widgets::find("gpu").expect("видеокарта в реестре");
        assert!(settings.widget(gpu).click_through);
    }

    /// Прежние вариации переводятся в ближайшее по смыслу, а не в
    /// несуществующий селектор: окно с таким атрибутом осталось бы без цветов.
    #[test]
    fn old_palettes_migrate_to_the_nearest_theme() {
        assert_eq!(sane_palette("multi".to_owned()), default_palette());
        assert_eq!(sane_palette("accent".to_owned()), "single");
        assert_eq!(sane_palette("mono".to_owned()), "monochrome");
        // Темы клона, не пережившие перенос, съезжают на тему по умолчанию
        assert_eq!(sane_palette("arctic".to_owned()), default_palette());
        assert_eq!(sane_palette("review-amber".to_owned()), default_palette());
        assert_eq!(sane_palette("reference-mint".to_owned()), "reference-mint");
    }

    /// Файл прежней версии не знает ни стекла, ни своих цветов: они достаются
    /// по умолчанию, тоны ролей пропускаются молча, а остальное читается как
    /// было.
    #[test]
    fn old_files_gain_glass_and_colours_by_default() {
        let settings =
            parse_settings(r#"{"palette":"accent","scale":1.15,"hues":{"cpu":60.0},"pick":326.0}"#);
        assert_eq!(settings.appearance.app.palette, "single");
        assert_eq!(settings.scale, 1.15);
        assert_eq!(settings.appearance.app.glass, default_glass());
        assert_eq!(settings.appearance.app.accent_color, DEFAULT_ACCENT_COLOR);
        assert_eq!(settings.appearance.app.custom_colors, default_custom_colors());
    }

    /// Испорченное цветовое поле чинится на месте и не сбрасывает весь файл:
    /// город, автозапуск и окна пережили бы его только так.
    #[test]
    fn malformed_colours_do_not_reset_other_settings() {
        let settings = parse_settings(
            r##"{
                "palette":"custom", "glass":false, "accent_color":42,
                "scale":1.15, "autostart":true,
                "custom_colors":{
                    "accent":" #abcdef ", "positive":false,
                    "caution":"#bad", "negative":"var(--surface-card)",
                    "cpu":"#123456"
                }
            }"##,
        );
        assert_eq!(settings.appearance.app.palette, "custom");
        assert_eq!(settings.scale, 1.15);
        assert!(settings.autostart);
        assert_eq!(settings.appearance.app.glass, default_glass());
        assert_eq!(settings.appearance.app.accent_color, DEFAULT_ACCENT_COLOR);

        let mut expected = default_custom_colors();
        expected.insert("accent".to_owned(), "#ABCDEF".to_owned());
        assert_eq!(settings.appearance.app.custom_colors, expected);

        let settings = parse_settings(
            r##"{"palette":"single","glass":"unknown","accent_color":"#abcXYZ","custom_colors":null}"##,
        );
        assert_eq!(settings.appearance.app.palette, "single");
        assert_eq!(settings.appearance.app.glass, default_glass());
        assert_eq!(settings.appearance.app.accent_color, DEFAULT_ACCENT_COLOR);
        assert_eq!(settings.appearance.app.custom_colors, default_custom_colors());
    }

    /// Каждая тема и годные цвета переживают запись и чтение ровно такими,
    /// какими были.
    #[test]
    fn every_theme_survives_the_disk() {
        let mut original = parse_settings(
            r##"{"palette":"single","glass":"midnight","accent_color":"#34cde0","custom_colors":{"positive":"#78d8b1"}}"##,
        );
        original.appearance.reference = original.appearance.app.clone();
        for palette in PALETTES {
            original.appearance.reference.palette = (*palette).to_owned();
            let text = serde_json::to_string(&original).expect("настройки записываются");
            let restored = parse_settings(&text);
            assert_eq!(restored.appearance.reference.palette, *palette);
            assert_eq!(restored.appearance.reference.glass, "midnight");
            assert_eq!(restored.appearance.reference.accent_color, "#34CDE0");
            assert_eq!(restored.appearance.reference.custom_colors["positive"], "#78D8B1");
            assert_eq!(restored.appearance.reference.custom_colors.len(), CUSTOM_COLORS.len());
        }
    }


    #[test]
    fn design_profiles_migrate_once_and_remain_independent() {
        let mut settings = parse_settings(r##"{"palette":"reference-a","glass":"plum","accent_color":"#123456","custom_colors":{"positive":"#ABCDEF"},"vivid":1.5}"##);
        assert_eq!(settings.appearance.app.palette, "reference-a");
        assert_eq!(settings.appearance.app.glass, "plum");
        assert_eq!(settings.appearance.app.accent_color, "#123456");
        assert_eq!(settings.appearance.app.custom_colors["positive"], "#ABCDEF");
        assert_eq!(settings.appearance.reference.palette, "gpt");
        assert_eq!(settings.appearance.reference.glass, "gpt");
        assert_eq!(settings.vivid, 1.5);
        settings.appearance_for_mut(Design::Reference).glass = "warm".to_owned();
        settings.appearance_for_mut(Design::Reference).palette = "custom".to_owned();
        settings.appearance_for_mut(Design::Reference).custom_colors.insert("accent".to_owned(), "#654321".to_owned());
        let text = serde_json::to_string(&settings).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        for key in ["palette", "glass", "accent_color", "custom_colors"] { assert!(value.get(key).is_none()); }
        let restored = parse_settings(&text);
        assert_eq!(restored.appearance, settings.appearance);
        assert_eq!(restored.vivid, 1.5);
        let preferred = parse_settings(r##"{"palette":"single","glass":"warm","appearance":{"app":{"palette":"reference-b","glass":"midnight"},"reference":{"palette":"gpt","glass":"gpt"}}}"##);
        assert_eq!(preferred.appearance.app.palette, "reference-b");
        assert_eq!(preferred.appearance.app.glass, "midnight");
    }


    #[test]
    fn removed_presets_and_spotify_b_normalize_without_changing_other_options() {
        let settings = parse_settings(r#"{"widgets":{"notes":{"size":1,"layout":0,"scale":1.3},"clipboard":{"size":1},"todo":{"size":1,"layout":1},"converter":{"size":2,"scale":1.1,"locked":true},"spotify":{"layout":1,"view":2,"size":3,"cover":1}}}"#);
        for id in ["notes", "clipboard", "todo", "converter"] { assert_eq!(settings.widgets[id].size, 0); }
        assert_eq!(settings.widgets["notes"].layout, 0);
        assert_eq!(settings.widgets["notes"].scale, Some(1.3));
        assert_eq!(settings.widgets["todo"].layout, 1);
        assert_eq!(settings.widgets["converter"].scale, Some(1.1));
        assert!(settings.widgets["converter"].locked);
        assert_eq!(settings.widgets["spotify"].layout, 0);
        assert_eq!(settings.widgets["spotify"].view, 2);
        assert_eq!(settings.widgets["spotify"].size, 3);
        assert_eq!(settings.widgets["spotify"].cover, 1);
        assert_eq!(Settings::default().widget(crate::widgets::find("notes").unwrap()).layout, 1);
    }
    #[test]
    fn delayed_profile_edit_does_not_touch_the_active_design() {
        let mut settings = Settings::default();
        settings.design = Design::Reference;
        let active = settings.appearance.reference.clone();
        settings.appearance_for_mut(Design::App).palette = palette_for_design("single".to_owned(), Design::App);
        assert_eq!(settings.appearance.reference, active);
        assert_eq!(palette_for_design("gpt".to_owned(), Design::App), default_palette());
        let malformed = parse_settings(r#"{"scale":1.15,"appearance":{"app":false,"reference":{"glass":42,"palette":false}}}"#);
        assert_eq!(malformed.scale, 1.15);
        assert_eq!(malformed.appearance.reference, DesignAppearance::for_design(Design::Reference));
    }
    /// Яркость ниже нынешнего предела могла остаться от прежней насыщенности
    /// и не должна возвращать выцветший вид.
    #[test]
    fn vivid_stays_within_working_range() {
        assert_eq!(sane_vivid(0.3), MIN_VIVID);
        assert_eq!(sane_vivid(f64::NEG_INFINITY), MIN_VIVID);
        assert_eq!(sane_vivid(2.0), MAX_VIVID);
        assert_eq!(sane_vivid(f64::NAN), DEFAULT_VIVID);
        // Годное значение остаётся точным, а не округляется под шаг ползунка
        assert_eq!(sane_vivid(1.23), 1.23);
    }

    /// Своё число виджета могло сохраниться ниже нынешнего предела. Карточка и
    /// ползунок обязаны увидеть одно и то же, а не 86 % в карточке и 50 % в
    /// подписи настроек.
    #[test]
    fn stored_widget_appearance_uses_current_limits() {
        let mut settings = Settings::default();
        settings.set_widget(
            "gpu",
            WidgetSettings {
                card_opacity: Some(0.5),
                scale: Some(0.1),
                ..WidgetSettings::default()
            },
        );

        let gpu = widgets::find("gpu").expect("видеокарта в реестре");
        let own = settings.widget(gpu);
        assert_eq!(own.card_opacity, Some(MIN_CARD_OPACITY));
        assert_eq!(own.scale, Some(MIN_SCALE));
    }
}


#[cfg(test)]
mod redesign_layout_tests {
    use super::*;
    use crate::widgets;

    #[test]
    fn layout_migrates_from_absence_and_survives_save_and_restore() {
        let widget = widgets::find("speedtest").unwrap();
        let legacy: Settings = serde_json::from_str(r#"{"widgets":{"speedtest":{"view":1,"size":1}}}"#).unwrap();
        let mut own = legacy.widget(widget);
        assert_eq!((own.view, own.size, own.layout), (1, 1, 0));
        let mut settings = legacy;
        own.layout = 1;
        settings.set_widget(widget.id, own);
        let serialized = serde_json::to_string(&settings).unwrap();
        let restored: Settings = serde_json::from_str(&serialized).unwrap();
        let saved = restored.widget(widget);
        assert_eq!((saved.view, saved.size, saved.layout), (1, 1, 1));
        assert_eq!(saved.enabled, own.enabled);
        own.layout = 999;
        settings.set_widget(widget.id, own);
        assert_eq!(settings.widget(widget).layout, 0);
    }

    #[test]
    fn removed_slate_uses_the_renamed_gpt_surface() {
        assert!(!GLASSES.contains(&"slate"));
        assert_eq!(sane_glass("slate".to_owned()), "gpt");
        let settings = parse_settings(r#"{"glass":"slate","appearance":{"reference":{"glass":"slate"}}}"#);
        assert_eq!(settings.appearance.app.glass, "gpt");
        assert_eq!(settings.appearance.reference.glass, "gpt");
        let saved = serde_json::to_string(&settings).unwrap();
        assert!(!saved.contains("slate"));
    }

}
