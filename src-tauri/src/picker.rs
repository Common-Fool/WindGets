//! Окно выбора файлов или папки — системное `IFileOpenDialog`.
//!
//! Плагина диалогов у приложения нет, и заводить его ради двух кнопок незачем:
//! окно Windows достаётся тем же крейтом `windows`, что и остальное.
//!
//! ВАЖНО: **окно живёт в своём потоке со своим STA, а не в потоке команды.**
//! Модальное окно крутит свою очередь сообщений, пока человек выбирает, — а
//! синхронная команда Tauri выполняется в главном потоке, где WebView2
//! принимает вызовы всех окон. Показанное оттуда, оно заперло бы приложение на
//! всё время выбора. Тот же довод увёл COM звука и WinRT медиа в свои потоки, а
//! открытие ссылок — в отдельный процесс проводника.
//!
//! Владелец окна — окно виджета или настроек: так выбор встаёт над ним, а не
//! под виджетами «поверх всех».

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// Что выбирают.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    /// Несколько существующих файлов.
    Files,
    /// Одна папка.
    Folder,
}

/// Окно выбора уже открыто. Второе поверх первого только путало бы: оба
/// модальны к разным владельцам, и какой ответ куда — не угадать.
static PICKING: AtomicBool = AtomicBool::new(false);

/// Открывает окно выбора и отдаёт выбранное в `done` — из своего потока.
///
/// Отмена и любая беда приходят пустым списком: зовущему нечего с ними делать,
/// кроме как ничего не делать. Беда при этом пишется в лог.
pub fn pick<F>(owner: isize, what: Pick, start: Option<PathBuf>, done: F)
where
    F: FnOnce(Vec<PathBuf>) + Send + 'static,
{
    if PICKING.swap(true, Ordering::SeqCst) {
        return;
    }

    std::thread::spawn(move || {
        #[cfg(windows)]
        let chosen = match show(owner, what, start) {
            Ok(chosen) => chosen,
            Err(error) => {
                eprintln!("окно выбора не открылось: {error}");
                Vec::new()
            }
        };
        #[cfg(not(windows))]
        let chosen = {
            let _ = (owner, what, start);
            Vec::new()
        };

        PICKING.store(false, Ordering::SeqCst);
        done(chosen);
    });
}

#[cfg(windows)]
fn show(owner: isize, what: Pick, start: Option<PathBuf>) -> windows::core::Result<Vec<PathBuf>> {
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
    };

    unsafe {
        // Окно выбора — клиент оболочки, а ей нужна однопоточная квартира
        CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).ok()?;
        let result = (|| {
            let dialog: windows::Win32::UI::Shell::IFileOpenDialog = CoCreateInstance(
                &windows::Win32::UI::Shell::FileOpenDialog,
                None,
                CLSCTX_INPROC_SERVER,
            )?;
            ask(&dialog, owner, what, start)
        })();
        CoUninitialize();
        result
    }
}

#[cfg(windows)]
unsafe fn ask(
    dialog: &windows::Win32::UI::Shell::IFileOpenDialog,
    owner: isize,
    what: Pick,
    start: Option<PathBuf>,
) -> windows::core::Result<Vec<PathBuf>> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::{HRESULT, PCWSTR};
    use windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
    use windows::Win32::System::Com::{CoTaskMemFree, IBindCtx};
    use windows::Win32::UI::Shell::{
        IShellItem, SHCreateItemFromParsingName, FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST,
        FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
    };

    let mut options = dialog.GetOptions()? | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST;
    options |= match what {
        Pick::Files => FOS_ALLOWMULTISELECT | FOS_FILEMUSTEXIST,
        Pick::Folder => FOS_PICKFOLDERS,
    };
    dialog.SetOptions(options)?;

    // Начальная папка — удобство, а не условие: её могли удалить
    if let Some(start) = start.filter(|dir| dir.is_dir()) {
        let wide: Vec<u16> = start.as_os_str().encode_wide().chain([0]).collect();
        if let Ok(item) =
            SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(wide.as_ptr()), None::<&IBindCtx>)
        {
            let _ = dialog.SetFolder(&item);
        }
    }

    let owner = (owner != 0).then_some(HWND(owner as *mut std::ffi::c_void));
    if let Err(error) = dialog.Show(owner) {
        // Закрытое крестиком окно — не беда, а ответ «ничего»
        if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) {
            return Ok(Vec::new());
        }
        return Err(error);
    }

    let path_of = |item: &IShellItem| -> windows::core::Result<Option<PathBuf>> {
        let name = item.GetDisplayName(SIGDN_FILESYSPATH)?;
        let path = name.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(name.0 as *const std::ffi::c_void));
        Ok(path)
    };

    let mut chosen = Vec::new();
    match what {
        Pick::Folder => {
            if let Some(path) = path_of(&dialog.GetResult()?)? {
                chosen.push(path);
            }
        }
        Pick::Files => {
            let items = dialog.GetResults()?;
            for index in 0..items.GetCount()? {
                if let Some(path) = path_of(&items.GetItemAt(index)?)? {
                    chosen.push(path);
                }
            }
        }
    }
    Ok(chosen)
}
