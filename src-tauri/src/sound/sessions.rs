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
    AudioSessionStateExpired, IAudioSessionControl, IAudioSessionControl2, IAudioSessionManager2,
    ISimpleAudioVolume,
};
use windows::Win32::Foundation::S_OK;
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::Track;


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
            .filter_map(live_session)
            .collect()
    }
}

fn live_session(session: IAudioSessionControl) -> Option<IAudioSessionControl2> {
    if unsafe { session.GetState() } == Ok(AudioSessionStateExpired) {
        return None;
    }
    session.cast().ok()
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
/// Системность проверяем до pid: у системных звуков процесса нет, и Windows
/// может не отдать его номер вовсе.
fn name_of(session: &IAudioSessionControl2) -> Option<String> {
    // S_FALSE тоже успешный HRESULT, но системной сессией считается только
    // S_OK. Проверка по pid теряла системные звуки при отказе GetProcessId.
    if unsafe { session.IsSystemSoundsSession() } == S_OK {
        return Some(tr!("sound.systemSounds"));
    }
    let pid = unsafe { session.GetProcessId() }.ok()?;

    // Имени не спросить (защищённый процесс, путь длиннее буфера): строка
    // своя на процесс, с его номером. Под одним общим именем все такие
    // программы слились бы в одну строку и крутились бы одной громкостью
    let Some(path) = image_path(pid) else {
        return Some(format!("{} ({pid})", tr!("sound.application")));
    };

    Some(short_name(&path))
}

/// Путь к exe той программы, чья это сессия. Нужен ради значка.
fn path_of(session: &IAudioSessionControl2) -> Option<String> {
    if unsafe { session.IsSystemSoundsSession() } == S_OK {
        return None;
    }
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
    use windows::core::{implement, HRESULT};
    use windows::Win32::Foundation::S_FALSE;
    use windows::Win32::Media::Audio::{
        AudioSessionState, AudioSessionStateActive, AudioSessionStateInactive,
        IAudioSessionControl_Impl, IAudioSessionControl2_Impl, IAudioSessionEvents,
    };

    // Настоящий COM-интерфейс без устройства и процесса. Отсутствующий pid
    // возвращает отказ: системную сессию нельзя узнавать через него.
    #[implement(IAudioSessionControl2)]
    struct TestSession {
        state: AudioSessionState,
        system: HRESULT,
        pid: Option<u32>,
    }

    #[allow(non_snake_case, unused_variables)]
    impl IAudioSessionControl_Impl for TestSession_Impl {
        fn GetState(&self) -> windows_core::Result<AudioSessionState> { Ok(self.state) }
        fn GetDisplayName(&self) -> windows_core::Result<windows_core::PWSTR> {
            Err(windows::Win32::Foundation::E_NOTIMPL.into())
        }
        fn SetDisplayName(&self, value: &windows_core::PCWSTR, eventcontext: *const windows_core::GUID) -> windows_core::Result<()> {
            Err(windows::Win32::Foundation::E_NOTIMPL.into())
        }
        fn GetIconPath(&self) -> windows_core::Result<windows_core::PWSTR> {
            Err(windows::Win32::Foundation::E_NOTIMPL.into())
        }
        fn SetIconPath(&self, value: &windows_core::PCWSTR, eventcontext: *const windows_core::GUID) -> windows_core::Result<()> {
            Err(windows::Win32::Foundation::E_NOTIMPL.into())
        }
        fn GetGroupingParam(&self) -> windows_core::Result<windows_core::GUID> {
            Err(windows::Win32::Foundation::E_NOTIMPL.into())
        }
        fn SetGroupingParam(&self, r#override: *const windows_core::GUID, eventcontext: *const windows_core::GUID) -> windows_core::Result<()> {
            Err(windows::Win32::Foundation::E_NOTIMPL.into())
        }
        fn RegisterAudioSessionNotification(&self, newnotifications: windows_core::Ref<'_, IAudioSessionEvents>) -> windows_core::Result<()> {
            Err(windows::Win32::Foundation::E_NOTIMPL.into())
        }
        fn UnregisterAudioSessionNotification(&self, newnotifications: windows_core::Ref<'_, IAudioSessionEvents>) -> windows_core::Result<()> {
            Err(windows::Win32::Foundation::E_NOTIMPL.into())
        }
    }

    #[allow(non_snake_case, unused_variables)]
    impl IAudioSessionControl2_Impl for TestSession_Impl {
        fn GetSessionIdentifier(&self) -> windows_core::Result<windows_core::PWSTR> {
            Err(windows::Win32::Foundation::E_NOTIMPL.into())
        }
        fn GetSessionInstanceIdentifier(&self) -> windows_core::Result<windows_core::PWSTR> {
            Err(windows::Win32::Foundation::E_NOTIMPL.into())
        }
        fn GetProcessId(&self) -> windows_core::Result<u32> { self.pid.ok_or_else(|| windows::Win32::Foundation::E_FAIL.into()) }
        fn IsSystemSoundsSession(&self) -> windows_core::HRESULT { self.system }
        fn SetDuckingPreference(&self, optout: windows_core::BOOL) -> windows_core::Result<()> {
            Err(windows::Win32::Foundation::E_NOTIMPL.into())
        }
    }

    fn session(state: AudioSessionState, system: HRESULT, pid: Option<u32>) -> IAudioSessionControl2 {
        TestSession { state, system, pid }.into()
    }

    #[test]
    fn system_sounds_are_identified_before_asking_for_a_process() {
        let session = session(AudioSessionStateActive, S_OK, None);
        assert_eq!(name_of(&session), Some(tr!("sound.systemSounds")));
        assert_eq!(path_of(&session), None);
    }

    /// S_FALSE не означает системные звуки, хотя HRESULT успешный. Нулевой
    /// pid сам по себе тоже не даёт права назвать сессию системной.
    #[test]
    fn a_successful_non_system_status_does_not_mean_system_sounds() {
        let session = session(AudioSessionStateActive, S_FALSE, Some(0));
        assert_eq!(name_of(&session), Some(format!("{} (0)", tr!("sound.application"))));
    }

    /// Закрытая программа может ещё перечисляться Windows. Через тот же
    /// фильтр, что стоит в микшере, должны проходить только живые сессии;
    /// неактивная остаётся, чтобы громкость программы можно было вернуть.
    #[test]
    fn expired_sessions_are_filtered_before_their_process_is_queried() {
        let found: Vec<_> = [AudioSessionStateActive, AudioSessionStateExpired, AudioSessionStateInactive]
            .into_iter()
            .map(|state| session(state, S_FALSE, None).cast::<IAudioSessionControl>().unwrap())
            .filter_map(live_session)
            .map(|session| unsafe { session.GetState() }.unwrap())
            .collect();
        assert_eq!(found, vec![AudioSessionStateActive, AudioSessionStateInactive]);
    }

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
