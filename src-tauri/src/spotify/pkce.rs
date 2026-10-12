//! Proof Key for Code Exchange — RFC 7636.
//!
//! Публичному клиенту секрет доверить некуда: он лежал бы прямо в бинарнике.
//! Вместо секрета мы придумываем случайную строку `verifier`, отправляем на
//! авторизацию только её хеш, а сам `verifier` показываем лишь при обмене
//! кода. Перехваченный код без него бесполезен.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use sha2::{Digest, Sha256};

/// Длина случайной части в байтах. После base64url выходит 43 символа —
/// нижняя граница, разрешённая RFC (43..128).
const VERIFIER_BYTES: usize = 32;

/// Длина параметра `state`, которым сверяем, что вернулись из своей же сессии.
const STATE_BYTES: usize = 16;

pub struct Pkce {
    /// Секрет этой попытки входа. Наружу уходит только при обмене кода.
    pub verifier: String,
    /// Хеш секрета — его и видит Spotify на странице авторизации.
    pub challenge: String,
    /// Защита от подмены колбэка: возвращается нам нетронутым.
    pub state: String,
}

impl Pkce {
    pub fn generate() -> Result<Self, String> {
        let verifier = random_token(VERIFIER_BYTES)?;
        let challenge = challenge_for(&verifier);
        let state = random_token(STATE_BYTES)?;

        Ok(Self {
            verifier,
            challenge,
            state,
        })
    }
}

/// base64url без набивки от SHA-256 — метод `S256`, единственный, который
/// принимает Spotify.
fn challenge_for(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn random_token(bytes: usize) -> Result<String, String> {
    let mut buffer = vec![0u8; bytes];
    getrandom::fill(&mut buffer)
        .map_err(|error| tr!("spotify.error.noRandom", error = error))?;
    Ok(URL_SAFE_NO_PAD.encode(buffer))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Контрольный пример из RFC 7636, приложение B. Если наш способ считать
    /// challenge когда-нибудь разъедется со спецификацией, поймается здесь,
    /// а не на живой авторизации.
    #[test]
    fn matches_rfc_example() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            challenge_for(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn verifier_length_is_within_spec() {
        let pkce = Pkce::generate().expect("случайность доступна");
        assert!(
            (43..=128).contains(&pkce.verifier.len()),
            "verifier длиной {} вне разрешённого диапазона",
            pkce.verifier.len()
        );
    }

    /// Символы вне base64url сломали бы ссылку авторизации молча.
    #[test]
    fn tokens_are_url_safe() {
        let pkce = Pkce::generate().expect("случайность доступна");
        for token in [&pkce.verifier, &pkce.challenge, &pkce.state] {
            assert!(
                token
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'),
                "в токене «{token}» есть символы, требующие экранирования"
            );
        }
    }

    #[test]
    fn each_attempt_is_unique() {
        let first = Pkce::generate().expect("случайность доступна");
        let second = Pkce::generate().expect("случайность доступна");
        assert_ne!(first.verifier, second.verifier);
        assert_ne!(first.state, second.state);
    }
}
