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
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

/// Процесс создаётся спящим (`CREATE_SUSPENDED`) и просыпается уже в задании.
pub const CREATE_SUSPENDED: u32 = 0x0000_0004;

/// Хендл хранится числом: `HANDLE` — голый указатель, и с ним `Spawned`
/// нельзя было бы даже подержать в чужом потоке.
pub struct Job(isize);

impl Job {
    /// Заводит задание и кладёт в него процесс.
    ///
    /// Процесс к этому времени спит (`CREATE_SUSPENDED`) и будится [`resume`]
    /// уже из задания: иначе потомок, рождённый между запуском и этим вызовом,
    /// в задание не попал бы и пережил бы отмену.
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

/// Будит процесс, созданный спящим. Поток у такого ровно один, но хендла на
/// него `std::process::Child` не отдаёт — находим его снимком потоков системы.
///
/// `false` — не разбудили: процесс остался бы спать вечно, и зовущий его гасит.
pub fn resume(child: &Child) -> bool {
    let process = child.id();
    let mut woken = false;
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) else {
            return false;
        };
        let mut entry = THREADENTRY32 { dwSize: std::mem::size_of::<THREADENTRY32>() as u32, ..Default::default() };
        let mut found = Thread32First(snapshot, &mut entry).is_ok();
        while found {
            if entry.th32OwnerProcessID == process {
                if let Ok(thread) = OpenThread(THREAD_SUSPEND_RESUME, false, entry.th32ThreadID) {
                    woken |= ResumeThread(thread) != u32::MAX;
                    let _ = CloseHandle(thread);
                }
            }
            found = Thread32Next(snapshot, &mut entry).is_ok();
        }
        let _ = CloseHandle(snapshot);
    }
    woken
}

impl Drop for Job {
    fn drop(&mut self) {
        // Закрытие последнего хендла само гасит всё, что осталось в задании
        unsafe {
            let _ = CloseHandle(self.handle());
        }
    }
}
