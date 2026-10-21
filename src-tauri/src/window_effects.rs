use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use window_vibrancy::clear_blur;

use crate::settings::{self, SettingsState};

/// Событие смены вида: по нему каждое окно правит у корня документа плотность
/// фона, масштаб, тему, стекло и яркость цветов.
///
/// Значения едут вместе одним снимком: ими двигают ползунки и кнопки одного
/// раздела настроек, и отдельное событие на каждое завело бы несколько
/// источников правды на одно окно.
const APPEARANCE_EVENT: &str = "appearance";

/// Вид одного окна.
///
/// Плотность фона и масштаб бывают своими у виджета, поэтому общего снимка на
/// все окна не существует — каждому отвечают своим. Тема, стекло и цвета
/// режимов общие.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Appearance {
    /// Сохранённый общий выбор; настройки показывают его даже до своего редизайна.
    pub design: settings::Design,
    /// Временный возврат к Claude у ещё не переработанного окна.
    pub effective_design: settings::Design,
    pub card_opacity: f64,
    pub scale: f64,
    /// Цветовая тема. Общая на все окна: своей у виджета она не бывает.
    pub palette: String,
    /// Стекло — фон карточек. От темы не зависит.
    pub glass: String,
    /// Цвет режима «Один акцент».
    pub accent_color: String,
    /// Четыре базы режима «Своя палитра».
    pub custom_colors: std::collections::BTreeMap<String, String>,
    /// Яркость цветов: светлота и цветность баз разом.
    pub vivid: f64,
}

/// Вид, каким его видит окно с этой меткой.
///
/// Метка окна виджета — это его `id` в реестре, поэтому своя плотность
/// находится прямым поиском. У окон, которых в реестре нет (настройки, витрина,
/// отладка), своей плотности не бывает вовсе.
fn effective(settings: &settings::Settings, label: &str) -> Appearance {
    let (card_opacity, scale) = match crate::widgets::find(label) {
        Some(widget) => (
            settings
                .widget(widget)
                .card_opacity
                .unwrap_or(settings.card_opacity),
            settings.scale_of(widget),
        ),
        // Окно не виджета: настройки, витрина, отладка — своего вида у них нет
        None => (settings.card_opacity, settings.scale),
    };

    let effective_design = effective_design(settings, label);
    let profile = settings.appearance_for(effective_design);
    Appearance {
        design: settings.design,
        effective_design,
        card_opacity: settings::sane_opacity(card_opacity),
        scale: settings::sane_scale(scale),
        palette: profile.palette.clone(),
        glass: profile.glass.clone(),
        accent_color: profile.accent_color.clone(),
        custom_colors: profile.custom_colors.clone(),
        vivid: settings.vivid,
    }
}

/// Дизайн, которым окно рисует прямо сейчас: выбранный, а у неготового окна —
/// Claude. Им же сверяются запросы геометрии, пришедшие от окна.
pub fn effective_design(settings: &settings::Settings, label: &str) -> settings::Design {
    if settings.design == settings::Design::Reference && !new_design_ready(label) {
        settings::Design::App
    } else { settings.design }
}

/// Готовность — свойство реализации; выбор пользователя при возврате не меняется.
///
/// Карточка напоминания о донатах (`support-reminder`) собрана на общих
/// токенах и идёт за выбранным дизайном с самого начала.
fn new_design_ready(label: &str) -> bool {
    matches!(label, "cpu" | "gpu" | "sound" | "processes" | "disks" | "network" | "speedtest" | "weather" | "media" | "spotify" | "clipboard" | "notes" | "todo" | "mail" | "password" | "totp" | "converter" | "downloader" | "settings" | crate::support_reminder::LABEL)
}

/// Смена дизайна не пересоздаёт окна и не меняет их виды, размеры и панели.
#[tauri::command]
pub async fn set_design(app: AppHandle, window: WebviewWindow, design: settings::Design) -> Appearance {
    let state = app.state::<SettingsState>();
    let changed = state.get().design != design;
    if changed { settings::update(&app, &state, |settings| settings.design = design); }
    let label = window.label().to_owned();
    let reply_label = label.clone();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    crate::windows::on_main(&app, move |app| {
        if changed { crate::windows::sync_design_geometry(&app); }
        broadcast(&app);
        crate::settings_window::emit(&app);
        let settings = app.state::<SettingsState>().get();
        let _ = sender.send(effective(&settings, &reply_label));
    });
    receiver.await.unwrap_or_else(|_| effective(&state.get(), &label))
}

