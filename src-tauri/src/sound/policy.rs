//! Переключение устройства вывода.
//!
//! Публичного способа сменить устройство по умолчанию в Windows нет вовсе —
//! ни в MMDevice API, ни где-либо ещё. Единственный — недокументированный
//! `IPolicyConfig`, которым пользуется сам системный флайаут. В крейте
//! `windows` его нет, поэтому объявляем сами.
//!
//! ВАЖНО: **у таблицы методов важно только их число и порядок.** Нужный нам
//! `SetDefaultEndpoint` одиннадцатый после `IUnknown`, и десять предыдущих
//! обязаны быть объявлены, иначе вызов уедет в чужой слот — например, в
//! `SetDeviceFormat`. Их аргументы при этом не значат ничего: мы их не зовём,
//! и `*mut c_void` вместо настоящих структур тут не небрежность, а отказ
//! описывать то, что нам не нужно.
//!
//! Интерфейс недокументированный, и однажды он может просто исчезнуть.
//! Поэтому [`Switcher::open`] возвращает `Option`, а виджет умеет жить без
//! переключения: плитка устройства остаётся и показывает, куда идёт звук.

// Десять методов-заглушек существуют ради смещения в таблице и никем не
// зовутся — это их работа, а не забытый код
#![allow(dead_code, non_snake_case)]

use std::ffi::c_void;

use windows::core::{interface, IUnknown, IUnknown_Vtbl, GUID, HRESULT, PCWSTR};
use windows::Win32::Media::Audio::{eCommunications, eConsole, eMultimedia, ERole};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

/// `CPolicyConfigClient` — та же реализация, которой пользуется флайаут.
const POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

#[interface("f8679f50-850a-41cf-9c72-430f290290c8")]
unsafe trait IPolicyConfig: IUnknown {
    fn GetMixFormat(&self, device: PCWSTR, format: *mut *mut c_void) -> HRESULT;
    fn GetDeviceFormat(&self, device: PCWSTR, default: i32, format: *mut *mut c_void) -> HRESULT;
    fn ResetDeviceFormat(&self, device: PCWSTR) -> HRESULT;
    fn SetDeviceFormat(&self, device: PCWSTR, endpoint: *mut c_void, mix: *mut c_void) -> HRESULT;
    fn GetProcessingPeriod(
        &self,
        device: PCWSTR,
        default: i32,
        period: *mut i64,
        minimum: *mut i64,
    ) -> HRESULT;
    fn SetProcessingPeriod(&self, device: PCWSTR, period: *mut i64) -> HRESULT;
    fn GetShareMode(&self, device: PCWSTR, mode: *mut c_void) -> HRESULT;
    fn SetShareMode(&self, device: PCWSTR, mode: *mut c_void) -> HRESULT;
    fn GetPropertyValue(&self, device: PCWSTR, key: *const c_void, value: *mut c_void) -> HRESULT;
    fn SetPropertyValue(&self, device: PCWSTR, key: *const c_void, value: *mut c_void) -> HRESULT;
    /// Единственный, ради которого всё остальное и объявлено.
    fn SetDefaultEndpoint(&self, device: PCWSTR, role: ERole) -> HRESULT;
    fn SetEndpointVisibility(&self, device: PCWSTR, visible: i32) -> HRESULT;
}

pub struct Switcher(IPolicyConfig);

impl Switcher {
    /// `None` — интерфейс не завёлся. Не беда, а причина погасить переключение.
    pub fn open() -> Option<Self> {
        unsafe {
            CoCreateInstance(&POLICY_CONFIG_CLIENT, None, CLSCTX_ALL)
                .ok()
                .map(Self)
        }
    }

    /// Делает устройство выводом по умолчанию.
    ///
    /// Переключаем **всеми тремя ролями**: так делает системный флайаут.
    /// Одной `eConsole` мало — часть приложений спрашивает `eMultimedia` или
    /// `eCommunications`, и звук у них остался бы на прежнем устройстве.
    pub fn make_default(&self, id: &str) -> Result<(), String> {
        // Строка обязана пережить все три вызова, поэтому лежит здесь, а не
        // собирается по месту
        let wide: Vec<u16> = id.encode_utf16().chain(std::iter::once(0)).collect();

        for role in [eConsole, eMultimedia, eCommunications] {
            unsafe { self.0.SetDefaultEndpoint(PCWSTR(wide.as_ptr()), role) }
                .ok()
                .map_err(|error| {
                    format!("устройство не переключилось (роль {}): {error}", role.0)
                })?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Разведка: интерфейс создаётся и его таблица на месте.
    ///
    /// Переключаем на то устройство, которое **уже** стоит по умолчанию —
    /// вызов идемпотентный, а таблицу проверяет целиком: промахнись мы мимо
    /// слота, попали бы в `SetPropertyValue` с чужими аргументами.
    ///
    /// Запуск: `cargo test --manifest-path src-tauri/Cargo.toml -- --ignored`
    #[test]
    #[ignore = "трогает живое устройство вывода"]
    fn takes_the_current_device_as_default() {
        let endpoints = super::super::endpoints::Endpoints::open().expect("COM завёлся");
        let (device, _, _) = endpoints.output().expect("устройство вывода есть");

        let switcher = Switcher::open().expect("IPolicyConfig создался");
        switcher
            .make_default(&device.id)
            .expect("переключение прошло");

        // Устройство осталось тем же: мы поставили его же
        let (after, _, _) = endpoints.output().expect("устройство вывода на месте");
        assert_eq!(after.id, device.id);
    }
}
