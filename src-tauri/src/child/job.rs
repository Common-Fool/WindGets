//! Объект задания Windows: гасит программу вместе со всеми её потомками.
//!
//! `yt-dlp.exe` одним файлом — это распаковщик, который сам запускает второй
//! процесс, а тот запускает ffmpeg. `Child::kill` достаёт только первого, и
//! отменённая загрузка продолжала бы качать и склеивать без хозяина. Задание с
//! `KILL_ON_JOB_CLOSE` гасит всё дерево разом — по отмене и по закрытию
//! последнего хендла, то есть и при падении самого приложения.
//!
//! Единственный файл модуля, который ходит в Windows.

use std::ffi::c_void;
use std::os::windows::io::AsRawHandle;
use std::process::Child;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

/// Хендл хранится числом: `HANDLE` — голый указатель, и с ним `Spawned`
/// нельзя было бы даже подержать в чужом потоке.
pub struct Job(isize);

impl Job {
    /// Заводит задание и кладёт в него уже запущенный процесс.
    ///
    /// Между запуском и этим вызовом процесс уже работает, и потомок, рождённый
    /// в этот промежуток, в задание не попал бы. Не страшно: ни ffmpeg, ни
    /// yt-dlp никого не порождают в первые миллисекунды.
    pub fn adopt(child: &Child) -> windows::core::Result<Self> {
        unsafe {
            let handle = CreateJobObjectW(None, PCWSTR::null())?;
            let job = Job(handle.0 as isize);

            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )?;

            AssignProcessToJobObject(handle, HANDLE(child.as_raw_handle()))?;
            Ok(job)
        }
    }

    fn handle(&self) -> HANDLE {
        HANDLE(self.0 as *mut c_void)
    }

    pub fn terminate(&self) {
        unsafe {
            let _ = TerminateJobObject(self.handle(), 1);
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        // Закрытие последнего хендла само гасит всё, что осталось в задании
        unsafe {
            let _ = CloseHandle(self.handle());
        }
    }
}
