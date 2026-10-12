//! Устройства звука средствами MMDevice API.
//!
//! Весь модуль работает только из потока звука ([`super::work`]): COM-объекты
//! принадлежат апартаменту, в котором созданы, и в windows-rs это выражено
//! прямо — они не `Send`.
//!
//! Устройство вывода берём заново на каждое чтение и на каждую правку, а не
//! держим найденное. Оно меняется под нами — и когда его переключает сам
//! виджет, и когда это делает Windows, воткнув наушники, — а промахнуться
//! мимо действующего устройства тут страшнее лишнего вызова: их всё равно
//! два в секунду.

use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, DigitalAudioDisplayDevice, EDataFlow, Headphones, Headset,
    IAudioSessionManager2, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
    PKEY_AudioEndpoint_FormFactor, Speakers, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantClear;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::System::Variant::{VT_LPWSTR, VT_UI4};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;

use super::{Found, Kind};

pub struct Endpoints {
    enumerator: IMMDeviceEnumerator,
}

impl Endpoints {
    /// Заводит COM в этом потоке и создаёт перечислитель.
    ///
    /// `CoUninitialize` не зовём нигде: поток живёт до конца работы
    /// приложения, а разбирать за собой апартамент перед самым выходом
    /// процесса — работа без последствий.
    pub fn open() -> Result<Self, String> {
        unsafe {
            // S_FALSE значит «уже заведён» и отказом не является
            let started = CoInitializeEx(None, COINIT_MULTITHREADED);
            if started.is_err() {
                return Err(format!("COM не завёлся: {started:?}"));
            }

            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .map_err(|error| format!("перечислитель устройств не создался: {error}"))?;

            Ok(Self { enumerator })
        }
    }

    /// Текущее устройство вывода: описание, громкость в процентах и молчание.
    pub fn output(&self) -> Option<(Found, u8, bool)> {
        unsafe {
            let device = self.default(eRender)?;
            let found = describe(&device)?;
            let volume = volume_of(&device)?;

            let level = volume.GetMasterVolumeLevelScalar().ok()?;
            let muted = volume.GetMute().ok()?.as_bool();

            Some((found, to_percent(level), muted))
        }
    }

    /// Текущий микрофон: описание и молчание. Громкость его виджету не нужна.
    pub fn microphone(&self) -> Option<(Found, bool)> {
        unsafe {
            let device = self.default(eCapture)?;
            let found = describe(&device)?;
            let muted = volume_of(&device)?.GetMute().ok()?.as_bool();

            Some((found, muted))
        }
    }

    /// Все включённые устройства вывода в системном порядке — том же, что во
    /// флайауте Windows. Ротация идёт по нему, и своего порядка мы не
    /// придумываем.
    pub fn outputs(&self) -> Vec<Found> {
        self.list(eRender)
    }

    /// То же для устройств ввода: список микрофонов.
    pub fn inputs(&self) -> Vec<Found> {
        self.list(eCapture)
    }

