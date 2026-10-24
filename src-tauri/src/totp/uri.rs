//! Разбор ссылки `otpauth://totp/…` — того, что отдаёт сервис под QR-кодом.
//!
//! Крейта `url` тут нет намеренно: нам нужен ровно один вид ссылки с известным
//! набором полей, а не разбор произвольного URL с его портами, логинами и
//! нормализацией путей. Зато нужен свой отказ с объяснением — строку вставляет
//! человек, и сказать ему, что именно не так, дешевле, чем принять испорченное.
//!
//! ВАЖНО: **чужой тип — отказ, а не «сойдёт».** `otpauth://hotp/…` считается по
//! счётчику, а не по времени: принятый как TOTP, он давал бы вечно неверный код.

use super::code::{Algorithm, Shape, DEFAULT_DIGITS, DEFAULT_PERIOD, MAX_DIGITS, MIN_DIGITS};

/// Что удалось вынуть из ссылки.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub issuer: String,
    pub label: String,
    /// Секрет строкой base32, как он записан в ссылке. Байты из него делает
    /// `base32::decode`, и делает уже при счёте кода.
    pub secret: String,
    pub shape: Shape,
}

/// Раскодирование `%XX`.
///
/// Непонятная тройка остаётся как есть: ярлык с процентом в имени — это
/// испорченная подпись, а не повод отказать в заведении кода.
fn percent(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut at = 0;

    while at < bytes.len() {
        if bytes[at] == b'%' && at + 2 < bytes.len() {
            let pair = std::str::from_utf8(&bytes[at + 1..at + 3]).ok();
            if let Some(value) = pair.and_then(|pair| u8::from_str_radix(pair, 16).ok()) {
                out.push(value);
                at += 3;
                continue;
            }
        }

        out.push(bytes[at]);
        at += 1;
    }

    // Ярлыки бывают на любом языке, и проценты скрывают UTF-8 по байтам
    String::from_utf8_lossy(&out).into_owned()
}

