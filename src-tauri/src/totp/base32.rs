//! Разбор секрета в base32 (RFC 4648) — том виде, в котором сервисы печатают
//! ключ для ручного ввода.
//!
//! ВАЖНО: **крейта тут нет намеренно, и это решение, а не экономия.** Base32 —
//! кодирование, а не криптография: ошибка в нём громкая и мгновенная, первый же
//! вектор RFC её ловит. А вот разбор нам нужен свой, прощающий: сервисы печатают
//! ключ группами по четыре, через пробел или дефис, строчными буквами и без
//! набивки. Обёртка вокруг готового крейта, приводящая строку к тому, что он
//! примет, вышла бы длиннее самого разбора.
//!
//! ВАЖНО: **знак вне алфавита — отказ, а не пропуск.** Прощающий разбор,
//! молча выбрасывающий чужое, превратил бы `1`, набранный вместо `I`, в
//! «работающий» секрет с всегда неверным кодом. Найти такое потом невозможно:
//! виджет показывает код, сервис его не принимает, и человек идёт чинить
//! сервис. Ровно поэтому `0`, `1`, `8` и `9` в алфавите и отсутствуют.

/// Алфавит RFC 4648: двадцать шесть букв и цифры со второй по седьмую.
/// Единицы и нуля тут нет — они слишком похожи на `I` и `O`.
const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// Знаки, которые разделяют группы и значения не несут.
fn is_spacer(symbol: char) -> bool {
    symbol == ' ' || symbol == '-' || symbol == '\t' || symbol == '\n' || symbol == '\r'
}

/// Место знака в алфавите, без оглядки на регистр.
///
/// ВАЖНО: **сперва проверка, что знак вообще ASCII, и только потом байт.**
/// Приведение `as u8` отрезает старшие разряды: кириллическая «с» (U+0441)
/// становится латинской `A`, «в» — цифрой `2`. Секрет, набранный в русской
/// раскладке, проходил бы проверку и давал вечно неверный код — ровно то, от
/// чего разбор и строгий. Нашло это ревью второй модели.
fn value_of(symbol: char) -> Option<u32> {
    let upper = u8::try_from(symbol).ok()?.to_ascii_uppercase();
    ALPHABET
        .iter()
        .position(|letter| *letter == upper)
        .map(|at| at as u32)
}

