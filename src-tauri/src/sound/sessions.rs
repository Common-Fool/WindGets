//! Микшер громкости: кто именно сейчас звучит и как громко.
//!
//! Работает только из потока звука ([`super::work`]) — как и всё остальное
//! здесь, это COM, а его объекты принадлежат апартаменту, в котором созданы.
//!
//! Сессии берём у **текущего** устройства вывода и заново на каждый заход, по
//! той же причине, по которой так берётся громкость: устройство меняется под
//! нами, а промахнуться мимо действующего страшнее лишнего вызова.
//!
//! ВАЖНО: **строка микшера — это приложение, а не сессия.** Браузер заводит их
//! по одной на вкладку со звуком, и порознь они дали бы десяток одинаковых
//! строк «msedge». Системный микшер показывает одну — и мы тоже: сессии
//! складываются по имени процесса, а команда идёт сразу во все сессии этого
//! имени, иначе половина вкладок осталась бы громкой.

use std::collections::BTreeMap;

use windows::core::Interface;
use windows::Win32::Media::Audio::{
    AudioSessionStateExpired, IAudioSessionControl2, IAudioSessionManager2, ISimpleAudioVolume,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::Track;

/// Имя строки для системных звуков: у их сессии процесса нет вовсе.
///
/// По-английски, как и весь текст в карточках. Имена самих программ рядом
/// приходят от Windows и остаются на языке системы — переводить их нечем.
const SYSTEM_SOUNDS: &str = "System sounds";

/// Чем подписать программу, у которой не спросить имени.
const UNKNOWN: &str = "Application";

/// Звучащие программы текущего устройства вывода.
///
/// Пустой список — не отказ: молчащая машина это обычное дело, а сессии
/// заводятся только под играющий звук.
pub fn tracks(manager: &IAudioSessionManager2) -> Vec<Track> {
    // Складываем по имени, сохраняя порядок первого появления не нужно:
    // в карточке строки всё равно идут по имени, а не по времени
    let mut found: BTreeMap<String, Track> = BTreeMap::new();

    for session in sessions(manager) {
        let Some((name, volume, muted)) = describe(&session) else {
            continue;
        };

        // Первая сессия задаёт громкость строки: у вкладок одного браузера
        // она общая, и разбирать их порознь незачем
        found.entry(name.clone()).or_insert(Track {
            id: name.clone(),
            name,
            volume,
            muted,
            path: path_of(&session),
        });
    }

    found.into_values().collect()
}

/// Ставит громкость всем сессиям приложения.
pub fn set_volume(manager: &IAudioSessionManager2, id: &str, percent: u8) -> Result<(), String> {
    let level = f32::from(percent.min(100)) / 100.0;

    each(manager, id, |volume| unsafe {
        volume.SetMasterVolume(level, std::ptr::null())?;
        // Поднятая полоса возвращает звук, как у общей громкости. Одной
        // громкости мало: заглушённая сессия иначе так и осталась бы молчать.
        if percent > 0 {
            volume.SetMute(false, std::ptr::null())?;
        }
        Ok(())
    })
}

/// Молчание — тоже всем сессиям приложения.
pub fn set_muted(manager: &IAudioSessionManager2, id: &str, on: bool) -> Result<(), String> {
    each(manager, id, |volume| unsafe {
        volume.SetMute(on, std::ptr::null())
    })
}

/// Проходит по всем сессиям приложения и делает с каждой одно и то же.
///
/// Отказ одной сессии не отменяет остальных: программа могла закрыть вкладку
/// прямо между перечислением и правкой.
fn each<F>(manager: &IAudioSessionManager2, id: &str, act: F) -> Result<(), String>
where
    F: Fn(&ISimpleAudioVolume) -> windows::core::Result<()>,
{
    let mut touched = 0_u32;

    for session in sessions(manager) {
        if name_of(&session).as_deref() != Some(id) {
            continue;
        }

        let Ok(volume) = session.cast::<ISimpleAudioVolume>() else {
            continue;
        };
        if act(&volume).is_ok() {
            touched += 1;
        }
    }

    if touched == 0 {
        return Err(format!("«{id}» больше не звучит"));
    }
    Ok(())
}

/// Живые сессии устройства.
///
/// Отжившие (`Expired`) отбрасываем сразу: программа уже закрылась, а строка
/// от неё осталась бы висеть до перезапуска микшера.
fn sessions(manager: &IAudioSessionManager2) -> Vec<IAudioSessionControl2> {
    unsafe {
        let Ok(list) = manager.GetSessionEnumerator() else {
            return Vec::new();
        };
        let count = list.GetCount().unwrap_or(0);

        (0..count)
            .filter_map(|index| list.GetSession(index).ok())
            .filter(|session| session.GetState() != Ok(AudioSessionStateExpired))
            .filter_map(|session| session.cast::<IAudioSessionControl2>().ok())
            .collect()
    }
}

/// Имя, громкость и молчание одной сессии.
fn describe(session: &IAudioSessionControl2) -> Option<(String, u8, bool)> {
    let name = name_of(session)?;
    let volume = session.cast::<ISimpleAudioVolume>().ok()?;

    unsafe {
        let level = volume.GetMasterVolume().ok()?;
        let muted = volume.GetMute().ok()?.as_bool();

        Some((name, (level * 100.0).round().clamp(0.0, 100.0) as u8, muted))
    }
}

/// Имя приложения для подписи строки.
///
/// У системных звуков процесса нет, и спрашивать путь не у кого — их узнаём по
/// нулевому идентификатору.
fn name_of(session: &IAudioSessionControl2) -> Option<String> {
    let pid = unsafe { session.GetProcessId() }.ok()?;
    if pid == 0 {
        return Some(SYSTEM_SOUNDS.to_owned());
    }

    let Some(path) = image_path(pid) else {
        return Some(UNKNOWN.to_owned());
    };

    Some(short_name(&path))
}

/// Путь к exe той программы, чья это сессия. Нужен ради значка.
fn path_of(session: &IAudioSessionControl2) -> Option<String> {
    let pid = unsafe { session.GetProcessId() }.ok()?;
    (pid != 0).then(|| image_path(pid)).flatten()
}

/// Полный путь к образу процесса.
///
/// `None` у защищённых процессов: хендла к ним не дают вовсе, и это не беда —
/// строка останется без значка, а не пропадёт.
fn image_path(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;

        let mut buffer = [0u16; 260];
        let mut length = buffer.len() as u32;
        let outcome = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut length,
        );

        // Хендл закрываем в любом случае: сессий за тик десяток
        let _ = windows::Win32::Foundation::CloseHandle(handle);
        outcome.ok()?;

        Some(String::from_utf16_lossy(&buffer[..length as usize]))
    }
}

/// «C:\\...\\msedge.exe» → «msedge».
///
/// Расширение убираем по той же причине, что в виджете процессов: место в
/// строке дороже, а различает оно ничего.
fn short_name(path: &str) -> String {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);

    match file.rsplit_once('.') {
        Some((head, tail)) if !head.is_empty() && tail.eq_ignore_ascii_case("exe") => {
            head.to_owned()
        }
        _ => file.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_comes_from_the_path() {
        assert_eq!(short_name(r"C:\Program Files\Spotify\Spotify.exe"), "Spotify");
        assert_eq!(short_name(r"C:\Windows\System32\msedge.EXE"), "msedge");
    }

    /// Путь без каталога и имя без расширения тоже должны читаться: строка
    /// приходит от системы, и полагаться на её вид нельзя.
    #[test]
    fn odd_paths_still_give_a_name() {
        assert_eq!(short_name("game"), "game");
        assert_eq!(short_name(r"D:\games\dota2"), "dota2");
        assert_eq!(short_name(".exe"), ".exe");
    }
}
