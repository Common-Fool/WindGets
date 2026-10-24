//! Запуск вместе с системой — задачей планировщика, а не ключом `Run`.
//!
//! Причина в правах. Приложению они нужны постоянно (иначе не читаются
//! накопители, плата и температура процессора), и оно просит их манифестом.
//! Запущенное из ветки `Run`, такое приложение упиралось бы в окно UAC на
//! каждом входе в систему; задача с наивысшими правами поднимает его молча —
//! ровно для этого признак `RunLevel` и существует.
//!
//! Разговариваем с планировщиком через `schtasks`, а не через COM-интерфейс
//! Task Scheduler: три команды против регистрации COM-объекта и дерева
//! интерфейсов, а на COM в этом приложении уже держится WebView2.

use std::process::Command;

/// Имя задачи. Оно же видно в планировщике, поэтому человеческое.
const TASK: &str = "Windgets";

/// Консольного окна от `schtasks` быть не должно: галочку жмут при живых
/// виджетах, и чёрный прямоугольник моргнул бы поверх обоев.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Полный путь к системной программе вместо голого имени.
///
/// ВАЖНО: **`Command::new("что-то.exe")` ищет по правилам `CreateProcess`, а
/// они начинают с каталога самого приложения.** Подложенный туда файл с тем же
/// именем запустился бы вместо системного — и с нашими правами, а приложение
/// работает от администратора. В установленном виде каталог защищён правами,
/// но в разработке он открыт на запись, и полагаться тут не на что.
///
/// ВАЖНО: **папку Windows спрашиваем у системы, а не у переменной
/// `SystemRoot`.** Окружение собирается и из `HKCU\Environment`, куда пишет
/// пользователь без повышения: подставленная там папка отдала бы наш запуск
/// подложенному exe. `GetSystemWindowsDirectoryW` окружения не читает.
///
/// Общая на всё приложение: тем же путём идут `tar.exe` у компонентов и
/// `explorer.exe` у «показать в папке».
#[cfg(windows)]
pub(crate) fn system_exe(relative: &str) -> std::path::PathBuf {
    use windows::Win32::System::SystemInformation::GetSystemWindowsDirectoryW;

    let mut wide = [0u16; 260];
    // SAFETY: буфер — срез с длиной; ответ больше буфера значит «не влезло»
    let length = unsafe { GetSystemWindowsDirectoryW(Some(&mut wide)) } as usize;
    let root = if length > 0 && length < wide.len() {
        String::from_utf16_lossy(&wide[..length])
    } else {
        // Отказать системный вызов не должен; на этот случай — обычная папка,
        // но не относительный путь, который искался бы от рабочего каталога
        r"C:\Windows".to_owned()
    };
    std::path::Path::new(&root).join(relative)
}

fn schtasks(args: &[&str]) -> Result<(), String> {
    #[cfg(windows)]
    let mut command = Command::new(system_exe("System32\\schtasks.exe"));
    #[cfg(not(windows))]
    let mut command = Command::new("schtasks.exe");
    command.args(args);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let outcome = command
        .output()
        .map_err(|error| format!("schtasks не запустился: {error}"))?;

    if outcome.status.success() {
        return Ok(());
    }

    // Планировщик пишет на языке Windows в кодировке консоли: человеку такую
    // строку не покажешь, а в журнале она — единственная подсказка, почему
    let said = console_text(&outcome.stderr);
    Err(format!("schtasks вернул {}: {}", outcome.status, said.trim()))
}

