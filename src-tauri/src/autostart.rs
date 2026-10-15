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
/// но в разработке он открыт на запись, и полагаться тут не на что. Путь берём
/// от `SystemRoot`; если переменной нет, остаётся прежнее поведение — хуже
/// не станет.
///
/// Общая на всё приложение: тем же путём идут `tar.exe` у компонентов и
/// `explorer.exe` у «показать в папке».
#[cfg(windows)]
pub(crate) fn system_exe(relative: &str) -> std::path::PathBuf {
    match std::env::var_os("SystemRoot") {
        Some(root) => std::path::Path::new(&root).join(relative),
        None => std::path::PathBuf::from(relative),
    }
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

    // Планировщик пишет по-русски в кодировке консоли, и осмысленной строки из
    // этого не собрать: отдаём код возврата, а подробности уходят в лог
    Err(format!("schtasks вернул {}", outcome.status))
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
    ])
}

/// Убирает задачу. Отсутствие задачи ошибкой не считаем: цель достигнута.
pub fn disable() -> Result<(), String> {
    if !is_enabled() {
        return Ok(());
    }

    schtasks(&["/Delete", "/TN", TASK, "/F"])
}

/// Есть ли задача в планировщике.
pub fn is_enabled() -> bool {
    schtasks(&["/Query", "/TN", TASK]).is_ok()
}

/// Кем задача будет выполняться — нынешний пользователь с доменом.
///
/// Повышение прав пользователя не меняет, поэтому спрашивать окружение можно
/// и из привилегированного процесса.
fn whoami() -> String {
    let name = std::env::var("USERNAME").unwrap_or_default();

    match std::env::var("USERDOMAIN") {
        Ok(domain) if !domain.is_empty() && !name.is_empty() => format!("{domain}\\{name}"),
        _ => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Имя пользователя обязано собираться в вид, который принимает `/RU`.
    #[test]
    fn the_user_is_named_the_way_schtasks_expects() {
        let user = whoami();
        assert!(!user.is_empty(), "имя пользователя не собралось");
        assert!(
            !user.starts_with('\\') && !user.ends_with('\\'),
            "разделитель домена повис без половины: {user}"
        );
    }

    /// Круг «завести — проверить — убрать» на живом планировщике.
    ///
    /// Требует прав администратора: задачу с наивысшими правами иначе не
    /// зарегистрировать. Задачу за собой убирает.
    #[test]
    #[ignore = "трогает живой планировщик задач, нужны права администратора"]
    fn the_task_appears_and_goes() {
        let had = is_enabled();

        enable().expect("задача заводится");
        assert!(is_enabled(), "заведённой задачи не видно");

        disable().expect("задача убирается");
        assert!(!is_enabled(), "задача осталась после удаления");

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
}
