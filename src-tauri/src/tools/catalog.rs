//! Каталог докачиваемых программ: что это, откуда берётся и кому нужно.
//!
//! Единственное место, где названы репозитории и имена файлов. Сменил проект
//! имя сборки — правится строка здесь, а до правки приложение прямо говорит
//! «сборка не найдена», а не ломается молча.
//!
//! Здесь же записано, какой модуль что тянет (`Module::needs`). Будущий
//! переключатель модулей встанет сюда одним фильтром: выключенный модуль не
//! просит своих программ, и приложение их не качает.

use serde::{Deserialize, Serialize};

/// Докачиваемая программа.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Component {
    Ffmpeg,
    YtDlp,
    QuickJs,
    PresentMon,
    HeadsetControl,
}

impl Component {
    pub const ALL: [Component; 5] = [Component::Ffmpeg, Component::YtDlp, Component::QuickJs, Component::PresentMon, Component::HeadsetControl];
}

/// Часть приложения, которой нужны программы. Совпадает с `id` виджета в
/// реестре и с ключом папки вывода в настройках.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Module {
    Converter,
    Downloader,
    Fps,
    Battery,
}

impl Module {
    pub const ALL: [Module; 4] = [Module::Converter, Module::Downloader, Module::Fps, Module::Battery];

    pub fn id(self) -> &'static str {
        match self {
            Module::Converter => "converter",
            Module::Downloader => "downloader",
            Module::Fps => "fps",
            Module::Battery => "battery",
        }
    }

    pub fn from_id(id: &str) -> Option<Module> {
        Module::ALL.into_iter().find(|module| module.id() == id)
    }

    /// Папка вывода внутри `Рабочий стол\WinGets`. Имена английские: приложение
    /// раздают и переводят, и папка у всех должна называться одинаково.
    pub fn folder(self) -> &'static str {
        match self {
            Module::Converter => "Converter",
            Module::Downloader => "Downloads",
            // Счётчик кадров файлов не пишет; имя ради полноты сопоставления
            Module::Fps => "Fps",
            Module::Battery => "Battery",
        }
    }

    /// Что модулю нужно для работы.
    ///
    /// Загрузчику нужен и ffmpeg: YouTube отдаёт качество выше 720p отдельно
    /// видео и отдельно звуком, склеивает их yt-dlp именно им, им же делает MP3.
    pub fn needs(self) -> &'static [Component] {
        match self {
            Module::Converter => &[Component::Ffmpeg],
            Module::Downloader => &[Component::YtDlp, Component::QuickJs, Component::Ffmpeg],
            Module::Fps => &[Component::PresentMon],
            Module::Battery => &[Component::HeadsetControl],
        }
    }
}

