//! Язык интерфейса: каталог строк, язык системы и выбранный.
//!
//! Каталог один на обе стороны — `src/i18n/<код>.json`. Rust берёт файлы в
//! сборку `include_str!`, фронт грузит нужный динамическим `import()`: так
//! строка трея и подпись в окне настроек не могут разойтись, а переводчику
//! достаётся один файл на язык.
//!
//! Подписи показаний (CPU, GPU, Temperature…) и единицы в каталог не входят
//! вовсе: они английские во всех языках — решение пользователя.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::LazyLock;

use tauri::{AppHandle, Emitter, State};

use crate::settings::{self, SettingsState};

/// Событие смены языка. Несёт код языка, одинаковый для всех окон.
const EVENT: &str = "language";

pub struct Language {
    /// Код: имя файла каталога и значение в `settings.json`.
    pub code: &'static str,
    /// Самоназвание. Не переводится никогда: язык ищут по тому, как он
    /// называет себя сам, — человек, случайно включивший китайский, должен
    /// найти «Русский», не читая по-китайски.
    pub name: &'static str,
    catalog: &'static str,
}

/// Языки интерфейса. Первый — русский: он источник каталога, и в нём есть
/// каждая строка.
pub const LANGUAGES: &[Language] = &[
    Language { code: "ru", name: "Русский", catalog: include_str!("../../src/i18n/ru.json") },
    Language { code: "uk", name: "Українська", catalog: include_str!("../../src/i18n/uk.json") },
    Language { code: "en", name: "English", catalog: include_str!("../../src/i18n/en.json") },
    Language { code: "es", name: "Español", catalog: include_str!("../../src/i18n/es.json") },
    Language { code: "zh", name: "中文", catalog: include_str!("../../src/i18n/zh.json") },
];

/// Язык, когда системный не совпал ни с одним из наших.
const FALLBACK: &str = "en";

type Table = HashMap<String, String>;

static TABLES: LazyLock<Vec<Table>> = LazyLock::new(|| {
    LANGUAGES
        .iter()
        .map(|language| {
            serde_json::from_str(language.catalog).unwrap_or_else(|error| {
                eprintln!("[язык] каталог «{}» не разобрался: {error}", language.code);
                Table::new()
            })
        })
        .collect()
});

/// Номер выбранного языка в [`LANGUAGES`].
///
/// Статический атом, а не поле настроек: строки берут и трей, и потоки
/// опроса, и ни одному из них не за чем ходить за замком настроек.
static CURRENT: AtomicUsize = AtomicUsize::new(0);

fn index_of(code: &str) -> Option<usize> {
    LANGUAGES.iter().position(|language| language.code == code)
}

/// Код выбранного языка.
pub fn current() -> &'static str {
    LANGUAGES[CURRENT.load(Ordering::Relaxed).min(LANGUAGES.len() - 1)].code
}

/// Код языка Windows — того, на котором система говорит с человеком. Не
/// совпал ни с одним из наших — английский.
pub fn system() -> &'static str {
    code_for_langid(system_langid())
}

#[cfg(windows)]
fn system_langid() -> u16 {
    // SAFETY: функция без аргументов, только читает настройку текущего
    // пользователя
    unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() }
}

#[cfg(not(windows))]
fn system_langid() -> u16 {
    0
}

/// Код языка по LANGID Windows.
///
/// Смотрим только на основной язык — младшие десять бит: страна для выбора
/// каталога не важна, испанский Мексики — тот же испанский. Китайский
/// традиционный (Тайвань, Гонконг) при этом получает упрощённый: каталога на
/// традиционном нет, а упрощённый ему понятнее английского.
fn code_for_langid(langid: u16) -> &'static str {
    match langid & 0x3ff {
        0x19 => "ru",
        0x22 => "uk",
        0x09 => "en",
        0x0a => "es",
        0x04 => "zh",
        _ => FALLBACK,
    }
}