/// Рассылает вид каждому окну — своё каждому.
///
/// Именно поштучно, а не одним `emit` на всех: с тех пор как у виджета бывает
/// своя плотность, одинакового снимка на все окна больше не существует. Общий
/// `emit` внутри всё равно обходит окна по одному, так что дороже это не стало.
fn broadcast(app: &AppHandle) {
    let settings = app.state::<SettingsState>().get();

    for label in app.webview_windows().into_keys() {
        let _ = app.emit_to(&label, APPEARANCE_EVENT, effective(&settings, &label));
    }
}

/// Снимает с окна системную подложку.
///
/// Настоящего размытия обоев под виджетом добиться не вышло, и это проверено
/// обоими доступными способами на живых окнах:
///
/// - `apply_acrylic` на Windows 11 уходит в `DWMWA_SYSTEMBACKDROP_TYPE`.
///   Атрибут выставляется, но под прозрачным окном ничего не рисует, а цвет
///   тонировки не принимает вовсе;
/// - `apply_blur` идёт через `SetWindowCompositionAttribute` и размывает,
///   но делает виджет чёрным и мерцающим при перетаскивании.
///
/// Поэтому подложку снимаем всегда, а плотность фона задаёт CSS. Рамка и
/// заголовок окна — отдельная история, их держит `native_frame.rs`.
pub fn apply_to(window: &WebviewWindow) -> Result<(), String> {
    // Именно `clear_blur`: он сбрасывает политику композиции через
    // `SetWindowCompositionAttribute`, а не только атрибут фона DWM
    clear_blur(window).map_err(|error| format!("не удалось снять подложку: {error}"))
}

/// Живое изменение: применяется во всех окнах, но дальше памяти не идёт.
///
/// В нативный слой тут идти не за чем — фон и кегль рисует карточка, а не
/// окно. Ползунок зовёт это десятки раз в секунду, поэтому записи на диск
/// здесь нет: она стоила бы дороже самой отрисовки.
pub fn preview(app: &AppHandle, card_opacity: f64, scale: f64, vivid: f64) {
    let card_opacity = settings::sane_opacity(card_opacity);
    let scale = settings::sane_scale(scale);
    let vivid = settings::sane_vivid(vivid);

    app.state::<SettingsState>().update_with(|settings| {
        settings.card_opacity = card_opacity;
        settings.scale = scale;
        settings.vivid = vivid;
    });

    // Общий ползунок двигает всех, у кого своей плотности нет: у кого есть —
    // тот получит своё же число и не шелохнётся
    broadcast(app);
}

/// То же для одного виджета: свои плотность фона и масштаб, начерно.
///
/// `None` — «как у всех»: виджет возвращается под общий ползунок. Именно
/// стирание, а не запись нынешнего общего числа — записанное, оно перестало бы
/// слушать общий ползунок навсегда.
///
/// Правка идёт одним замыканием над своими настройками: два ползунка, два
/// вызова, а ветка правки одна — ровно как у галочек в списке виджетов.
fn preview_widget(app: &AppHandle, id: &str, change: impl FnOnce(&mut settings::WidgetSettings)) {
    let Some(widget) = crate::widgets::find(id) else {
        return;
    };

    app.state::<SettingsState>().update_with(|settings| {
        settings.edit_widget(widget, |own| {
            change(own);
            true
        });
    });

    // Только тому окну, которого касается: остальным пересчитывать нечего
    let settings = app.state::<SettingsState>().get();
    let _ = app.emit_to(widget.id, APPEARANCE_EVENT, effective(&settings, widget.id));
}

pub fn preview_widget_opacity(app: &AppHandle, id: &str, opacity: Option<f64>) {
    preview_widget(app, id, |own| {
        own.card_opacity = opacity.map(settings::sane_opacity);
    });
}