/// Вывод консольной программы — в кодировке консоли (OEM), а не в UTF-8.
#[cfg(windows)]
fn console_text(bytes: &[u8]) -> String {
    use windows::Win32::Globalization::{MultiByteToWideChar, CP_OEMCP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS};

    if bytes.is_empty() {
        return String::new();
    }
    // SAFETY: оба буфера — срезы с длиной; первый вызов только меряет
    let length = unsafe { MultiByteToWideChar(CP_OEMCP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, None) };
    if length <= 0 {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let mut wide = vec![0u16; length as usize];
    let written =
        unsafe { MultiByteToWideChar(CP_OEMCP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, Some(&mut wide)) };
    String::from_utf16_lossy(&wide[..written.max(0) as usize])
}

#[cfg(not(windows))]
fn console_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Заводит задачу на вход в систему.
///
/// `/RL HIGHEST` — то самое, ради чего всё затевалось. `/F` перезаписывает
/// прежнюю: путь к приложению меняется при переустановке, и оставленная с
/// прошлого раза задача указывала бы в никуда.
pub fn enable() -> Result<(), String> {
    let exe = std::env::current_exe()
        .map_err(|error| format!("не найти своё же расположение: {error}"))?;

    // Кавычки вокруг пути обязательны: в «Program Files» есть пробел
    let target = format!("\"{}\"", exe.display());
    let user = whoami();

    // На входе в систему оболочка и WebView2 ещё запускаются. Даём им 15 секунд,
    // а /IT явно удерживает графическое приложение в интерактивной сессии.
    schtasks(&[
        "/Create", "/TN", TASK, "/TR", &target, "/SC", "ONLOGON", "/RU", &user, "/RL", "HIGHEST",
        "/IT", "/DELAY", "0000:15", "/F",
    ])?;
    // Задача уже есть и работает: неудача правки — не отказ включения, иначе
    // галочка осталась бы снятой при живой задаче. Правку повторит запуск
    // (`settings::reconcile_autostart`)
    if let Err(error) = relax() {
        eprintln!("[автозапуск] {error}");
    }
    Ok(())
}

/// Снимает с задачи умолчания `schtasks`, которые гасят приложение.
///
/// ВАЖНО: **`schtasks /Create` без XML ставит лимит работы 72 часа и запреты
/// питания от батареи.** Через трое суток без перезагрузки Windows убивала бы
/// приложение на любой машине, а на ноутбуке оно не стартовало бы от батареи и
/// гибло бы, когда выдернут зарядку. Ключей для этого у `schtasks` нет, а XML
/// пришлось бы класть файлом в открытую пользователю папку и отдавать
/// повышенному `schtasks` — подменяемая команда автозапуска. Поэтому правим
/// уже заведённую задачу через COM планировщика в отдельном PowerShell, как и
/// проверку наличия.
#[cfg(windows)]
pub fn relax() -> Result<(), String> {
    let outcome = powershell(&relax_script(TASK))
        .map_err(|error| format!("настройка задачи не запустилась: {error}"))?;
    if outcome.status.success() {
        return Ok(());
    }
    Err(format!(
        "настройки задачи не поправились (код {:?}): {}",
        outcome.status.code(),
        String::from_utf8_lossy(&outcome.stderr).trim()
    ))
}

#[cfg(not(windows))]
pub fn relax() -> Result<(), String> {
    Ok(())
}

#[cfg(any(windows, test))]
fn relax_script(task: &str) -> String {
    // Имя передаётся строкой PowerShell, а не частью выполняемого выражения.
    let task = task.replace('\'', "''");
    // Регистрируем заново только при расхождении: 6 — создать или обновить,
    // пользователь и вид входа — те же, что у задачи
    format!(r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
try {{
    $scheduler = New-Object -ComObject Schedule.Service
    $scheduler.Connect()
    $folder = $scheduler.GetFolder('\')
    $definition = $folder.GetTask('{task}').Definition
    $settings = $definition.Settings
    if ($settings.DisallowStartIfOnBatteries -or $settings.StopIfGoingOnBatteries -or $settings.ExecutionTimeLimit -ne 'PT0S') {{
        $settings.DisallowStartIfOnBatteries = $false
        $settings.StopIfGoingOnBatteries = $false
        $settings.ExecutionTimeLimit = 'PT0S'
        $principal = $definition.Principal
        $null = $folder.RegisterTaskDefinition('{task}', $definition, 6, $principal.UserId, $null, $principal.LogonType)
    }}
    exit 0
}} catch {{
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}}
"#)
}

/// Убирает задачу. Отсутствие задачи ошибкой не считаем: цель достигнута.
///
/// Сперва удаляем, потом спрашиваем, а не наоборот: отказ самого вопроса
/// (служба планировщика занята) прежде читался как «задачи нет», и галочка
/// снималась при живой задаче.
pub fn disable() -> Result<(), String> {
    confirm_disabled(schtasks(&["/Delete", "/TN", TASK, "/F"]), is_enabled)
}

fn confirm_disabled(deleted: Result<(), String>, query: impl FnOnce() -> Result<bool, String>) -> Result<(), String> {
    let Err(error) = deleted else {
        return Ok(());
    };
    match query() {
        Ok(false) => Ok(()),
        Ok(true) => Err(error),
        Err(check) => Err(format!("{error}; отсутствие задачи не подтверждено: {check}")),
    }
}

/// Есть / подтверждённо нет / проверить не удалось. Обычный schtasks
/// возвращает один код и для отсутствия задачи, и для отказа доступа.
pub fn is_enabled() -> Result<bool, String> {
    match schtasks(&["/Query", "/TN", TASK]) {
        Ok(()) => Ok(true),
        Err(error) => query_missing().map_err(|check| format!("{error}; {check}")),
    }
}

/// Только код отсутствия от GetTask подтверждает удаление. Локализованный
/// текст schtasks для этого не годится. COM живёт в отдельном PowerShell,
/// чтобы не пересекаться с COM главного потока WebView2; обычный удачный
/// /Query до этого пути не доходит.
#[cfg(windows)]
fn query_missing() -> Result<bool, String> {
    let outcome = powershell(&presence_script(TASK))
        .map_err(|error| format!("проверка планировщика не запустилась: {error}"))?;
    presence_result(outcome.status.code(), &String::from_utf8_lossy(&outcome.stderr))
}

/// Скрипт планировщика в отдельном PowerShell без окна: COM планировщика не
/// пересекается с COM главного потока WebView2.
#[cfg(windows)]
fn powershell(script: &str) -> std::io::Result<std::process::Output> {
    use std::os::windows::process::CommandExt;

    Command::new(system_exe("System32\\WindowsPowerShell\\v1.0\\powershell.exe"))
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
}

#[cfg(not(windows))]
fn query_missing() -> Result<bool, String> {
    Err("проверка планировщика доступна только в Windows".to_owned())
}

#[cfg(any(windows, test))]
fn presence_script(task: &str) -> String {
    // Имя передаётся строкой PowerShell, а не частью выполняемого выражения.
    let task = task.replace('\'', "''");
    format!(r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
try {{
    $scheduler = New-Object -ComObject Schedule.Service
    $scheduler.Connect()
    $folder = $scheduler.GetFolder('\')
}} catch {{
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}}
try {{
    $null = $folder.GetTask('{task}')
    exit 0
}} catch {{
    $failure = $_.Exception
    while ($failure.InnerException) {{ $failure = $failure.InnerException }}
    if ($failure.HResult -eq -2147024894) {{ exit 2 }}
    [Console]::Error.WriteLine($failure.Message)
    exit 1
}}
"#)
}

#[cfg(any(windows, test))]
fn presence_result(code: Option<i32>, said: &str) -> Result<bool, String> {
    match code {
        Some(0) => Ok(true),
        // Собственный код скрипта: HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)
        // именно от GetTask, а не от запуска COM или подключения к службе.
        Some(2) => Ok(false),
        _ => Err(format!("планировщик не подтвердил наличие задачи (код {code:?}): {}", said.trim())),
    }
}

/// Кем задача будет выполняться — нынешний пользователь с доменом.
///
/// Повышение прав пользователя не меняет, поэтому спрашивать окружение можно
/// и из привилегированного процесса.
fn whoami() -> String {
    let name = std::env::var("USERNAME").unwrap_or_default();
    let domain = std::env::var("USERDOMAIN").ok();
    account_name(&name, domain.as_deref())
}

fn account_name(name: &str, domain: Option<&str>) -> String {
    match domain {
        Some(domain) if !domain.is_empty() && !name.is_empty() => format!("{domain}\\{name}"),
        _ => name.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Имя пользователя обязано собираться в вид, который принимает `/RU`.
    #[test]
    fn the_user_is_named_the_way_schtasks_expects() {
        assert_eq!(account_name("sample", Some("DOMAIN")), "DOMAIN\\sample");
        assert_eq!(account_name("sample", None), "sample");
        assert_eq!(account_name("sample", Some("")), "sample");
        assert_eq!(account_name("", Some("DOMAIN")), "");
    }

    /// Круг «завести — проверить — убрать» на живом планировщике.
    ///
    /// Требует прав администратора: задачу с наивысшими правами иначе не
    /// зарегистрировать. Задачу за собой убирает.
    #[test]
    #[ignore = "трогает живой планировщик задач, нужны права администратора"]
    fn the_task_appears_and_goes() {
        let had = is_enabled().expect("состояние задачи проверено");

        enable().expect("задача заводится");
        assert!(is_enabled().expect("состояние задачи проверено"), "заведённой задачи не видно");

        disable().expect("задача убирается");
        assert!(!is_enabled().expect("состояние задачи проверено"), "задача осталась после удаления");

        // Возвращаем как было: тест не должен менять настройки машины
        if had {
            enable().expect("задача возвращается на место");
        }
    }

    /// Удаление программы снимает задачу автозапуска — ту самую, с тем же
    /// именем. Разойдись имя здесь и в хуке установщика, задача пережила бы
    /// удаление и запускала бы на каждом входе несуществующий exe.
    ///
    /// Права у удаления есть только при установке «для всех»: задачу с
    /// наивысшими правами без них не снять. А при обновлении поверх задача
    /// обязана остаться — отсюда проверка `$UpdateMode`.
    #[test]
    fn the_uninstaller_drops_the_autostart_task() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let config: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root.join("tauri.conf.json")).expect("конфиг читается"),
        )
        .expect("конфиг — JSON");
        let nsis = &config["bundle"]["windows"]["nsis"];
        assert_eq!(nsis["installMode"], "perMachine", "удалению нужны права");

        let hooks = nsis["installerHooks"].as_str().expect("хуки подключены");
        let hooks = std::fs::read_to_string(root.join(hooks)).expect("файл хуков есть");
        let body = hooks
            .split("!macro NSIS_HOOK_PREUNINSTALL")
            .nth(1)
            .and_then(|rest| rest.split("!macroend").next())
            .expect("хук перед удалением есть");
        assert!(body.contains(&format!("/Delete /TN \"{TASK}\" /F")), "та же задача, что заводит приложение");
        assert!(body.contains("$UpdateMode <> 1"), "обновление поверх задачу не трогает");
    }

    #[test]
    fn failed_delete_needs_confirmed_absence() {
        assert!(confirm_disabled(Err("удаление запрещено".to_owned()), || Err("служба недоступна".to_owned())).is_err());
        assert!(confirm_disabled(Err("удаление запрещено".to_owned()), || Ok(true)).is_err());
        assert!(confirm_disabled(Err("задачи нет".to_owned()), || Ok(false)).is_ok());
        assert!(confirm_disabled(Ok(()), || panic!("успешное удаление не требует нового запроса")).is_ok());
    }

    #[test]
    fn task_presence_has_three_outcomes() {
        assert_eq!(presence_result(Some(0), "").unwrap(), true);
        assert_eq!(presence_result(Some(2), "").unwrap(), false);
        for code in [None, Some(1), Some(5), Some(-1)] {
            assert!(presence_result(code, "проверка не удалась").is_err());
        }
    }

    #[test]
    fn the_task_loses_the_schtasks_defaults() {
        let script = relax_script("name'quoted");
        assert!(script.contains("GetTask('name''quoted')"));
        assert!(script.contains("ExecutionTimeLimit = 'PT0S'"));
        assert!(script.contains("DisallowStartIfOnBatteries = $false"));
        assert!(script.contains("StopIfGoingOnBatteries = $false"));
    }

    #[cfg(windows)]
    #[test]
    fn system_programs_live_in_an_absolute_folder() {
        let path = system_exe("System32\\schtasks.exe");
        assert!(path.is_absolute(), "{}", path.display());
        assert!(path.exists(), "{}", path.display());
    }

    #[test]
    fn a_task_name_stays_a_literal_in_the_probe() {
        assert!(presence_script("name'quoted").contains("GetTask('name''quoted')"));
    }
}