/// Язык, которым говорит приложение: выбранный, а не выбран — системный.
pub fn effective(chosen: Option<&str>) -> &'static str {
    chosen
        .and_then(index_of)
        .map_or_else(system, |index| LANGUAGES[index].code)
}

/// Ставит язык по выбору из настроек.
pub fn apply(chosen: Option<&str>) {
    let index = index_of(effective(chosen)).unwrap_or(0);
    CURRENT.store(index, Ordering::Relaxed);
}

/// Выбор из файла настроек: знакомый код или «как в Windows».
///
/// Незнакомый код — это правка руками или язык, которого в этой версии нет.
/// Съезжаем на системный, а не на первый: так человек хотя бы получит язык
/// своей Windows.
pub fn sane(chosen: Option<String>) -> Option<String> {
    chosen.filter(|code| index_of(code).is_some())
}

/// Строка каталога на выбранном языке.
///
/// Нет в выбранном — берём русскую: русский каталог — источник, и тест
/// держит, что в остальных ключи те же. Нет вовсе — сам ключ и строка в лог:
/// ключ на экране заметен сразу, а паника из-за подписи уронила бы окно.
pub fn text(key: &str) -> String {
    lookup(CURRENT.load(Ordering::Relaxed), key)
}

fn lookup(index: usize, key: &str) -> String {
    let tables = &*TABLES;

    if let Some(value) = tables.get(index).and_then(|table| table.get(key)) {
        return value.clone();
    }
    if let Some(value) = tables.first().and_then(|table| table.get(key)) {
        eprintln!("[язык] строки «{key}» нет в каталоге «{}»", LANGUAGES[index].code);
        return value.clone();
    }

    eprintln!("[язык] строки «{key}» нет в каталоге");
    key.to_owned()
}

/// Строка с подстановками: `{name}` в тексте меняется на значение.
pub fn format(key: &str, args: &[(&str, String)]) -> String {
    fill(text(key), args)
}

fn fill(mut text: String, args: &[(&str, String)]) -> String {
    for (name, value) in args {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    text
}

/// Строка каталога: `tr!("tray.quit")` или
/// `tr!("look.language.system", language = name)`.
///
/// Ключ обязан быть литералом: тест находит каждый `tr!` в исходниках и
/// требует его ключ в каталоге. Ключ, собранный на ходу (`widget.<id>`), идёт
/// через [`text`], и его держит свой тест.
#[macro_export]
macro_rules! tr {
    ($key:literal) => {
        $crate::i18n::text($key)
    };
    ($key:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::format($key, &[$((stringify!($name), ($value).to_string())),+])
    };
}

/// Язык окна при открытии.
#[tauri::command]
pub fn language() -> &'static str {
    current()
}