/// Свой масштаб виджета, начерно.
///
/// Размер окна при этом не трогаем: он подтянется в [`commit`], когда ползунок
/// отпустят. Тот приходит десятки раз в секунду, и окно прыгало бы под мышью —
/// то же правило, что у общего ползунка.
pub fn preview_widget_scale(app: &AppHandle, id: &str, scale: Option<f64>) {
    preview_widget(app, id, |own| {
        own.scale = scale.map(settings::sane_scale);
    });
}

/// Закрепляет то, что накрутили ползунки: пишет настройки на диск и
/// подтягивает размеры окон под новый кегль.
///
/// Зовётся один раз — когда ползунок отпустили. Пересчёт окон именно здесь, а
/// не в [`preview`]: тот приходит десятки раз в секунду, и окна прыгали бы
/// под мышью.
pub fn commit(app: &AppHandle) {
    let state = app.state::<SettingsState>();

    // Значение уже в памяти — остаётся записать его
    settings::update(app, &state, |_| {});

    // Размерами и стилями окон ведает главный поток. Число ему не передаём:
    // масштаб бывает свой у виджета, и одного числа на все окна больше нет
    crate::windows::on_main(app, |app| crate::windows::rescale(&app));

    crate::settings_window::emit(app);
}

/// Вид при открытии окна — спрашивает каждое, и ответ у каждого свой.
///
/// Окно тут не для порядка: плотность фона бывает своя у виджета, и общего
/// ответа на этот вопрос больше нет.
#[tauri::command]
pub fn appearance(window: WebviewWindow, state: State<'_, SettingsState>) -> Appearance {
    effective(&state.get(), window.label())
}

/// Своя плотность фона виджета, начерно. `None` — вернуть под общий ползунок.
#[tauri::command]
pub fn preview_widget_appearance(app: AppHandle, id: String, card_opacity: Option<f64>) {
    preview_widget_opacity(&app, &id, card_opacity);
}

/// Свой масштаб виджета, начерно. `None` — вернуть под общий ползунок.
///
/// Отдельной командой, а не вторым полем к плотности: ползунки живут порознь,
/// и одна команда на оба требовала бы `Option<Option<f64>>` — «не трогай» и
/// «сотри» пришлось бы различать пустотой и `null`, что читается только с
/// комментарием.
#[tauri::command]
pub fn preview_widget_scale_appearance(app: AppHandle, id: String, scale: Option<f64>) {
    preview_widget_scale(&app, &id, scale);
}

/// Приходят только числа, тронутые ползунком: остальные берутся нынешние, а
/// не те, что окно видело в последнем событии. Иначе ползунок плотности,
/// тронутый сразу после яркости, вернул бы яркость назад прежним числом из
/// ещё не дошедшего события.
#[tauri::command]
pub fn preview_appearance(app: AppHandle, card_opacity: Option<f64>, scale: Option<f64>, vivid: Option<f64>) {
    let now = app.state::<SettingsState>().get();
    preview(
        &app,
        card_opacity.unwrap_or(now.card_opacity),
        scale.unwrap_or(now.scale),
        vivid.unwrap_or(now.vivid),
    );
}

/// Цветовая тема. Общая на все окна, рассылка тоже общая.
///
/// Без пары «начерно — начисто»: её выбирают нажатием, а не тянут ползунком, и
/// промежуточных значений у неё нет вовсе. Пишем сразу.
///
/// Снимок настроек не рассылаем: выбранную тему окно настроек знает из того же
/// события вида, что и все остальные окна. То же у стекла и цветов режимов.
#[tauri::command]
pub fn set_palette(app: AppHandle, design: settings::Design, name: String) {
    let state = app.state::<SettingsState>();
    settings::update(&app, &state, |settings| {
        settings.appearance_for_mut(design).palette = settings::palette_for_design(name, design);
    });

    broadcast(&app);
}

/// Стекло. Тоже нажатием и тоже сразу; тему не трогает — фон и цвета
/// показателей выбираются независимо.
#[tauri::command]
pub fn set_glass(app: AppHandle, design: settings::Design, name: String) {
    let state = app.state::<SettingsState>();
    settings::update(&app, &state, |settings| {
        settings.appearance_for_mut(design).glass = settings::sane_glass(name);
    });

    broadcast(&app);
}