    /// Перечисление включённых устройств одного направления.
    ///
    /// ВАЖНО: зовётся только под раскрытую панель. Это полтора десятка
    /// COM-вызовов на каждое устройство, а держать список наготове незачем:
    /// плитке хватает текущего, а ротация по кругу спрашивает список сама.
    fn list(&self, flow: EDataFlow) -> Vec<Found> {
        unsafe {
            let Ok(collection) = self.enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)
            else {
                return Vec::new();
            };
            let count = collection.GetCount().unwrap_or(0);

            (0..count)
                .filter_map(|index| collection.Item(index).ok())
                .filter_map(|device| describe(&device))
                .collect()
        }
    }

    pub fn set_volume(&self, percent: u8) -> Result<(), String> {
        unsafe {
            let device = self.default(eRender).ok_or("устройства вывода нет")?;
            let volume = volume_of(&device).ok_or("громкость недоступна")?;

            volume
                .SetMasterVolumeLevelScalar(to_scalar(percent), std::ptr::null())
                .map_err(|error| format!("громкость не поставилась: {error}"))
        }
    }

    pub fn set_muted(&self, on: bool) -> Result<(), String> {
        unsafe {
            let device = self.default(eRender).ok_or("устройства вывода нет")?;
            let volume = volume_of(&device).ok_or("громкость недоступна")?;

            volume
                .SetMute(on, std::ptr::null())
                .map_err(|error| format!("молчание не поставилось: {error}"))
        }
    }

    /// Микшер текущего устройства вывода — вход к сессиям приложений.
    ///
    /// Берётся заново, как и всё прочее: устройство меняется под нами, а
    /// сессии принадлежат ему, а не нам.
    pub fn mixer(&self) -> Option<IAudioSessionManager2> {
        unsafe {
            let device = self.default(eRender)?;
            device.Activate(CLSCTX_ALL, None).ok()
        }
    }

    pub fn set_microphone_muted(&self, on: bool) -> Result<(), String> {
        unsafe {
            let device = self.default(eCapture).ok_or("микрофона нет")?;
            let volume = volume_of(&device).ok_or("микрофон не отвечает")?;

            volume
                .SetMute(on, std::ptr::null())
                .map_err(|error| format!("микрофон не замолчал: {error}"))
        }
    }

    /// Устройство по умолчанию. `eConsole` — та роль, которой ведает флайаут:
    /// именно её человек и называет «устройством вывода».
    unsafe fn default(&self, flow: EDataFlow) -> Option<IMMDevice> {
        unsafe { self.enumerator.GetDefaultAudioEndpoint(flow, eConsole).ok() }
    }
}

/// Идентификатор, имя и тип устройства.
unsafe fn describe(device: &IMMDevice) -> Option<Found> {
    unsafe {
        let id = id_of(device)?;
        let store = device.OpenPropertyStore(STGM_READ).ok()?;

        Some(Found {
            id,
            // Имя обязано быть: без него плитка подписана пустотой
            name: text(&store, &PKEY_Device_FriendlyName)?,
            kind: kind_of(number(&store, &PKEY_AudioEndpoint_FormFactor)),
        })
    }
}

/// Строку отдаёт сама Windows, освобождать её нам.
unsafe fn id_of(device: &IMMDevice) -> Option<String> {
    unsafe {
        let raw = device.GetId().ok()?;
        let id = raw.to_string().ok();
        CoTaskMemFree(Some(raw.0.cast()));

        id
    }
}

/// Строковое свойство из хранилища. Забирает и освобождает `PROPVARIANT` сам.
unsafe fn text(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    unsafe {
        let mut value = store.GetValue(key).ok()?;

        // Читаем до очистки, но владение забираем сразу: `PropVariantClear`
        // освободит строку, на которую иначе осталась бы ссылка
        let found = {
            let inner = &value.Anonymous.Anonymous;
            (inner.vt == VT_LPWSTR)
                .then(|| inner.Anonymous.pwszVal.to_string().ok())
                .flatten()
        };

        let _ = PropVariantClear(&mut value);
        found
    }
}