/// Секрет из строки в байты.
///
/// Набивка `=` необязательна, регистр не важен, пробелы и дефисы пропускаются.
/// Всё остальное — отказ с объяснением: строку набирает человек, и сказать ему,
/// что именно не так, дешевле, чем отдать молча испорченный ключ.
pub fn decode(text: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::with_capacity(text.len() * 5 / 8);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    let mut padded = false;
    let mut symbols = 0usize;

    for symbol in text.chars() {
        if is_spacer(symbol) {
            continue;
        }

        if symbol == '=' {
            // Набивка только в хвосте: она говорит «данные кончились»
            padded = true;
            continue;
        }

        if padded {
            return Err(tr!("totp.error.afterPadding"));
        }

        let Some(value) = value_of(symbol) else {
            return Err(tr!("totp.error.badSymbol", symbol = symbol));
        };

        buffer = (buffer << 5) | value;
        bits += 5;
        symbols += 1;

        if bits >= 8 {
            bits -= 8;
            bytes.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }

    if symbols == 0 {
        return Err(tr!("totp.error.emptySecret"));
    }

    // Остаток в пять бит и больше — это лишний знак, который не дал ни байта:
    // такой длины у правильной записи не бывает вовсе
    if bits >= 5 {
        return Err(tr!("totp.error.halfSymbol"));
    }

    // Хвостовые биты обязаны быть нулями: иначе запись не каноническая, и два
    // разных секрета давали бы одни и те же байты
    if buffer != 0 {
        return Err(tr!("totp.error.dirtyTail"));
    }

    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Векторы RFC 4648. Держат алфавит, порядок бит и набивку разом.
    #[test]
    fn rfc4648_vectors_decode() {
        let pairs = [
            ("MY======", "f"),
            ("MZXQ====", "fo"),
            ("MZXW6===", "foo"),
            ("MZXW6YQ=", "foob"),
            ("MZXW6YTB", "fooba"),
            ("MZXW6YTBOI======", "foobar"),
        ];

        for (encoded, expected) in pairs {
            let got = decode(encoded).expect("вектор RFC 4648 обязан разбираться");
            assert_eq!(
                got,
                expected.as_bytes(),
                "вектор {encoded} разобран не в {expected}"
            );
        }
    }

    /// Секреты копируют с сайтов строчными и без набивки — и так тоже надо.
    #[test]
    fn padding_is_optional_and_case_is_ignored() {
        let full = decode("MZXW6YTBOI======").expect("с набивкой");
        assert_eq!(decode("MZXW6YTBOI").expect("без набивки"), full);
        assert_eq!(decode("mzxw6ytboi").expect("строчными"), full);
        assert_eq!(decode("MzXw6YtBoI").expect("вперемешку"), full);
    }

    /// Сервисы печатают ключ группами по четыре, через пробел или дефис.
    #[test]
    fn spaces_and_dashes_are_ignored() {
        let plain = decode("JBSWY3DP").expect("подряд");
        assert_eq!(decode("JBSW Y3DP").expect("через пробел"), plain);
        assert_eq!(decode("JBSW-Y3DP").expect("через дефис"), plain);
        assert_eq!(decode("  JBSW Y3DP  ").expect("с полями"), plain);
    }

    /// Самый важный тест файла: единица вместо `I` не должна становиться
    /// «работающим» секретом с всегда неверным кодом.
    #[test]
    fn a_letter_outside_the_alphabet_is_refused() {
        for bad in ["JBSW03DP", "JBSW13DP", "JBSW83DP", "JBSW93DP"] {
            assert!(
                decode(bad).is_err(),
                "знак вне алфавита проглочен в {bad}"
            );
        }
    }

    /// Длина, оставляющая лишние биты, — испорченная запись, а не секрет.
    /// Кириллица из русской раскладки. У «с», «в» и «е» младший байт кода
    /// совпадает с `A`, `2` и `5` из алфавита, и приведение `as u8` пускало
    /// их внутрь молча.
    #[test]
    fn cyrillic_lookalikes_are_refused() {
        for bad in ["JBSWY3Dс", "JBSWY3Dв", "JBSWY3Dе", "JBSWY3Dт"] {
            assert!(decode(bad).is_err(), "{bad} прошёл бы с неверным кодом");
        }
    }

    #[test]
    fn a_broken_tail_is_refused() {
        // Один, три и шесть знаков — длины, которых у правильной записи нет
        assert!(decode("M").is_err(), "один знак не даёт ни байта");
        assert!(decode("MZX").is_err(), "три знака обрываются на половине");
        assert!(decode("MZXW6Y").is_err(), "шесть знаков обрываются тоже");
    }

    /// Ненулевой хвост — две разные записи одного и того же, а так нельзя.
    #[test]
    fn a_dirty_tail_is_refused() {
        // «MZ» даёт байт и два бита хвоста; «MZ» с другим вторым знаком, у
        // которого младшие биты не нули, обязан быть отвергнут
        assert!(decode("MY").is_ok(), "чистый хвост разбирается");
        assert!(decode("MZ").is_err(), "грязный хвост обязан быть отвергнут");
    }

    /// Пустая строка — не секрет, и молчать об этом нельзя.
    #[test]
    fn an_empty_secret_is_refused() {
        assert!(decode("").is_err());
        assert!(decode("   ").is_err(), "одни пробелы — тоже пусто");
        assert!(decode("====").is_err(), "одна набивка — тоже пусто");
    }

    /// Набивка стоит в хвосте и ничего за собой не пускает.
    #[test]
    fn symbols_after_the_padding_are_refused() {
        assert!(decode("MY======MY").is_err());
    }
}
