//! Authorization Code с PKCE: ссылка на страницу входа, обмен кода на токены
//! и их обновление.
//!
//! Implicit grant и алиасы `localhost` Spotify выключил — работает только
//! этот поток и только с петлевым адресом по IP.

use std::time::{Duration, Instant};

use reqwest::Url;
use serde::Deserialize;

const AUTHORIZE_URL: &str = "https://accounts.spotify.com/authorize";
const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";

/// Права, которые просим. Лишнего не берём: чтение состояния плеера,
/// управление им и список плейлистов.
const SCOPES: &str = concat!(
    "user-read-playback-state ",
    "user-modify-playback-state ",
    "user-read-currently-playing ",
    "playlist-read-private ",
    "playlist-read-collaborative"
);

/// Насколько раньше срока считаем токен просроченным. Секунда опроса плюс
/// дорога до Spotify — запас нужен, иначе поймаем 401 на ровном месте.
const EXPIRY_MARGIN: Duration = Duration::from_secs(60);

/// Живой токен доступа. Наружу не отдаётся и на диск не пишется: срок жизни
/// у него час, восстановить его всегда можно из refresh token.
#[derive(Debug, Clone)]
pub struct AccessToken {
    pub value: String,
    expires_at: Instant,
}

impl AccessToken {
    pub fn is_fresh(&self) -> bool {
        Instant::now() + EXPIRY_MARGIN < self.expires_at
    }
}

/// Ответ Spotify на обмен или обновление.
pub struct Tokens {
    pub access: AccessToken,
    /// При обновлении Spotify может выдать новый refresh token, а может
    /// промолчать — тогда остаётся действовать прежним.
    pub refresh: Option<String>,
}

/// Различать эти два случая обязательно: при отказе сохранённый refresh
/// token уже ничего не стоит и его надо стереть, а при обрыве связи —
/// наоборот, беречь и пробовать снова.
#[derive(Debug)]
pub enum AuthError {
    /// Spotify отверг сам грант: токен отозван, Client ID не тот, redirect
    /// URI не зарегистрирован.
    Rejected(String),
    /// Сеть, таймаут, нечитаемый ответ.
    Transport(String),
}

impl AuthError {
    pub fn message(&self) -> String {
        match self {
            AuthError::Rejected(detail) | AuthError::Transport(detail) => detail.clone(),
        }
    }
}

pub fn authorize_url(client_id: &str, redirect_uri: &str, challenge: &str, state: &str) -> String {
    let mut url = Url::parse(AUTHORIZE_URL).expect("адрес авторизации разбирается");
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("state", state)
        .append_pair("scope", SCOPES)
        .append_pair("code_challenge_method", "S256")
        .append_pair("code_challenge", challenge);
    url.into()
}

pub async fn exchange_code(
    client: &reqwest::Client,
    client_id: &str,
    redirect_uri: &str,
    code: &str,
    verifier: &str,
) -> Result<Tokens, AuthError> {
    post_token(
        client,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", client_id),
            ("code_verifier", verifier),
        ],
    )
    .await
}

pub async fn refresh(
    client: &reqwest::Client,
    client_id: &str,
    refresh_token: &str,
) -> Result<Tokens, AuthError> {
    post_token(
        client,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client_id),
        ],
    )
    .await
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
}

#[derive(Deserialize)]
struct TokenError {
    error: Option<String>,
    error_description: Option<String>,
}

async fn post_token(client: &reqwest::Client, form: &[(&str, &str)]) -> Result<Tokens, AuthError> {
    let response = client
        .post(TOKEN_URL)
        .form(form)
        .send()
        .await
        .map_err(|error| {
            AuthError::Transport(tr!("spotify.error.accountsUnreachable", error = error))
        })?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| AuthError::Transport(tr!("spotify.error.unreadable", error = error)))?;

    if !status.is_success() {
        return Err(AuthError::Rejected(describe_failure(status, &body)));
    }

    let parsed: TokenResponse = serde_json::from_str(&body)
        .map_err(|error| AuthError::Transport(tr!("spotify.error.garbled", error = error)))?;

    Ok(Tokens {
        access: AccessToken {
            value: parsed.access_token,
            expires_at: Instant::now() + Duration::from_secs(parsed.expires_in),
        },
        refresh: parsed.refresh_token,
    })
}

/// Ошибки этого эндпоинта осмысленные, и их стоит показать целиком: чаще
/// всего это неверный Client ID или незарегистрированный redirect URI, и
/// угадывать причину по коду ответа — только тратить время.
fn describe_failure(status: reqwest::StatusCode, body: &str) -> String {
    let detail = serde_json::from_str::<TokenError>(body)
        .ok()
        .and_then(|parsed| parsed.error_description.or(parsed.error));

    match detail {
        Some(text) => tr!("spotify.error.refusedWith", status = status, text = text),
        None => tr!("spotify.error.refused", status = status),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(url: &str) -> Vec<(String, String)> {
        Url::parse(url)
            .expect("ссылка разбирается")
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect()
    }

    #[test]
    fn authorize_url_carries_everything_spotify_asks_for() {
        let url = authorize_url(
            "client-123",
            "http://127.0.0.1:8888/callback",
            "challenge-abc",
            "state-xyz",
        );
        let pairs = params(&url);

        let get = |name: &str| {
            pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        };

        assert!(url.starts_with(AUTHORIZE_URL));
        assert_eq!(get("client_id"), Some("client-123"));
        assert_eq!(get("response_type"), Some("code"));
        assert_eq!(get("redirect_uri"), Some("http://127.0.0.1:8888/callback"));
        assert_eq!(get("code_challenge_method"), Some("S256"));
        assert_eq!(get("code_challenge"), Some("challenge-abc"));
        assert_eq!(get("state"), Some("state-xyz"));
    }

    /// Разделитель областей — пробел, и он обязан уехать закодированным.
    /// Ручная склейка строки запроса здесь однажды и ломается.
    #[test]
    fn scopes_are_encoded_not_glued() {
        let url = authorize_url("id", "http://127.0.0.1:8888/callback", "c", "s");
        assert!(!url.contains(' '), "в ссылке остался голый пробел: {url}");

        let pairs = params(&url);
        let scope = pairs
            .iter()
            .find(|(key, _)| key == "scope")
            .map(|(_, value)| value.as_str())
            .expect("области запрошены");
        assert!(scope.contains("user-modify-playback-state"));
        assert!(scope.contains("playlist-read-private"));
    }

    #[test]
    fn failure_shows_spotify_own_words() {
        let body = r#"{"error":"invalid_client","error_description":"Invalid client"}"#;
        let message = describe_failure(reqwest::StatusCode::BAD_REQUEST, body);
        assert!(message.contains("Invalid client"), "получилось: {message}");
    }

    #[test]
    fn failure_survives_body_that_is_not_json() {
        let message = describe_failure(reqwest::StatusCode::BAD_GATEWAY, "<html>502</html>");
        assert!(message.contains("502"), "получилось: {message}");
    }

    #[test]
    fn token_counts_as_stale_before_it_actually_expires() {
        let almost_gone = AccessToken {
            value: "x".into(),
            expires_at: Instant::now() + Duration::from_secs(30),
        };
        assert!(!almost_gone.is_fresh(), "запас перед истечением не сработал");

        let plenty = AccessToken {
            value: "x".into(),
            expires_at: Instant::now() + Duration::from_secs(3600),
        };
        assert!(plenty.is_fresh());
    }
}
