//! Формирование адресов веб-ящика и конкретных писем для перехода в браузер.
//!
//! Не ходит в сеть, производит чистые расчёты по адресу IMAP-сервера и Message-ID.

/// Адрес веб-ящика по адресу IMAP-сервера.
///
/// У хоста отбрасывается ведущая метка `imap.`, остальное становится `https://<остаток>`.
/// Хост без `imap.` берётся как есть. Пустой или бессмысленный хост даёт `None`.
pub fn mailbox_url(host: &str) -> Option<String> {
    let host = host.trim();
    if host.is_empty() {
        return None;
    }

    // Бессмысленный хост (пробелы, косые черты, знак @)
    if host.chars().any(|c| c.is_whitespace() || c == '/' || c == '\\' || c == '@') {
        return None;
    }

    let remainder = if host.to_ascii_lowercase().starts_with("imap.") {
        &host[5..]
    } else {
        host
    };

    if remainder.is_empty() {
        return None;
    }

    Some(format!("https://{remainder}"))
}

/// Адрес конкретного письма, если провайдер такое умеет.
///
/// У Gmail при известном `Message-ID` открывает поиск письма по `rfc822msgid`.
/// Во всех остальных случаях ведёт туда же, куда кнопка перехода в ящик.
///
/// ВАЖНО: **своя ссылка старше выведенной и тут тоже.** Раз человек задал её
/// руками, значит выведенная вела не туда, — и письмо, открывающееся по
/// выведенной, вело бы не туда ровно так же.
pub fn letter_url(host: &str, message_id: Option<&str>, own_mailbox: Option<&str>) -> Option<String> {
    let base = own_mailbox
        .map(str::trim)
        .filter(|link| !link.is_empty())
        .map(str::to_owned)
        .or_else(|| mailbox_url(host))?;

    let host_clean = host.trim().to_ascii_lowercase();
    let is_gmail = host_clean == "gmail.com" || host_clean.ends_with(".gmail.com");

    if is_gmail {
        if let Some(raw_id) = message_id {
            let mut id = raw_id.trim();
            // Снимаем обрамляющие угловые скобки <...>, если они есть
            if id.starts_with('<') && id.ends_with('>') && id.len() >= 2 {
                id = &id[1..id.len() - 1];
            } else {
                id = id.strip_prefix('<').unwrap_or(id);
                id = id.strip_suffix('>').unwrap_or(id);
            }
            let id = id.trim();

            if !id.is_empty() {
                let encoded = percent_encode(id);
                let user = gmail_user(&base).unwrap_or("0");
                return Some(format!(
                    "https://mail.google.com/mail/u/{user}/#search/rfc822msgid:{encoded}"
                ));
            }
        }
    }

    Some(base)
}

/// Номер аккаунта берём только из пути своей ссылки Gmail, до запроса и фрагмента.
fn gmail_user(link: &str) -> Option<&str> {
    let (_, address) = link.split_once("://")?;
    let (host, path) = address.split_once('/')?;
    if !["mail.google.com", "gmail.com", "www.gmail.com"].iter().any(|gmail| host.eq_ignore_ascii_case(gmail)) {
        return None;
    }
    let path = path.split(['?', '#']).next()?;
    let user = path.strip_prefix("mail/u/")?.split('/').next()?;
    (!user.is_empty() && user.bytes().all(|ch| ch.is_ascii_digit())).then_some(user)
}

