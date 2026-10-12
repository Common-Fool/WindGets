//! Открытие внешних ссылок в браузере по умолчанию.
//!
//! Строка адреса приходит из текста заметки (то есть вводится человеком), поэтому
//! перед запуском мы строго проверяем её на безопасность. Схема `file://`,
//! локальные пути и любые команды командной строки отсекаются.
//!
//! ВАЖНО: **не открывать ссылки через `opener` из потока команды.** На Windows
//! он идёт в `ShellExecute`, то есть в COM, — а на COM же держится и WebView2.
//! Вызов оттуда подвешивает приём IPC во всех окнах разом. Поэтому веб-адрес
//! открывает `ShellExecuteExW` в своём одноразовом потоке ([`open_web`]), а
//! папки и протоколы — отдельный процесс `explorer.exe` ([`launch`]).

/// Проверяет адрес ссылки перед открытием.
///
/// Разрешены только веб-ссылки (`http://` и `https://`), без пробелов,
/// переносов строк и любых управляющих символов.
pub fn is_safe_link(url: &str) -> bool {
    let has_valid_scheme = url.starts_with("http://") || url.starts_with("https://");
    if !has_valid_scheme {
        return false;
    }

    !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Открывает папку или короткий адрес протокола через explorer.exe.
///
/// Для веб-ссылок не годится — см. [`open_web`]. Здесь бывают только папка
/// вывода и адрес Спотифая: оба короткие, без запятых, и проводник их понимает.
///
/// ВАЖНО: строку сюда передают только проверенную или зашитую в код, сама она
/// ничего не проверяет.
pub(crate) fn launch(target: &str) {
    #[cfg(windows)]
    {
        // Ответа не ждём — `spawn`, а не `status`: браузер живёт своей жизнью.
        // Путь полный от SystemRoot: см. `autostart::system_exe` и `settings_window::open_config_dir`.
        let explorer = match std::env::var_os("SystemRoot") {
            Some(root) => std::path::Path::new(&root).join("explorer.exe"),
            None => std::path::PathBuf::from("explorer.exe"),
        };
        if let Err(error) = std::process::Command::new(explorer).arg(target).spawn() {
            eprintln!("ссылка не открылась: {error}");
        }
    }
    #[cfg(not(windows))]
    {
        let _ = target;
    }
}

/// Открывает веб-адрес в браузере по умолчанию.
///
/// ВАЖНО: **не через `explorer.exe`.** Проводник разбирает свой аргумент как
/// путь со списком ключей, а не как адрес: запятая делит адрес на ключи, а
/// длинный он не разбирает вовсе — и молча открывает папку по умолчанию,
/// «Документы». Так ломались ссылки из писем: у рассылок они бывают в тысячу с
/// лишним знаков. Выглядело это как сломанная ссылка, хотя адрес был верный.
///
/// Открывает оболочка — `ShellExecuteExW`, ровно как сама Windows открывает
/// адрес, — но в своём одноразовом STA-потоке, как окно выбора в `picker.rs`:
/// из потока команды её звать нельзя, там COM, на котором держится WebView2.
/// Ответа не ждём: поток закончится сам вместе с запуском браузера.
///
/// ВАЖНО: строку сюда передают только проверенную (`is_safe_link`).
fn open_web(url: &str) {
    #[cfg(windows)]
    {
        let wide: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
        let spawned = std::thread::Builder::new()
            .name("open-link".to_owned())
            // SAFETY: строка оканчивается нулём и живёт в потоке до конца вызова
            .spawn(move || unsafe { shell_open(&wide) });
        if let Err(error) = spawned {
            eprintln!("поток ссылки не поднялся: {error}");
        }
    }
    #[cfg(not(windows))]
    {
        let _ = url;
    }
}

/// # Safety
///
/// `url` — строка UTF-16, оканчивающаяся нулём.
#[cfg(windows)]
unsafe fn shell_open(url: &[u16]) {
    use windows::core::{w, PCWSTR};
    use windows::Win32::System::Com::{
        CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
    };
    use windows::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW,
    };
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    // Оболочке нужна однопоточная квартира — та же, что окну выбора
    let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);

    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        // NOASYNC: поток живёт ровно до конца вызова, и запуск не должен
        // оборваться вместе с ним. NO_UI: окно ошибки оболочки тут некому
        // показывать — отказ уходит в лог
        fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: w!("open"),
        lpFile: PCWSTR(url.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };

    if let Err(error) = ShellExecuteExW(&mut info) {
        eprintln!("ссылка не открылась: {error}");
    }

    if com.is_ok() {
        CoUninitialize();
    }
}