/// Ссылка целиком: схема, тип, ярлык и поля запроса.
pub fn parse(link: &str) -> Result<Parsed, String> {
    let trimmed = link.trim();
    let lower = trimmed.to_ascii_lowercase();

    let Some(rest) = lower
        .starts_with("otpauth://")
        .then(|| &trimmed["otpauth://".len()..])
    else {
        return Err(tr!("totp.error.notOtpauth"));
    };

    let (kind, tail) = rest.split_once('/').unwrap_or((rest, ""));
    match kind.to_ascii_lowercase().as_str() {
        "totp" => {}
        "hotp" => {
            return Err(tr!("totp.error.hotp"))
        }
        other => return Err(tr!("totp.error.unknownType", kind = other)),
    }

    let (path, query) = tail.split_once('?').unwrap_or((tail, ""));

    // Ярлык бывает «Издатель:учётная запись», и двоеточие в нём часто закодировано
    let whole = percent(path);
    let (from_path, label) = match whole.split_once(':') {
        Some((issuer, account)) => (issuer.trim().to_owned(), account.trim().to_owned()),
        None => (String::new(), whole.trim().to_owned()),
    };

    let mut secret = String::new();
    let mut from_query = String::new();
    let mut digits = DEFAULT_DIGITS;
    let mut period = DEFAULT_PERIOD;
    let mut algorithm = Algorithm::Sha1;

    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = percent(value);

        match name.to_ascii_lowercase().as_str() {
            "secret" => secret = value,
            "issuer" => from_query = value.trim().to_owned(),
            // Явное число цифр нельзя подменять умолчанием: минус, мусор и
            // переполнение тоже дали бы код, которого сервис не ждёт.
            "digits" => {
                let asked = value.trim();
                digits = asked.parse::<u64>()
                    .ok()
                    .filter(|digits| (u64::from(MIN_DIGITS)..=u64::from(MAX_DIGITS)).contains(digits))
                    .ok_or_else(|| tr!("totp.error.digits", digits = asked))? as u8;
            }
            "period" => {
                let asked = value.trim();
                period = asked.parse::<u32>().ok().filter(|period| *period > 0)
                    .ok_or_else(|| tr!("totp.error.period", period = asked))?;
            }
            // Чужой алгоритм не подменяется SHA-1: то же «нули опаснее
            // пропусков», что у датчиков, — код был бы неверным всегда
            "algorithm" => match Algorithm::from_name(&value) {
                Some(asked) => algorithm = asked,
                None => return Err(tr!("totp.error.algorithm", name = value.trim())),
            },
            _ => {}
        }
    }

    if secret.trim().is_empty() {
        return Err(tr!("totp.error.noSecret"));
    }

    // Издатель из запроса главнее: спецификация Key URI называет ярлык
    // совместимостью со старыми приложениями, а поле — источником правды
    let issuer = if from_query.is_empty() {
        from_path
    } else {
        from_query
    };

    Ok(Parsed {
        issuer,
        label,
        secret: secret.trim().to_owned(),
        shape: Shape {
            digits,
            period,
            algorithm,
        }
        .sane(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_google_authenticator_link_is_parsed() {
        let got = parse("otpauth://totp/Example:alice@google.com?secret=JBSWY3DPEHPK3PXP&issuer=Example")
            .expect("обычная ссылка обязана разбираться");

        assert_eq!(got.issuer, "Example");
        assert_eq!(got.label, "alice@google.com");
        assert_eq!(got.secret, "JBSWY3DPEHPK3PXP");
        assert_eq!(got.shape, Shape::default());
    }

    /// Издатель бывает записан дважды, и у поля запроса приоритет.
    #[test]
    fn the_issuer_comes_from_the_query_or_from_the_label() {
        let from_path = parse("otpauth://totp/GitHub:demo-user?secret=JBSWY3DP").expect("из пути");
        assert_eq!(from_path.issuer, "GitHub");
        assert_eq!(from_path.label, "demo-user");

        let both = parse("otpauth://totp/Старое:demo-user?secret=JBSWY3DP&issuer=GitHub")
            .expect("оба разом");
        assert_eq!(both.issuer, "GitHub", "поле запроса обязано быть главнее");

        let none = parse("otpauth://totp/demo-user?secret=JBSWY3DP").expect("без издателя");
        assert_eq!(none.issuer, "");
        assert_eq!(none.label, "demo-user");
    }

    /// Без раскодирования на карточке висело бы `alice%40google.com`.
    #[test]
    fn percent_escapes_are_decoded() {
        let got = parse("otpauth://totp/My%20Site%3Aalice%40mail.com?secret=JBSWY3DP")
            .expect("со спецзнаками");

        assert_eq!(got.issuer, "My Site");
        assert_eq!(got.label, "alice@mail.com");
    }

    /// Кириллица в ярлыке приезжает байтами UTF-8 под процентами.
    #[test]
    fn a_utf8_label_survives_the_escapes() {
        let got = parse("otpauth://totp/%D0%9F%D0%BE%D1%87%D1%82%D0%B0:me?secret=JBSWY3DP")
            .expect("кириллица в издателе");
        assert_eq!(got.issuer, "Почта");
    }

    /// HOTP считается по счётчику: принятый как TOTP, он давал бы вечно
    /// неверный код, и человек чинил бы сервис.
    #[test]
    fn a_foreign_type_is_refused() {
        assert!(parse("otpauth://hotp/A:b?secret=JBSWY3DP&counter=1").is_err());
        assert!(parse("otpauth://steam/A:b?secret=JBSWY3DP").is_err());
        assert!(parse("https://example.com/?secret=JBSWY3DP").is_err());
    }

    #[test]
    fn defaults_fill_what_the_link_omits() {
        let got = parse("otpauth://totp/A:b?secret=JBSWY3DP").expect("голая ссылка");
        assert_eq!(got.shape.digits, DEFAULT_DIGITS);
        assert_eq!(got.shape.period, DEFAULT_PERIOD);
        assert_eq!(got.shape.algorithm, Algorithm::Sha1);
    }

    #[test]
    fn the_shape_comes_from_the_query() {
        let got = parse("otpauth://totp/A:b?secret=JBSWY3DP&digits=8&period=60&algorithm=SHA256")
            .expect("со своей формой");
        assert_eq!(got.shape.digits, 8);
        assert_eq!(got.shape.period, 60);
        assert_eq!(got.shape.algorithm, Algorithm::Sha256);
    }

    #[test]
    fn invalid_explicit_periods_are_refused_and_spaces_are_trimmed() {
        for period in ["0", "-1", "4294967296", "", "нет", "30.0", "%200%20"] {
            let error = parse(&format!("otpauth://totp/A:b?secret=JBSWY3DP&period={period}"))
                .expect_err("неподдерживаемый период отвергается");
            assert_eq!(error, tr!("totp.error.period", period = percent(period).trim()));
        }
        for (period, expected) in [("%2060%20", 60), ("1", 1), ("4294967295", u32::MAX)] {
            assert_eq!(parse(&format!("otpauth://totp/A:b?secret=JBSWY3DP&period={period}"))
                .unwrap().shape.period, expected);
        }
    }

    /// Чужой алгоритм и число цифр вне шести—восьми — отказ, а не тихая
    /// подмена: с ней код был бы неверным всегда.
    #[test]
    fn an_unknown_algorithm_or_digit_count_is_refused() {
        assert!(parse("otpauth://totp/A:b?secret=JBSWY3DP&algorithm=md5").is_err());
        assert!(parse("otpauth://totp/A:b?secret=JBSWY3DP&digits=10").is_err());
        assert!(parse("otpauth://totp/A:b?secret=JBSWY3DP&digits=5").is_err());
        assert!(parse("otpauth://totp/A:b?secret=JBSWY3DP&digits=8&algorithm=SHA256").is_ok());
    }

    /// Явное неподдерживаемое значение не должно превращаться в шесть цифр.
    #[test]
    fn invalid_explicit_digits_use_the_digits_error() {
        for digits in ["256", "-1", "18446744073709551616", "", "нет", "6.0", "0", "9"] {
            let error = parse(&format!("otpauth://totp/A:b?secret=JBSWY3DP&digits={digits}"))
                .expect_err("неподдерживаемая форма отвергается");
            assert_eq!(error, tr!("totp.error.digits", digits = digits));
        }
        for digits in [6, 7, 8] {
            assert_eq!(parse(&format!("otpauth://totp/A:b?secret=JBSWY3DP&digits={digits}"))
                .expect("поддерживаемая форма").shape.digits, digits);
        }
    }

    /// Ссылка без секрета — не запись, а опечатка.
    #[test]
    fn a_link_without_a_secret_is_refused() {
        assert!(parse("otpauth://totp/A:b?issuer=A").is_err());
        assert!(parse("otpauth://totp/A:b?secret=").is_err());
        assert!(parse("otpauth://totp/A:b").is_err());
    }

    /// Схему пишут и заглавными.
    #[test]
    fn the_scheme_is_read_in_any_case() {
        assert!(parse("OTPAUTH://TOTP/A:b?secret=JBSWY3DP").is_ok());
        assert!(parse("  otpauth://totp/A:b?secret=JBSWY3DP  ").is_ok(), "с полями по краям");
    }
}
