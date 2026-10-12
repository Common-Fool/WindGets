//! Один пакет ICMP туда и обратно.
//!
//! Через `IcmpSendEcho` из iphlpapi, а не сырым сокетом: сырой ICMP на Windows
//! открывается только с правами администратора, а помощник ICMP работает от
//! обычного пользователя. Приложение и так поднимается без повышения прав, и
//! терять это ради пинга незачем.

use std::net::Ipv4Addr;
use std::time::Duration;

/// Время отклика в миллисекундах. `None` — ответа не было.
#[cfg(windows)]
pub fn echo(target: Ipv4Addr, timeout: Duration) -> Option<f64> {
    use std::time::Instant;

    use windows::Win32::NetworkManagement::IpHelper::{
        IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho, ICMP_ECHO_REPLY,
    };

    /// Столько же шлёт системный `ping`: путь до узла не должен отличаться
    /// из-за размера пакета.
    const PAYLOAD: [u8; 32] = [b'e'; 32];

    /// Ответ, эхо нагрузки и запас на заголовок опций.
    const REPLY_CAPACITY: usize = size_of::<ICMP_ECHO_REPLY>() + PAYLOAD.len() + 8;

    /// Буфер выравниваем: Windows пишет в него структуру, а не просто байты.
    #[repr(C, align(8))]
    struct Reply([u8; REPLY_CAPACITY]);

    let handle = unsafe { IcmpCreateFile() }.ok()?;
    let mut reply = Reply([0; REPLY_CAPACITY]);

    let started = Instant::now();
    // Адрес идёт в сетевом порядке байтов, то есть октетами как есть
    let answers = unsafe {
        IcmpSendEcho(
            handle,
            u32::from_ne_bytes(target.octets()),
            PAYLOAD.as_ptr().cast(),
            PAYLOAD.len() as u16,
            None,
            reply.0.as_mut_ptr().cast(),
            reply.0.len() as u32,
            timeout.as_millis() as u32,
        )
    };
    let elapsed = started.elapsed();

    unsafe {
        let _ = IcmpCloseHandle(handle);
    }

    if answers == 0 {
        return None;
    }

    // Ноль — IP_SUCCESS. Всё остальное это «сеть недоступна», «время вышло»
    // и прочие беды: пакет вернулся, но ответом узла не является.
    let status = unsafe { std::ptr::read(reply.0.as_ptr().cast::<ICMP_ECHO_REPLY>()) }.Status;
    if status != 0 {
        return None;
    }

    // Своими часами, а не полем `RoundTripTime`: оно целое в миллисекундах,
    // и для шлюза это всегда ноль. Накладные расходы на вызов заметно меньше
    // самой быстрой доставки, так что дробная часть тут единственное, что
    // вообще различимо.
    Some(elapsed.as_secs_f64() * 1000.0)
}

#[cfg(not(windows))]
pub fn echo(_target: Ipv4Addr, _timeout: Duration) -> Option<f64> {
    None
}