/// Смена языка из окна настроек. `None` — «как в Windows».
///
/// Меню виджета собирается на каждое открытие и ничего не ждёт; остальное
/// переводится здесь же: окна — событием, трей — пересборкой, заголовки окон
/// — поштучно, окно настроек — новым снимком, где имена из реестра и тем уже
/// на новом языке.
#[tauri::command]
pub fn set_language(app: AppHandle, state: State<'_, SettingsState>, language: Option<String>) {
    let language = sane(language);
    settings::update(&app, &state, |settings| settings.language = language.clone());

    apply(language.as_deref());

    let _ = app.emit(EVENT, current());
    crate::tray::refresh(&app);
    crate::windows::on_main(&app, |app| {
        crate::windows::retitle(&app);
        crate::settings_window::retitle(&app);
    });
    crate::settings_window::emit(&app);
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use super::*;

    /// Суффиксы форм множественного числа: `Intl.PluralRules` во фронте
    /// выбирает одну из них. Нет нужной — фронт берёт `_other`.
    const PLURAL: &[&str] = &["_zero", "_one", "_two", "_few", "_many", "_other"];

    fn plural_base(key: &str) -> Option<&str> {
        PLURAL.iter().find_map(|suffix| key.strip_suffix(suffix))
    }

    /// Ключ без формы множественного числа: у разных языков формы разные, а
    /// основа обязана быть одна.
    fn base(key: &str) -> &str {
        plural_base(key).unwrap_or(key)
    }

    fn placeholders(text: &str) -> BTreeSet<String> {
        let mut found = BTreeSet::new();
        let mut rest = text;
        while let Some(start) = rest.find('{') {
            let tail = &rest[start + 1..];
            let Some(end) = tail.find('}') else { break };
            found.insert(tail[..end].to_owned());
            rest = &tail[end + 1..];
        }
        found
    }

    #[test]
    fn every_catalog_parses() {
        for (language, table) in LANGUAGES.iter().zip(TABLES.iter()) {
            assert!(
                serde_json::from_str::<Table>(language.catalog).is_ok(),
                "каталог «{}» — не плоский JSON строк",
                language.code
            );
            assert!(!table.is_empty(), "каталог «{}» пуст", language.code);
        }
    }

    /// Все языки знают одни и те же строки. Формы множественного числа
    /// сравниваются по основе: у китайского одна форма, у русского четыре.
    #[test]
    fn catalogs_share_their_keys() {
        let source: BTreeSet<&str> = TABLES[0].keys().map(|key| base(key)).collect();

        for (language, table) in LANGUAGES.iter().zip(TABLES.iter()).skip(1) {
            let own: BTreeSet<&str> = table.keys().map(|key| base(key)).collect();
            let missing: Vec<_> = source.difference(&own).collect();
            let extra: Vec<_> = own.difference(&source).collect();
            assert!(
                missing.is_empty() && extra.is_empty(),
                "каталог «{}»: нет {missing:?}, лишние {extra:?}",
                language.code
            );
        }
    }

    /// У каждой основы с формами есть `_other`: на неё фронт падает, когда
    /// нужной формы нет. У русского и украинского — ещё три формы, без них
    /// «5 секунда» выходило бы молча.
    #[test]
    fn plurals_have_their_forms() {
        for (language, table) in LANGUAGES.iter().zip(TABLES.iter()) {
            let bases: BTreeSet<&str> = table.keys().filter_map(|key| plural_base(key)).collect();
            for stem in bases {
                let mut needed = vec!["_other"];
                if matches!(language.code, "ru" | "uk") {
                    needed.extend(["_one", "_few", "_many"]);
                }
                for suffix in needed {
                    assert!(
                        table.contains_key(&format!("{stem}{suffix}")),
                        "каталог «{}»: у «{stem}» нет формы {suffix}",
                        language.code
                    );
                }
            }
        }
    }

    /// Подстановки одинаковы во всех языках, и пустых строк нет. Потерянная
    /// `{title}` оставила бы в строке пустое место, лишняя — фигурные скобки.
    #[test]
    fn placeholders_match_and_nothing_is_empty() {
        for (language, table) in LANGUAGES.iter().zip(TABLES.iter()) {
            for (key, value) in table {
                assert!(!value.trim().is_empty(), "каталог «{}»: «{key}» пуст", language.code);

                let Some(source) = TABLES[0]
                    .get(key)
                    .or_else(|| TABLES[0].get(&format!("{}_other", base(key))))
                else {
                    continue;
                };
                assert_eq!(
                    placeholders(value),
                    placeholders(source),
                    "каталог «{}»: подстановки у «{key}» не те, что в русском",
                    language.code
                );
            }
        }
    }

    fn rust_sources(dir: &Path, found: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                rust_sources(&path, found);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                found.push(path);
            }
        }
    }

    /// Каждый `tr!("…")` в исходниках есть в каталоге. Опечатка в ключе иначе
    /// всплыла бы ключом на экране — и только на том языке и в той ветке кода,
    /// до которой дошли руками.
    #[test]
    fn every_tr_key_exists() {
        let mut files = Vec::new();
        rust_sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);

        let mut missing = Vec::new();
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            let mut rest = text.as_str();
            while let Some(start) = rest.find("tr!(\"") {
                let tail = &rest[start + 5..];
                let Some(end) = tail.find('"') else { break };
                let key = &tail[..end];
                // Многоточие в комментариях вида «`tr!("…")`» ключом не считается
                let looks_like_key = !key.is_empty()
                    && key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
                if looks_like_key && !TABLES[0].contains_key(key) {
                    missing.push(format!("{}: {key}", file.display()));
                }
                rest = &tail[end..];
            }
        }

        assert!(missing.is_empty(), "ключей нет в каталоге: {missing:?}");
    }

    /// Название каждого виджета — в каталоге: его ключ собирается из `id`, и
    /// `tr!` его не видит.
    #[test]
    fn every_widget_has_a_title() {
        for widget in crate::widgets::WIDGETS {
            let key = format!("widget.{}", widget.id);
            assert!(TABLES[0].contains_key(&key), "нет названия «{key}»");
        }
    }

    /// Имена размеров, видов и обложек в реестре — ключи каталога, как и имена
    /// тем и стёкол. Забытый ключ вылез бы в меню виджета как «size.full».
    #[test]
    fn every_registry_name_is_a_key() {
        let mut keys: Vec<String> = Vec::new();
        for widget in crate::widgets::WIDGETS {
            keys.extend(widget.views.iter().map(|key| (*key).to_owned()));
            keys.extend(widget.covers.iter().map(|key| (*key).to_owned()));
            keys.extend(widget.presets.iter().map(|preset| preset.name.to_owned()));
            for list in widget.view_presets {
                keys.extend(list.iter().map(|preset| preset.name.to_owned()));
            }
        }
        keys.extend(crate::settings::PALETTES.iter().map(|id| format!("palette.{id}")));
        keys.extend(crate::settings::GLASSES.iter().map(|id| format!("glass.{id}")));

        let missing: Vec<_> = keys.iter().filter(|key| !TABLES[0].contains_key(*key)).collect();
        assert!(missing.is_empty(), "нет в каталоге: {missing:?}");
    }

    #[test]
    fn windows_languages_map_to_ours() {
        assert_eq!(code_for_langid(0x0419), "ru");
        assert_eq!(code_for_langid(0x0422), "uk");
        assert_eq!(code_for_langid(0x0409), "en");
        assert_eq!(code_for_langid(0x0809), "en");
        assert_eq!(code_for_langid(0x0c0a), "es");
        assert_eq!(code_for_langid(0x080a), "es");
        assert_eq!(code_for_langid(0x0804), "zh");
        assert_eq!(code_for_langid(0x0404), "zh", "традиционный получает упрощённый");
        assert_eq!(code_for_langid(0x0407), "en", "немецкого нет — английский");
    }

    #[test]
    fn chosen_language_beats_the_system() {
        assert_eq!(effective(Some("zh")), "zh");
        assert_eq!(effective(Some("uk")), "uk");
        assert_eq!(effective(None), system());
        assert_eq!(effective(Some("de")), system(), "незнакомый код — системный");
    }

    #[test]
    fn unknown_choice_becomes_system() {
        assert_eq!(sane(Some("es".to_owned())), Some("es".to_owned()));
        assert_eq!(sane(Some("de".to_owned())), None);
        assert_eq!(sane(None), None);
    }

    #[test]
    fn lookup_falls_back_to_russian_then_to_the_key() {
        assert_eq!(lookup(2, "tray.quit"), "Quit");
        assert_eq!(lookup(0, "tray.quit"), "Выход");
        assert_eq!(lookup(0, "no.such.key"), "no.such.key");
    }

    #[test]
    fn placeholders_are_filled() {
        assert_eq!(
            fill("Как в Windows · {language}".to_owned(), &[("language", "Русский".to_owned())]),
            "Как в Windows · Русский"
        );
    }
}
