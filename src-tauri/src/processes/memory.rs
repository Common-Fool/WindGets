//! Частный рабочий набор процесса — единственный вызов Windows в этом виджете.
//!
//! ВАЖНО: **это не то же, что отдаёт sysinfo.** Он берёт `WorkingSetSize` —
//! рабочий набор целиком, вместе с общими страницами: системными
//! библиотеками, движком браузера, шрифтами. Каждый процесс считает их своими,
//! и группа из тридцати вкладок насчитывает одни и те же библиотеки тридцать
//! раз. Поймано сверкой с диспетчером задач: у него в колонке «Память» стояло
//! 3,1 ГБ там, где виджет показывал 4,8.
//!
//! `PrivateWorkingSetSize` — ровно та колонка. Общие страницы в него не идут,
//! поэтому суммы по группе честны.
//!
//! Считает это `GetProcessMemoryInfo` со структурой `EX2` (Windows 10 и
//! новее), а ей нужен хендл процесса — по одному открытию на процесс за тик.
//! Диспетчер задач делает то же самое; на двух секундах шага это незаметно.

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX2,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

/// Частный рабочий набор процесса в байтах.
///
/// `None` — хендл не дали или счётчики не пришли. Так отвечают защищённые
/// процессы вроде «Система безопасности»: у них не спрашивают ничего и
/// обычные права тут не помогут.
pub fn private_working_set(pid: u32) -> Option<u64> {
    unsafe {
        // Прав просим самый минимум: полного доступа к чужому процессу для
        // чтения счётчиков не нужно, а с ним не открылась бы и половина
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;

        let mut counters = PROCESS_MEMORY_COUNTERS_EX2::default();
        let size = size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32;
        // Размером `cb` вызов и отличает EX2 от прежних: сама функция объявлена
        // через базовую структуру и разбирается по нему
        counters.cb = size;

        let outcome = GetProcessMemoryInfo(
            handle,
            &mut counters as *mut PROCESS_MEMORY_COUNTERS_EX2 as *mut PROCESS_MEMORY_COUNTERS,
            size,
        );

        // Хендл закрываем в любом случае: триста утечек за тик кончились бы
        // быстро
        let _ = CloseHandle(handle);

        outcome.ok()?;
        Some(counters.PrivateWorkingSetSize as u64)
    }
}
