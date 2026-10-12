//! Код TOTP по RFC 6238: HMAC от номера окна, усечённый до шести-восьми цифр.
//!
//! ВАЖНО: **ошибка тут молчит.** Неверный код неотличим от «сервис не принял»,
//! обратной связи у виджета нет никакой, и чинить человек пойдёт сервис, а не
//! нас. Поэтому вся арифметика закрыта векторами RFC 6238 — это единственная
//! обратная связь, которая в этом файле вообще есть.
//!
//! Алгоритмов три, и все три считаются по-настоящему: `sha2` в зависимостях уже
//! был, каждая лишняя ветка стоит одной строки. Молчаливая подмена чужого
//! алгоритма на SHA-1 давала бы всегда неверный код без единого объяснения.

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Sha256, Sha512};

/// Длина кода по умолчанию: столько отдают почти все сервисы.
pub const DEFAULT_DIGITS: u8 = 6;

/// Окно по умолчанию, секунд.
pub const DEFAULT_PERIOD: u32 = 30;

/// Границы длины кода. Меньше шести не бывает, больше восьми не отдаёт никто,
/// а девять уже не влезает в те 31 бит, что остаются после усечения.
pub const MIN_DIGITS: u8 = 6;
pub const MAX_DIGITS: u8 = 8;

/// Хеш, на котором сервис считает свои коды.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Algorithm {
    #[default]
    Sha1,
    Sha256,
    Sha512,
}

impl Algorithm {
    /// Имя из ссылки `otpauth://`. Регистр там бывает любой.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_uppercase().as_str() {
            "SHA1" => Some(Self::Sha1),
            "SHA256" => Some(Self::Sha256),
            "SHA512" => Some(Self::Sha512),
            _ => None,
        }
    }
}

/// Форма кода: сколько цифр, какое окно, каким хешем.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shape {
    pub digits: u8,
    pub period: u32,
    pub algorithm: Algorithm,
}

impl Default for Shape {
    fn default() -> Self {
        Self {
            digits: DEFAULT_DIGITS,
            period: DEFAULT_PERIOD,
            algorithm: Algorithm::Sha1,
        }
    }
}

impl Shape {
    /// Приводит в диапазон то, что пришло из ссылки или из правленного руками
    /// файла. Ноль в окне — деление на ноль, а не «поменьше».
    pub fn sane(self) -> Self {
        Self {
            digits: self.digits.clamp(MIN_DIGITS, MAX_DIGITS),
            period: if self.period == 0 {
                DEFAULT_PERIOD
            } else {
                self.period
            },
            algorithm: self.algorithm,
        }
    }
}

/// Код на заданное мгновение и момент, когда он сменится.
///
/// Второе число отдаётся наружу не для красоты: по нему окно ставит свой
/// будильник, и смена кода с концом кольца обязаны быть одним и тем же
/// мгновением, а не двумя близкими.
pub fn at(secret: &[u8], shape: &Shape, now: i64) -> (String, i64) {
    let shape = shape.sane();
    let period = i64::from(shape.period);

    // Часы раньше эпохи бывают у севшей батарейки BIOS; окно тогда нулевое
    let seconds = now.max(0);
    let counter = (seconds / period) as u64;
    let valid_until = (seconds / period + 1) * period;

    (hotp(secret, counter, shape.digits, shape.algorithm), valid_until)
}