/// Цвет режима «Один акцент».
///
/// Запись на каждую правку тут не дорога: поле HEX в настройках придерживает
/// правки, пока цвет тянут пипеткой, и зовёт это не чаще раза в сто двадцать
/// миллисекунд.
#[tauri::command]
pub fn set_accent_color(app: AppHandle, design: settings::Design, color: String) {
    let state = app.state::<SettingsState>();
    settings::update(&app, &state, |settings| {
        settings.appearance_for_mut(design).accent_color = settings::sane_color(&color, settings::DEFAULT_ACCENT_COLOR);
    });

    broadcast(&app);
}

/// Одна база «Своей палитры». Незнакомая роль не меняет ничего: имя приходит
/// от фронта и не должно завести в файле пятую базу.
#[tauri::command]
pub fn set_custom_color(app: AppHandle, design: settings::Design, role: String, color: String) {
    let Some((_, fallback)) = settings::CUSTOM_COLORS.iter().find(|(id, _)| *id == role) else {
        return;
    };
    let color = settings::sane_color(&color, fallback);

    let state = app.state::<SettingsState>();
    settings::update(&app, &state, |settings| {
        settings.appearance_for_mut(design).custom_colors.insert(role, color);
    });

    broadcast(&app);
}

/// Все четыре базы «Своей палитры» — обратно к умолчанию.
#[tauri::command]
pub fn reset_custom_colors(app: AppHandle, design: settings::Design) {
    let state = app.state::<SettingsState>();
    settings::update(&app, &state, |settings| {
        settings.appearance_for_mut(design).custom_colors = settings::default_custom_colors();
    });

    broadcast(&app);
}

#[tauri::command]
pub fn commit_appearance(app: AppHandle) {
    commit(&app);
}

#[cfg(test)]
mod tests {
    use crate::settings;

    #[test]
    fn design_fallback_does_not_overwrite_the_global_choice_or_widget_state() {
        let mut settings = settings::Settings::default();
        settings.set_widget("cpu", settings::WidgetSettings { view: 1, size: 2, scale: Some(1.3), card_opacity: Some(0.98), ..settings::WidgetSettings::default() });
        settings.sound_mixer_collapsed = true;
        let before = serde_json::to_value(&settings).unwrap();
        for design in [settings::Design::Reference, settings::Design::App, settings::Design::Reference] {
            settings.design = design;
            for widget in crate::widgets::WIDGETS {
                let appearance = super::effective(&settings, widget.id);
                assert_eq!(appearance.design, design);
                let expected = if super::new_design_ready(widget.id) { design } else { settings::Design::App };
                assert_eq!(appearance.effective_design, expected);
            }
            let appearance = super::effective(&settings, "settings");
            assert_eq!(appearance.design, design);
            assert_eq!(appearance.effective_design, design);
            assert_eq!(super::effective(&settings, "debug").effective_design, settings::Design::App);
        }
        assert_eq!(serde_json::to_value(&settings).unwrap(), before);
        let appearance = super::effective(&settings, "cpu");
        assert_eq!(appearance.scale, 1.3);
        assert_eq!(appearance.card_opacity, 0.98);
    }

    #[test]
    fn processes_design_tracks_the_global_choice_without_changing_its_settings() {
        let mut settings = settings::Settings::default();
        settings.set_widget("processes", settings::WidgetSettings { scale: Some(1.3), card_opacity: Some(0.98), ..settings::WidgetSettings::default() });
        let own = serde_json::to_value(settings.widget(crate::widgets::find("processes").unwrap())).unwrap();
        for design in [settings::Design::Reference, settings::Design::App, settings::Design::Reference] {
            settings.design = design;
            let appearance = super::effective(&settings, "processes");
            assert_eq!(appearance.effective_design, design);
            assert_eq!(appearance.scale, 1.3);
            assert_eq!(appearance.card_opacity, 0.98);
            assert_eq!(super::effective(&settings, "debug").effective_design, settings::Design::App);
            assert_eq!(serde_json::to_value(settings.widget(crate::widgets::find("processes").unwrap())).unwrap(), own);
        }
    }

    /// Шкала кеглей включена в сборку строкой: своё число в обход множителя
    /// ползунок не заметит, и поймать это можно будет только глазами.
    const TOKENS: &str = include_str!("../../src/styles/tokens.css");

    #[test]
    fn every_text_size_goes_through_the_scale() {
        // Цвета текста называются так же (`--text-muted`), но точек в них нет
        let sizes: Vec<&str> = TOKENS
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with("--text-") && line.contains("px"))
            .collect();