/// Ручное процентное кодирование (URL-encoding) по списку разрешённых знаков RFC 3986 (unreserved).
fn percent_encode(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len());
    for b in input.as_bytes() {
        // Разрешённые знаки по RFC 3986: A-Z, a-z, 0-9, '-', '_', '.', '~'
        if b.is_ascii_alphanumeric() || *b == b'-' || *b == b'_' || *b == b'.' || *b == b'~' {
            encoded.push(*b as char);
        } else {
            use std::fmt::Write;
            let _ = write!(encoded, "%{:02X}", b);
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_own_link_falls_back_to_host() {
        for own in ["", "   ", "\t\n"] {
            assert_eq!(letter_url("imap.example.org", None, Some(own)), mailbox_url("imap.example.org"));
            assert_eq!(letter_url("", None, Some(own)), None);
        }
        assert_eq!(letter_url("imap.example.org", None, Some("  https://mail.example.org  ")), Some("https://mail.example.org".into()));
    }

    #[test]
    fn own_link_wins_over_derived() {
        assert_eq!(
            letter_url("imap.example.com", Some("id@example.com"), Some("https://mail.example.com/login")),
            Some("https://mail.example.com/login".to_string()),
            "своя ссылка обязана перебивать выведенную"
        );
        assert_eq!(
            mailbox_url("imap.example.com"),
            Some("https://example.com".to_string()),
            "сама выводка при этом не меняется"
        );
    }

    /// У Gmail точная ссылка на письмо старше и своей: она открывает само
    /// письмо, а не ящик, и это ровно то, чего от кнопки ждут.
    #[test]
    fn gmail_message_link_survives_an_own_link() {
        let url = letter_url("imap.gmail.com", Some("<abc@mail>"), Some("https://mail.google.com/u/1"))
            .expect("ссылка собралась");
        assert!(url.contains("rfc822msgid:abc%40mail"), "{url}");
    }

    #[test]
    fn gmail_keeps_the_user_from_the_custom_path() {
        for (own, expected) in [
            ("https://mail.google.com/mail/u/1/#inbox", "1"),
            ("https://mail.google.com/mail/u/12/?authuser=0#inbox", "12"),
            ("https://gmail.com/mail/u/2", "2"),
            ("https://mail.google.com/mail/u/x/#inbox", "0"),
            ("https://mail.google.com/#/mail/u/3/", "0"),
            ("https://example.org/mail/u/4/", "0"),
        ] {
            let url = letter_url("imap.gmail.com", Some("<id@mail>"), Some(own)).unwrap();
            assert_eq!(url, format!("https://mail.google.com/mail/u/{expected}/#search/rfc822msgid:id%40mail"));
        }
    }

    #[test]
    fn regular_host_with_imap_prefix() {
        assert_eq!(
            mailbox_url("imap.example.com"),
            Some("https://example.com".to_string())
        );
        assert_eq!(
            mailbox_url("imap.gmail.com"),
            Some("https://gmail.com".to_string())
        );
    }

    #[test]
    fn host_without_imap_prefix() {
        assert_eq!(
            mailbox_url("mail.example.org"),
            Some("https://mail.example.org".to_string())
        );
        assert_eq!(
            mailbox_url("gmail.com"),
            Some("https://gmail.com".to_string())
        );
    }

    #[test]
    fn empty_host_returns_none() {
        assert_eq!(mailbox_url(""), None);
        assert_eq!(mailbox_url("   "), None);
        assert_eq!(mailbox_url("imap."), None);
        assert_eq!(mailbox_url("imap.   "), None);
    }

    #[test]
    fn host_with_garbage_returns_none() {
        assert_eq!(mailbox_url("imap.example.com/webmail"), None);
        assert_eq!(mailbox_url("example.com\\path"), None);
        assert_eq!(mailbox_url("user@example.com"), None);
        assert_eq!(mailbox_url("imap.host with spaces.com"), None);
    }

    #[test]
    fn gmail_with_and_without_message_id() {
        // Без Message-ID отдаёт адрес ящика
        assert_eq!(
            letter_url("imap.gmail.com", None, None),
            Some("https://gmail.com".to_string())
        );
        assert_eq!(
            letter_url("imap.gmail.com", Some(""), None),
            Some("https://gmail.com".to_string())
        );
        assert_eq!(
            letter_url("imap.gmail.com", Some("   "), None),
            Some("https://gmail.com".to_string())
        );

        // С Message-ID формирует поиск rfc822msgid
        assert_eq!(
            letter_url("imap.gmail.com", Some("plain-id-123"), None),
            Some("https://mail.google.com/mail/u/0/#search/rfc822msgid:plain-id-123".to_string())
        );
    }

    #[test]
    fn non_gmail_with_message_id_returns_mailbox_url() {
        assert_eq!(
            letter_url("imap.example.com", Some("id-123@example.com"), None),
            Some("https://example.com".to_string())
        );
    }

    #[test]
    fn message_id_with_angle_brackets_and_special_chars() {
        // Угловые скобки снимаются, спецсимволы кодируются процентами
        let raw_id = "<CAB+123=foo/bar@mail.gmail.com>";
        let url = letter_url("imap.gmail.com", Some(raw_id), None);
        assert_eq!(
            url,
            Some("https://mail.google.com/mail/u/0/#search/rfc822msgid:CAB%2B123%3Dfoo%2Fbar%40mail.gmail.com".to_string())
        );

        // Только угловые скобки без содержимого дают адрес ящика
        assert_eq!(
            letter_url("imap.gmail.com", Some("<>"), None),
            Some("https://gmail.com".to_string())
        );
        assert_eq!(
            letter_url("imap.gmail.com", Some("<   >"), None),
            Some("https://gmail.com".to_string())
        );
    }
}