/// HOTP по RFC 4226 — то же самое, но от номера, а не от времени.
///
/// Отдельной функцией именно ради векторов: в RFC 6238 они заданы счётчиком,
/// и проверять через время значило бы проверять заодно и деление.
fn hotp(secret: &[u8], counter: u64, digits: u8, algorithm: Algorithm) -> String {
    let message = counter.to_be_bytes();

    // Ключ у HMAC любой длины, поэтому отказа тут быть не может; три ветки
    // вместо обобщения — у Hmac<D> границы типов длиннее самого кода
    macro_rules! sign {
        ($hash:ty) => {{
            let mut mac = Hmac::<$hash>::new_from_slice(secret)
                .expect("HMAC принимает ключ любой длины");
            mac.update(&message);
            mac.finalize().into_bytes().to_vec()
        }};
    }

    let mac = match algorithm {
        Algorithm::Sha1 => sign!(Sha1),
        Algorithm::Sha256 => sign!(Sha256),
        Algorithm::Sha512 => sign!(Sha512),
    };

    // Динамическое усечение: младшие четыре бита последнего байта говорят,
    // откуда брать четыре байта кода
    let offset = (mac[mac.len() - 1] & 0x0f) as usize;
    let binary = (u32::from(mac[offset] & 0x7f) << 24)
        | (u32::from(mac[offset + 1]) << 16)
        | (u32::from(mac[offset + 2]) << 8)
        | u32::from(mac[offset + 3]);

    let width = usize::from(digits);
    // Ведущие нули — часть кода: «012345» теряет ноль в одном случае из десяти
    format!("{:0width$}", binary % 10u32.pow(u32::from(digits)), width = width)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Семена векторов RFC 6238: та же строка, добитая до длины блока хеша.
    const SEED_SHA1: &[u8] = b"12345678901234567890";
    const SEED_SHA256: &[u8] = b"12345678901234567890123456789012";
    const SEED_SHA512: &[u8] =
        b"1234567890123456789012345678901234567890123456789012345678901234";

    /// Мгновения из таблицы RFC 6238.
    const MOMENTS: [i64; 6] = [59, 1111111109, 1111111111, 1234567890, 2000000000, 20000000000];

    fn eight(secret: &[u8], algorithm: Algorithm, now: i64) -> String {
        let shape = Shape {
            digits: 8,
            period: 30,
            algorithm,
        };
        at(secret, &shape, now).0
    }

    /// Вся таблица RFC 6238 на SHA-1. Держит вывод счётчика, усечение,
    /// порядок байт и остаток разом — ради этого теста HMAC и взят крейтом.
    #[test]
    fn rfc6238_vectors_sha1() {
        let expected = [
            "94287082", "07081804", "14050471", "89005924", "69279037", "65353130",
        ];

        for (moment, want) in MOMENTS.iter().zip(expected) {
            assert_eq!(
                eight(SEED_SHA1, Algorithm::Sha1, *moment),
                want,
                "SHA-1, мгновение {moment}"
            );
        }
    }

    /// Те же мгновения на своих семенах: переключатель алгоритма обязан
    /// переключать, а не считать всё SHA-1 молча.
    #[test]
    fn rfc6238_vectors_sha256_and_sha512() {
        let sha256 = [
            "46119246", "68084774", "67062674", "91819424", "90698825", "77737706",
        ];
        let sha512 = [
            "90693936", "25091201", "99943326", "93441116", "38618901", "47863826",
        ];

        for (moment, want) in MOMENTS.iter().zip(sha256) {
            assert_eq!(
                eight(SEED_SHA256, Algorithm::Sha256, *moment),
                want,
                "SHA-256, мгновение {moment}"
            );
        }

        for (moment, want) in MOMENTS.iter().zip(sha512) {
            assert_eq!(
                eight(SEED_SHA512, Algorithm::Sha512, *moment),
                want,
                "SHA-512, мгновение {moment}"
            );
        }
    }

    /// Потеря ведущего нуля видна в одном случае из десяти и без теста живёт
    /// годами: код просто «иногда не подходит».
    #[test]
    fn a_short_code_keeps_its_leading_zeros() {
        let shape = Shape::default();

        for moment in 0..2000i64 {
            let (code, _) = at(SEED_SHA1, &shape, moment);
            assert_eq!(code.len(), 6, "код на {moment} не шестизначный: {code}");
            assert!(code.chars().all(|sign| sign.is_ascii_digit()));
        }
    }

    /// Код стоит весь свой период и меняется ровно на границе.
    #[test]
    fn the_window_boundary_moves_the_code() {
        let shape = Shape::default();

        let (start, _) = at(SEED_SHA1, &shape, 0);
        let (inside, _) = at(SEED_SHA1, &shape, 29);
        let (next, _) = at(SEED_SHA1, &shape, 30);

        assert_eq!(start, inside, "код сменился внутри своего окна");
        assert_ne!(start, next, "код не сменился на границе окна");
    }

    /// Будильник окна и смена кода обязаны указывать на одно мгновение.
    #[test]
    fn valid_until_is_the_end_of_the_window() {
        let shape = Shape::default();

        for moment in [0i64, 1, 29, 30, 31, 1234567890] {
            let (_, until) = at(SEED_SHA1, &shape, moment);
            assert_eq!(until, (moment / 30 + 1) * 30, "мгновение {moment}");
            assert!(until > moment, "будильник в прошлом на {moment}");
        }
    }

    /// Период 0 из правленного руками файла не должен делить на ноль.
    #[test]
    fn a_zero_period_never_divides() {
        let shape = Shape {
            digits: 6,
            period: 0,
            algorithm: Algorithm::Sha1,
        };

        let (code, until) = at(SEED_SHA1, &shape, 1234567890);
        assert_eq!(code.len(), 6);
        assert!(until > 1234567890);
    }

    /// Длина кода из чужой ссылки приводится в диапазон, а не отдаётся как есть.
    #[test]
    fn digits_are_brought_into_range() {
        assert_eq!(
            at(SEED_SHA1, &Shape { digits: 99, period: 30, algorithm: Algorithm::Sha1 }, 59).0.len(),
            usize::from(MAX_DIGITS)
        );
        assert_eq!(
            at(SEED_SHA1, &Shape { digits: 0, period: 30, algorithm: Algorithm::Sha1 }, 59).0.len(),
            usize::from(MIN_DIGITS)
        );
    }

    /// Часы раньше эпохи не роняют счёт.
    #[test]
    fn a_clock_before_the_epoch_does_not_panic() {
        let (code, until) = at(SEED_SHA1, &Shape::default(), -100);
        assert_eq!(code.len(), 6);
        assert_eq!(until, 30);
    }

    /// Имя алгоритма из ссылки читается в любом регистре, чужое — отвергается.
    #[test]
    fn algorithm_names_are_read_in_any_case() {
        assert_eq!(Algorithm::from_name("sha1"), Some(Algorithm::Sha1));
        assert_eq!(Algorithm::from_name("SHA256"), Some(Algorithm::Sha256));
        assert_eq!(Algorithm::from_name("Sha512"), Some(Algorithm::Sha512));
        assert_eq!(Algorithm::from_name("md5"), None);
    }
}
