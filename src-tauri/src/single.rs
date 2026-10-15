//! Один экземпляр приложения.
//!
//! Два экземпляра опрашивали бы одно железо, писали бы одни и те же файлы
//! (настройки, почту, 2FA) каждый своим снимком и разбирали бы чужие рабочие
//! папки конвертера. Проверка в `scripts/dev.ps1` стережёт только запуск
//! разработки; ярлык установленного приложения при работающей задаче
//! автозапуска её обходит.
//!
//! Устроено двумя именованными объектами ядра в пространстве сеанса (`Local\`):
//! мьютекс держит первый экземпляр всю жизнь, а событием второй просит его
//! показать настройки — человек, нажавший ярлык, хотел увидеть приложение, и
//! тихий выход выглядел бы как «не запускается».
//!
//! ВАЖНО: **второй не уходит сразу, а сперва ждёт мьютекс несколько секунд.**
//! Перезапуск runtime (`request_restart` при потере окон) поднимает новый
//! процесс раньше, чем завершится старый: уйди новый сразу, приложение после
//! перезапуска не осталось бы вовсе. Старый, завершившись, бросает мьютекс, и
//! новый его подхватывает. Настоящий второй запуск ждёт впустую и уходит —
//! настройки к этому времени уже показал первый.
//!
//! Разработка (`debug`) держит свои имена: запуск из `npm run app` рядом с
//! установленной копией прежде не запрещался, и запрещать его заодно незачем.

use std::time::Duration;

use tauri::{AppHandle, Manager};
use ::windows::core::{w, PCWSTR};
use ::windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
use ::windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE, INFINITE,
};

#[cfg(not(debug_assertions))]
const MUTEX: PCWSTR = w!("Local\\io.github.commonfool.windgets.instance");
#[cfg(not(debug_assertions))]
const WAKE: PCWSTR = w!("Local\\io.github.commonfool.windgets.show-settings");
#[cfg(debug_assertions)]
const MUTEX: PCWSTR = w!("Local\\io.github.commonfool.windgets.dev.instance");
#[cfg(debug_assertions)]
const WAKE: PCWSTR = w!("Local\\io.github.commonfool.windgets.dev.show-settings");

/// Сколько второй экземпляр ждёт, не уходит ли первый.
const HANDOVER: Duration = Duration::from_secs(5);

/// Забирает место единственного экземпляра. `false` — работает другой, и этот
/// процесс должен уйти, ничего не трогая.
///
/// Не вышло создать сам мьютекс — запуск не запрещаем: защита от второго
/// экземпляра не повод не подняться вовсе.
pub fn claim() -> bool {
    let mutex = match unsafe { CreateMutexW(None, true, MUTEX) } {
        Ok(mutex) => mutex,
        Err(error) => {
            eprintln!("мьютекс единственного экземпляра не создался: {error}");
            return true;
        }
    };

    // Сразу за вызовом: следующий вызов Win32 перепишет код ошибки
    if unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
        // Описатель не закрываем: мьютекс держится всю жизнь процесса
        return true;
    }

    wake_first();

    let waited = unsafe { WaitForSingleObject(mutex, HANDOVER.as_millis() as u32) };
    waited == WAIT_OBJECT_0 || waited == WAIT_ABANDONED
}

/// Просит работающий экземпляр показать настройки.
fn wake_first() {
    match unsafe { OpenEventW(EVENT_MODIFY_STATE, false, WAKE) } {
        Ok(event) => {
            if let Err(error) = unsafe { SetEvent(event) } {
                eprintln!("первый экземпляр не разбужен: {error}");
            }
        }
        // Первый ещё не завёл событие (сам только поднимается) — будить некого
        Err(error) => eprintln!("событие первого экземпляра не открылось: {error}"),
    }
}

/// Слушает просьбы второго экземпляра: каждая показывает окно настроек.
///
/// Свой поток, вечно спящий в ожидании: события Win32 нечем ждать в цикле
/// Tauri, а поток без работы ничего не стоит.
pub fn listen(app: &AppHandle) {
    let event = match unsafe { CreateEventW(None, false, false, WAKE) } {
        Ok(event) => event,
        Err(error) => {
            eprintln!("событие единственного экземпляра не создалось: {error}");
            return;
        }
    };

    // HANDLE — сырой указатель и между потоками не ходит; ходит его число
    let raw = event.0 as isize;
    let app = app.clone();
    std::thread::spawn(move || {
        let event = HANDLE(raw as *mut _);
        loop {
            if unsafe { WaitForSingleObject(event, INFINITE) } != WAIT_OBJECT_0 {
                return;
            }
            // Будит и новый процесс перезапуска, пока этот уходит: уходящему
            // показывать настройки незачем
            if app.state::<crate::windows::Shutdown>().started() {
                continue;
            }
            crate::windows::on_main(&app, |app| crate::settings_window::show(&app));
        }
    });
}