/// Числовое свойство из хранилища.
unsafe fn number(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<u32> {
    unsafe {
        let mut value = store.GetValue(key).ok()?;

        let found = {
            let inner = &value.Anonymous.Anonymous;
            (inner.vt == VT_UI4).then(|| inner.Anonymous.uintVal)
        };

        let _ = PropVariantClear(&mut value);
        found
    }
}

unsafe fn volume_of(device: &IMMDevice) -> Option<IAudioEndpointVolume> {
    unsafe { device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok() }
}

/// Тип устройства по `EndpointFormFactor`.
///
/// Гарнитура идёт наушниками: у неё то же место на голове и тот же значок в
/// системном списке. Всё незнакомое — общим значком: врать формой хуже, чем
/// промолчать.
fn kind_of(form: Option<u32>) -> Kind {
    let Some(form) = form else {
        return Kind::Other;
    };

    match form as i32 {
        value if value == Speakers.0 => Kind::Speakers,
        value if value == Headphones.0 || value == Headset.0 => Kind::Headphones,
        value if value == DigitalAudioDisplayDevice.0 => Kind::Display,
        _ => Kind::Other,
    }
}

/// Доля от нуля до единицы — в проценты.
fn to_percent(scalar: f32) -> u8 {
    (scalar.clamp(0.0, 1.0) * 100.0).round() as u8
}

/// Проценты — обратно в долю.
fn to_scalar(percent: u8) -> f32 {
    f32::from(percent.min(100)) / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_turns_into_percent() {
        assert_eq!(to_percent(0.0), 0);
        assert_eq!(to_percent(1.0), 100);
        assert_eq!(to_percent(0.425), 43);
    }

    /// Windows отдаёт долю, а иногда и чуть за края шкалы: за сотню процентов
    /// полоса вылезла бы из плитки.
    #[test]
    fn percent_never_leaves_the_scale() {
        assert_eq!(to_percent(-0.1), 0);
        assert_eq!(to_percent(1.5), 100);
        assert_eq!(to_scalar(200), 1.0);
    }

    /// Полоса ходит в процентах, а система — в долях. Оборот обязан быть
    /// точным, иначе ползунок сползал бы сам собой на каждом тике опроса.
    #[test]
    fn percent_survives_the_round_trip() {
        for percent in 0..=100_u8 {
            assert_eq!(to_percent(to_scalar(percent)), percent, "сползло на {percent}");
        }
    }

    #[test]
    fn form_factor_picks_the_icon() {
        assert_eq!(kind_of(Some(Speakers.0 as u32)), Kind::Speakers);
        assert_eq!(kind_of(Some(Headphones.0 as u32)), Kind::Headphones);
        // Гарнитура — те же наушники
        assert_eq!(kind_of(Some(Headset.0 as u32)), Kind::Headphones);
        assert_eq!(kind_of(Some(DigitalAudioDisplayDevice.0 as u32)), Kind::Display);
    }

    /// Свойства у устройства может не быть вовсе — значок тогда общий.
    #[test]
    fn missing_form_factor_falls_back() {
        assert_eq!(kind_of(None), Kind::Other);
        assert_eq!(kind_of(Some(999)), Kind::Other);
    }

    /// Сквозная проверка на живом железе: устройство вывода находится, имя у
    /// него непустое, громкость в пределах шкалы.
    ///
    /// Запуск: `cargo test --manifest-path src-tauri/Cargo.toml -- --ignored`
    #[test]
    #[ignore = "нужны живые устройства звука"]
    fn reads_the_real_devices() {
        let endpoints = Endpoints::open().expect("COM завёлся");

        let (device, volume, _) = endpoints.output().expect("устройство вывода есть");
        assert!(!device.name.is_empty(), "имя пустое");
        assert!(!device.id.is_empty(), "идентификатор пустой");
        assert!(volume <= 100, "громкость вне шкалы: {volume}");

        let outputs = endpoints.outputs();
        assert!(!outputs.is_empty(), "список устройств пуст");
        assert!(
            outputs.iter().any(|other| other.id == device.id),
            "текущего устройства нет в списке"
        );

        for output in &outputs {
            println!("вывод: {:?} — {} [{}]", output.kind, output.name, output.id);
        }

        // Список входов собирается тем же перечислением, только в другую
        // сторону: пустой означал бы, что панель микрофона нечем наполнять
        let inputs = endpoints.inputs();
        assert!(!inputs.is_empty(), "список устройств ввода пуст");

        for input in &inputs {
            println!("вход: {:?} — {} [{}]", input.kind, input.name, input.id);
        }
    }

    /// Разведка микрофона: находится ли он и слушается ли `SetMute`.
    ///
    /// Молчание переключается и возвращается обратно, так что микрофон
    /// остаётся в том же состоянии, в каком был.
    #[test]
    #[ignore = "трогает живой микрофон"]
    fn reads_and_toggles_the_real_microphone() {
        let endpoints = Endpoints::open().expect("COM завёлся");

        let (device, before) = endpoints.microphone().expect("микрофон есть");
        println!("микрофон: {} [{}], молчит: {before}", device.name, device.id);

        endpoints
            .set_microphone_muted(!before)
            .expect("молчание поставилось");

        let (_, after) = endpoints.microphone().expect("микрофон на месте");
        println!("после переключения молчит: {after}");

        endpoints
            .set_microphone_muted(before)
            .expect("молчание вернулось");

        assert_eq!(after, !before, "SetMute не подействовал");
    }
}