        assert!(sizes.len() >= 8, "шкала кеглей поредела: {}", sizes.len());

        for line in sizes {
            assert!(
                line.contains("var(--scale)"),
                "кегль мимо множителя: {line}"
            );
        }
    }

    /// То же для геометрии: отступы и толщины едут за тем же самым
    /// множителем, и число в обход него ползунок точно так же не заметит.
    ///
    /// Множитель у геометрии был свой и шёл вполовину хода кегля — пока
    /// ползунок звался «размером шрифта». «Масштаб» — это полный ход по
    /// определению, и второй множитель исчез вместе с поводом.
    ///
    /// Скругления сюда не входят намеренно: углы окна рисует DWM своим числом,
    /// а у карточки в окне радиус и вовсе ноль.
    #[test]
    fn every_geometry_size_goes_through_the_scale() {
        const GEOMETRY: [&str; 6] = [
            "--space-",
            "--bar-thick",
            "--bar-thin",
            "--columns-gap",
            "--stroke-line",
            "--dot-size",
        ];

        let sizes: Vec<&str> = TOKENS
            .lines()
            .map(str::trim)
            .filter(|line| {
                line.contains("px") && GEOMETRY.iter().any(|name| line.starts_with(name))
            })
            .collect();

        assert!(
            sizes.len() >= 12,
            "геометрия поредела: {} строк",
            sizes.len()
        );

        for line in sizes {
            assert!(
                line.contains("var(--scale)"),
                "геометрия мимо множителя: {line}"
            );
        }
    }

    /// У каждой темы есть свой блок в токенах, и он задаёт все четыре базы.
    ///
    /// Список тем живёт в Rust, а цвета — в CSS, и разойтись им было бы
    /// вопросом времени: тема без блока молча рисовала бы чужими цветами, а
    /// блок, забывший базу, — цветом прошлой темы посреди своих. Стекло тема
    /// не трогает: оно выбирается отдельно.
    #[test]
    fn every_palette_has_its_own_four_colours() {
        for id in settings::PALETTES {
            let own = own(id);
            assert!(!own.is_empty(), "тема «{id}» без блока в токенах");

            for (role, _) in settings::CUSTOM_COLORS {
                let name = format!("--seed-{role}:");
                assert!(
                    declared(&own, &name).is_some(),
                    "тема «{id}» не задаёт {name} — там останется чужой цвет"
                );
            }

            assert!(
                TOKENS.contains(&format!(":root[data-palette=\"{id}\"]")),
                "тема «{id}» не применяется к корню окна"
            );
            assert!(
                own.iter().all(|body| !body.contains("--surface-")),
                "тема «{id}» меняет стекло, хотя оно выбирается отдельно"
            );
        }
    }

    /// Окно, до которого настройки ещё не доехали, атрибута темы не имеет и
    /// рисует первой темой из списка.
    ///
    /// Объявлена она для него через `:not([data-palette])`, а не на голом
    /// `:root`: тема переназначает роли метрик, и на голом корне её роли
    /// действовали бы при любом атрибуте — протекли бы в режимы.
    #[test]
    fn the_first_palette_paints_a_window_without_settings() {
        let first = settings::PALETTES[0];
        let lines: Vec<&str> = TOKENS.lines().map(str::trim).collect();

        let default = lines
            .iter()
            .position(|line| *line == ":root:not([data-palette]),")
            .expect("тема по умолчанию не объявлена для окна без атрибута");
        assert_eq!(
            lines[default + 1],
            format!(":root[data-palette=\"{first}\"],"),
            "окно без атрибута рисует не первой темой из списка"
        );

        for (index, line) in lines.iter().enumerate() {
            if *line == ":root," {
                assert!(
                    !lines[index + 1].starts_with(":root[data-palette="),
                    "тема объявлена на голом :root — её роли протекут в остальные"
                );
            }
        }
    }

    /// Где есть градация, там разные цвета — главное правило палитры.
    ///
    /// Процессор с памятью стоят в одной карточке процессов, видеокарта с
    /// видеопамятью — в карточке железа, приём с отдачей — в карточке сети;
    /// разряды пароля, ступени состояния и солнце с дождём идут рядом. Ясное
    /// небо и гроза светят карточку погоды тёплой базой и цветом «плохо», и
    /// гроза, засветившая солнцем, потерялась бы.
    ///
    /// Сравниваются не имена ролей, а HEX баз, до которых они разворачиваются:
    /// две базы с одним и тем же HEX на глаз — один цвет.
    ///
    /// «Своя палитра» не проверяется: её базы выбирает человек.
    #[test]
    fn pairs_differ_in_every_theme() {
        const PAIRS: [(&str, &str); 11] = [
            ("--metric-cpu", "--metric-ram"),
            ("--metric-gpu", "--metric-ram"),
            ("--metric-net-down", "--metric-net-up"),
            ("--char-upper", "--char-digit"),
            ("--char-upper", "--char-symbol"),
            ("--char-digit", "--char-symbol"),
            ("--status-ok", "--status-warn"),
            ("--status-ok", "--status-danger"),
            ("--status-warn", "--status-danger"),
            ("--sky-warm", "--sky-water"),
            ("--sky-warm", "--status-danger"),
        ];

        for id in settings::PALETTES {
            if *id == "custom" {
                continue;
            }

            let scope = scope(id);
            for (one, other) in PAIRS {
                let first = base_of(&scope, one);
                let second = base_of(&scope, other);
                assert_ne!(
                    first.to_ascii_lowercase(),
                    second.to_ascii_lowercase(),
                    "тема «{id}»: {one} и {other} — один и тот же цвет"
                );
            }
        }
    }

    /// Стекло задаёт свой фон и не перекрашивает показатели: тема и стекло
    /// выбираются и хранятся порознь.
    #[test]
    fn every_glass_has_an_independent_surface() {
        for id in settings::GLASSES {
            let own = blocks(&format!("[data-glass-preview=\"{id}\"] {{"));
            assert!(
                declared(&own, "--surface-base:").is_some(),
                "стекло «{id}» не задаёт фон"
            );
            assert!(
                TOKENS.contains(&format!(":root[data-glass=\"{id}\"]")),
                "стекло «{id}» не применяется к корню окна"
            );
            assert!(
                own.iter().all(|body| !body.contains("--seed-")),
                "стекло «{id}» меняет цветовую тему"
            );
        }
    }

    /// Тема, стекло и цвета режимов общие на все окна, а плотность бывает своя
    /// у виджета — и не должна утекать в окна, которые не виджеты.
    #[test]
    fn appearance_shares_colours_but_keeps_widget_opacity() {
        let mut settings = settings::Settings {
            appearance: settings::DesignAppearances {
                reference: settings::DesignAppearance {
                    palette: "custom".to_owned(), glass: "plum".to_owned(), accent_color: "#41C6DD".to_owned(),
                    ..settings::DesignAppearance::for_design(settings::Design::Reference)
                },
                ..settings::DesignAppearances::default()
            },
            ..settings::Settings::default()
        };
        settings
            .appearance.reference.custom_colors
            .insert("positive".to_owned(), "#71CFA6".to_owned());
        settings.set_widget(
            "gpu",
            settings::WidgetSettings {
                card_opacity: Some(0.99),
                ..settings::WidgetSettings::default()
            },
        );

        let widget = super::effective(&settings, "gpu");
        let window = super::effective(&settings, "settings");
        assert_eq!(widget.card_opacity, 0.99);
        assert_eq!(window.card_opacity, settings.card_opacity);
        assert_eq!(widget.glass, window.glass);
        assert_eq!(widget.custom_colors, window.custom_colors);

        // Имена полей — те, что читает `look.ts`
        let wire = serde_json::to_value(widget).expect("вид сериализуется");
        assert_eq!(wire["palette"], "custom");
        assert_eq!(wire["glass"], "plum");
        assert_eq!(wire["accentColor"], "#41C6DD");
        assert_eq!(wire["customColors"]["positive"], "#71CFA6");
    }


    #[test]
    fn technical_fallback_uses_claude_profile_without_changing_selection() {
        let settings = settings::Settings::default();
        let widget = super::effective(&settings, "sound");
        let technical = super::effective(&settings, "debug");
        assert_eq!(widget.palette, "gpt");
        assert_eq!(widget.glass, "gpt");
        assert_eq!(technical.effective_design, settings::Design::App);
        assert_eq!(technical.palette, settings::default_palette());
        assert_eq!(technical.design, settings::Design::Reference);
        assert_eq!(settings.appearance.reference.palette, "gpt");
    }
    /// Незнакомые имена и числа не должны раздавать окнам то, под что в токенах
    /// ничего нет.
    #[test]
    fn unknown_names_fall_back_to_the_first() {
        assert_eq!(
            settings::sane_palette("радужная".to_owned()),
            settings::default_palette()
        );
        assert_eq!(settings::sane_palette("mono".to_owned()), "monochrome");
        assert_eq!(
            settings::sane_glass("витраж".to_owned()),
            settings::default_glass()
        );
        assert_eq!(settings::sane_vivid(f64::NAN), settings::DEFAULT_VIVID);
        assert_eq!(settings::sane_vivid(9.0), settings::MAX_VIVID);
    }

    /// Собственные блоки темы — те, что объявлены на её образце.
    fn own(id: &str) -> Vec<&'static str> {
        blocks(&format!("[data-theme-preview=\"{id}\"] {{"))
    }

    /// Всё, из чего тема собирает цвета, в порядке каскада: корень, общий блок
    /// цветов и её собственные блоки. Именно так, а не весь файл: поиск по
    /// файлу нашёл бы объявление в чужой теме и прошёл бы по нему.
    fn scope(id: &str) -> Vec<&'static str> {
        let mut all = blocks(":root {");
        all.extend(blocks("[data-theme-preview] {"));
        all.extend(own(id));
        all
    }

    /// Все тела блоков с таким заголовком, по порядку в файле.
    ///
    /// ВАЖНО: **блоков с одним селектором бывает несколько, и берём мы все.**
    /// Первая редакция брала первый и резала его по ближайшей `}` — на такой
    /// проверке вариацию можно было доломать вторым блоком ниже по файлу, и
    /// тест продолжал бы говорить «прошло». Скобки считаем по балансу: внутри
    /// значения их нет, но полагаться на это незачем.
    fn blocks(head: &str) -> Vec<&'static str> {
        let mut found = Vec::new();
        let mut from = 0;

        while let Some(at) = TOKENS[from..].find(head) {
            let opens = from + at + head.len();
            let mut depth = 1;
            let mut end = opens;

            for (offset, symbol) in TOKENS[opens..].char_indices() {
                match symbol {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = opens + offset;
                            break;
                        }
                    }
                    _ => {}
                }
            }

            found.push(&TOKENS[opens..end]);
            from = end;
        }

        found
    }

    /// Последнее объявление переменной в этих блоках — то, что победит в
    /// каскаде. Именно последнее: первое переопределяют ниже по файлу.
    fn declared(scopes: &[&str], name: &str) -> Option<String> {
        let mut found = None;

        for body in scopes {
            let mut from = 0;
            while let Some(at) = body[from..].find(name) {
                let tail = &body[from + at + name.len()..];
                let end = tail.find(';')?;
                found = Some(tail[..end].trim().to_owned());
                from += at + name.len();
            }
        }

        found
    }

    /// Разворачивает роль до HEX базы, из которой она собрана.
    ///
    /// Без этого сравнение имён врёт: `var(--color-caution)` и `var(--metric-ram)`
    /// — разные строки и один и тот же цвет. Идёт и сквозь `var(...)`, и сквозь
    /// цвет, собранный из базы яркостью, — `oklch(from var(--seed-*) …)`.
    fn base_of(scope: &[&str], name: &str) -> String {
        let mut now =
            declared(scope, &format!("{name}:")).unwrap_or_else(|| panic!("{name} не объявлена"));

        // Цепочка ссылок короткая; предел спасает от кольца в самих токенах
        for _ in 0..8 {
            let reference = now
                .strip_prefix("oklch(from ")
                .and_then(|tail| tail.split_whitespace().next())
                .unwrap_or(&now);
            let Some(next) = reference
                .strip_prefix("var(")
                .and_then(|tail| tail.strip_suffix(')'))
            else {
                return now;
            };
            now = declared(scope, &format!("{next}:")).unwrap_or_else(|| panic!("{next} не объявлена"));
        }

        now
    }
}