/// Показывает файл в проводнике — открывает его папку и выделяет его.
///
/// ВАЖНО: **аргумент собирается руками (`raw_arg`), а не через `.arg()`.**
/// Проводник разбирает `/select,` сам и ждёт путь в кавычках после запятой:
/// `/select,"C:\a b\c.mp4"`. `.arg()` берёт в кавычки всё слово целиком, и с
/// пробелом в пути проводник молча открывает «Документы» вместо папки. Кавычек
/// внутри пути Windows не допускает, так что ломать эту сборку нечем.
pub(crate) fn reveal(path: &std::path::Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        let explorer = crate::autostart::system_exe("explorer.exe");
        let argument = format!("/select,\"{}\"", path.display());
        if let Err(error) = std::process::Command::new(explorer).raw_arg(argument).spawn() {
            eprintln!("проводник не открылся: {error}");
        }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
    }
}

/// Открывает проверенную ссылку в системном браузере.
///
/// Отказ пишется только в лог, карточка ответа не ждёт.
#[tauri::command]
pub fn open_link(url: String) {
    if !is_safe_link(&url) {
        eprintln!("недопустимый адрес ссылки: {url}");
        return;
    }

    open_web(&url);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Корректные http:// и https:// ссылки успешно проходят проверку.
    #[test]
    fn valid_http_and_https_links_are_accepted() {
        assert!(is_safe_link("https://example.com"));
        assert!(is_safe_link("http://localhost:8080/path?query=1&b=2#hash"));
        assert!(is_safe_link("https://github.com/tauri-apps/tauri"));
    }

    /// Ссылка с пробелом отклоняется.
    #[test]
    fn link_with_space_is_rejected() {
        assert!(!is_safe_link("https://example.com/foo bar"));
        assert!(!is_safe_link("https:// example.com"));
        assert!(!is_safe_link(" https://example.com"));
    }

    /// Ссылка с переводом строки отклоняется.
    #[test]
    fn link_with_newline_is_rejected() {
        assert!(!is_safe_link("https://example.com\n"));
        assert!(!is_safe_link("https://example.com\r\n"));
        assert!(!is_safe_link("https://example.com/page\nsecond_line"));
    }

    /// Схема file:// строго запрещена.
    #[test]
    fn file_scheme_is_rejected() {
        assert!(!is_safe_link("file:///C:/Windows/notepad.exe"));
        assert!(!is_safe_link("file://localhost/etc/passwd"));
    }

    /// Голые системные пути и команды отклоняются.
    #[test]
    fn bare_path_is_rejected() {
        assert!(!is_safe_link("C:\\Windows\\System32\\calc.exe"));
        assert!(!is_safe_link("/bin/sh"));
        assert!(!is_safe_link("notepad.exe"));
    }

    /// Пустая строка отклоняется.
    #[test]
    fn empty_string_is_rejected() {
        assert!(!is_safe_link(""));
    }

    /// Управляющие символы и табуляция отклоняются.
    #[test]
    fn control_characters_and_tabs_are_rejected() {
        assert!(!is_safe_link("https://example.com/\t"));
        assert!(!is_safe_link("https://example.com/\0"));
        assert!(!is_safe_link("https://example.com/\x1b"));
    }
}