/// Как узнать нужный файл среди файлов выпуска.
#[derive(Debug, Clone, Copy)]
pub enum AssetRule {
    /// Имя не меняется от выпуска к выпуску.
    Exact(&'static str),
    /// В имени номер версии: `ffmpeg-9.0.2-essentials_build.7z`.
    Around { prefix: &'static str, suffix: &'static str },
}

impl AssetRule {
    pub fn matches(self, name: &str) -> bool {
        match self {
            AssetRule::Exact(exact) => name == exact,
            AssetRule::Around { prefix, suffix } => {
                name.len() > prefix.len() + suffix.len()
                    && name.starts_with(prefix)
                    && name.ends_with(suffix)
            }
        }
    }
}

/// Что делать со скачанным.
#[derive(Debug, Clone, Copy)]
pub enum Install {
    /// Файл выпуска и есть программа.
    Exe,
    /// Программа лежит в архиве; путь внутри кончается на `member`.
    Archive { member: &'static str },
}

pub struct Spec {
    pub component: Component,
    /// Имя для людей — в карточке и в настройках.
    pub name: &'static str,
    /// `владелец/репозиторий` на GitHub.
    pub repo: &'static str,
    pub asset: AssetRule,
    pub install: Install,
    /// Папка программы внутри каталога компонентов.
    pub dir: &'static str,
    /// Имя исполняемого файла в этой папке.
    pub file: &'static str,
    /// Сколько качать и сколько займёт на диске — для текста согласия.
    ///
    /// Числа из выпуска, замеренного при написании, а не из сети: до нажатия
    /// кнопки приложение в сеть не ходит вовсе. Точный размер приходит уже в
    /// прогрессе скачивания.
    pub download_bytes: u64,
    pub disk_bytes: u64,
    /// Имя в `PATH`, если уже стоящую в системе программу можно взять как есть.
    pub system: Option<&'static str>,
}

/// Все программы.
///
/// ffmpeg — облегчённая сборка gyan.dev (essentials): 35 МБ в 7z против 87 МБ
/// zip у сборок yt-dlp и 194 МБ у полной. Из архива берётся один `ffmpeg.exe`:
/// `ffprobe` не нужен вовсе — что внутри файла, печатает и `ffmpeg -i`, а
/// yt-dlp без `ffprobe` обходится сам (замерено: склейка и MP3 проходят). Это
/// 105 МБ на диске, которых нет.
///
/// Движок JavaScript — QuickJS-NG, 2 МБ, а не рекомендованный yt-dlp Deno в
/// 43 МБ: YouTube с ним отдаёт все форматы, задачи решаются за две-три секунды
/// (замерено). Если Deno у человека стоит, yt-dlp возьмёт его сам — он у него
/// в приоритете.
///
/// PresentMon (Intel, MIT) — консольная сборка без оверлея и службы: 0,9 МБ
/// против 162 МБ установщика. Кадры он берёт из ETW, в игру ничего не
/// внедряет, поэтому античитам не виден.
///
/// HeadsetControl (GPL v3) — заряд гарнитур десятка брендов: их донглы
/// говорят каждый своим протоколом, и Windows этого заряда не видит. Отдельная
/// программа, которую мы только запускаем, — с приложением она не связывается.
/// Переносная сборка одним exe, без установщика.
pub const SPECS: [Spec; 5] = [
    Spec {
        component: Component::Ffmpeg,
        name: "FFmpeg",
        repo: "GyanD/codexffmpeg",
        asset: AssetRule::Around { prefix: "ffmpeg-", suffix: "-essentials_build.7z" },
        install: Install::Archive { member: "/bin/ffmpeg.exe" },
        dir: "ffmpeg",
        file: "ffmpeg.exe",
        download_bytes: 35_430_500,
        disk_bytes: 105_423_872,
        system: Some("ffmpeg.exe"),
    },
    Spec {
        component: Component::YtDlp,
        name: "yt-dlp",
        repo: "yt-dlp/yt-dlp",
        asset: AssetRule::Exact("yt-dlp.exe"),
        install: Install::Exe,
        dir: "yt-dlp",
        file: "yt-dlp.exe",
        download_bytes: 17_840_399,
        disk_bytes: 17_840_399,
        // Свой и только свой: копия из pip у человека обновляется не нами, а
        // старый yt-dlp на YouTube — это нерабочий yt-dlp
        system: None,
    },
    Spec {
        component: Component::QuickJs,
        name: "QuickJS",
        repo: "quickjs-ng/quickjs",
        asset: AssetRule::Exact("qjs-windows-x86_64.exe"),
        install: Install::Exe,
        dir: "quickjs",
        file: "qjs.exe",
        download_bytes: 2_151_363,
        disk_bytes: 2_151_363,
        system: None,
    },
    Spec {
        component: Component::PresentMon,
        name: "PresentMon",
        repo: "GameTechDev/PresentMon",
        // `PresentMon-2.6.0-x64.exe`; рядом лежат `.msi` и arm64 — не они
        asset: AssetRule::Around { prefix: "PresentMon-", suffix: "-x64.exe" },
        install: Install::Exe,
        dir: "presentmon",
        file: "PresentMon.exe",
        download_bytes: 980_320,
        disk_bytes: 980_320,
        system: None,
    },
    Spec {
        component: Component::HeadsetControl,
        name: "HeadsetControl",
        repo: "Sapd/HeadsetControl",
        // Рядом лежат установщик `-setup.exe` и подписи `.asc` — не они
        asset: AssetRule::Exact("headsetcontrol-windows-x86_64.exe"),
        install: Install::Exe,
        dir: "headsetcontrol",
        file: "headsetcontrol.exe",
        download_bytes: 1_339_392,
        disk_bytes: 1_339_392,
        system: None,
    },
];

pub fn spec(component: Component) -> &'static Spec {
    SPECS
        .iter()
        .find(|spec| spec.component == component)
        .expect("у каждой программы есть запись в каталоге")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_component_has_a_spec() {
        for component in Component::ALL {
            assert_eq!(spec(component).component, component);
        }
    }

    #[test]
    fn modules_round_trip_through_their_ids() {
        for module in Module::ALL {
            assert_eq!(Module::from_id(module.id()), Some(module));
        }
        assert_eq!(Module::from_id("notes"), None);
    }

    /// Номер версии в середине имени не мешает узнать сборку, а соседние
    /// варианты того же выпуска — полная и с общими библиотеками — не
    /// подходят.
    #[test]
    fn the_ffmpeg_rule_takes_only_the_essentials_7z() {
        let rule = spec(Component::Ffmpeg).asset;
        assert!(rule.matches("ffmpeg-9.0.2-essentials_build.7z"));
        assert!(rule.matches("ffmpeg-10.1-essentials_build.7z"));
        assert!(!rule.matches("ffmpeg-9.0.2-essentials_build.zip"));
        assert!(!rule.matches("ffmpeg-9.0.2-full_build.7z"));
        assert!(!rule.matches("ffmpeg-9.0.2-full_build-shared.7z"));
        assert!(!rule.matches("ffmpeg--essentials_build.7z"));
    }

    #[test]
    fn the_presentmon_rule_takes_only_the_x64_console() {
        let rule = spec(Component::PresentMon).asset;
        assert!(rule.matches("PresentMon-2.6.0-x64.exe"));
        assert!(!rule.matches("PresentMon-2.6.0.msi"));
        assert!(!rule.matches("PresentMon-2.6.0-arm64.exe"));
        assert!(!rule.matches("ReleaseSymbols.zip"));
    }

    #[test]
    fn the_downloader_needs_all_three_and_the_converter_one() {
        assert_eq!(Module::Converter.needs(), &[Component::Ffmpeg]);
        assert_eq!(Module::Downloader.needs().len(), 3);
    }

    #[test]
    fn ids_arrive_in_camel_case() {
        assert_eq!(serde_json::to_string(&Component::YtDlp).unwrap(), "\"ytDlp\"");
        assert_eq!(serde_json::to_string(&Component::QuickJs).unwrap(), "\"quickJs\"");
        assert_eq!(serde_json::to_string(&Component::PresentMon).unwrap(), "\"presentMon\"");
        assert_eq!(serde_json::to_string(&Module::Downloader).unwrap(), "\"downloader\"");
    }
}
