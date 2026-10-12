//! Приём редиректа со страницы авторизации.
//!
//! Spotify сверяет redirect URI строкой целиком, вместе с портом, поэтому
//! «занять любой свободный» не годится: адрес должен быть заранее записан
//! в дашборде. Держим короткий список и берём первый свободный из него.
//!
//! Сервер здесь свой на полсотни строк, а не библиотека: нам нужно принять
//! ровно один GET и ответить одной страницей.

use std::time::Duration;

use reqwest::Url;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Порты, зарегистрированные в дашборде Spotify. Порядок важен: первый
/// свободный и станет адресом редиректа.
pub const PORTS: [u16; 3] = [8888, 8889, 8890];

const PATH: &str = "/callback";

/// Больше заголовков нам не нужно, а бесконечный запрос читать незачем.
const MAX_REQUEST: usize = 8 * 1024;

/// Сколько ждём сам запрос от открытого соединения.
///
/// ВАЖНО: **браузеры открывают соединения впрок и молчат в них.** Приём тут
/// последовательный, и без срока такое соединение занимало бы его целиком:
/// настоящий редирект ждал бы своей очереди в `accept` до конца отведённого на
/// вход времени, а вход кончался бы отказом «ответа со страницы входа не
/// дождались». Причём с виду — по вине Spotify.
const REQUEST_WAIT: Duration = Duration::from_secs(5);

pub struct Callback {
    listener: TcpListener,
    pub redirect_uri: String,
}

pub fn redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}{PATH}")
}

/// Занимает первый свободный порт из списка.
pub async fn bind() -> Result<Callback, String> {
    for port in PORTS {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)).await {
            return Ok(Callback {
                listener,
                redirect_uri: redirect_uri(port),
            });
        }
    }

    Err(tr!(
        "spotify.error.portsBusy",
        ports = PORTS
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

impl Callback {
    /// Ждёт редиректа и возвращает код авторизации.
    ///
    /// Посторонние запросы (браузер любит спросить `/favicon.ico`) отвечаются
    /// и не считаются за ответ Spotify.
    pub async fn wait_for_code(&self, expected_state: &str) -> Result<String, String> {
        loop {
            let (mut stream, _) = self
                .listener
                .accept()
                .await
                .map_err(|error| tr!("spotify.error.notAccepted", error = error))?;

            let waited = tokio::time::timeout(REQUEST_WAIT, read_request_target(&mut stream)).await;

            let target = match waited {
                Ok(Some(target)) => target,
                Ok(None) => {
                    let _ = reply(&mut stream, 400, &page(&tr!("spotify.page.badRequest"), None)).await;
                    continue;
                }
                // Соединение молчит — почти наверняка открыто браузером впрок.
                // Отвечать нечего, ждём следующее
                Err(_) => continue,
            };

            match read_outcome(&target, expected_state) {
                Outcome::Elsewhere => {
                    let _ = reply(&mut stream, 404, "not found").await;
                }
                Outcome::Code(code) => {
                    let _ = reply(
                        &mut stream,
                        200,
                        &page(&tr!("spotify.page.done"), Some(&tr!("spotify.page.doneNote"))),
                    )
                    .await;
                    return Ok(code);
                }
                Outcome::Denied(reason) => {
                    let _ = reply(&mut stream, 200, &page(&tr!("spotify.page.denied"), Some(&reason))).await;
                    return Err(tr!("spotify.error.denied", reason = reason));
                }
                Outcome::Mismatch => {
                    // Ответ пришёл не на нашу попытку входа — возможно, в
                    // соседней вкладке висела старая страница авторизации
                    let _ = reply(
                        &mut stream,
                        400,
                        &page(&tr!("spotify.page.mismatch"), Some(&tr!("spotify.page.mismatchNote"))),
                    )
                    .await;
                }
            }
        }
    }
}

#[derive(Debug, PartialEq)]
enum Outcome {
    /// Не наш путь — отвечаем и ждём дальше.
    Elsewhere,
    Code(String),
    Denied(String),
    /// `state` не совпал: чужой или устаревший редирект.
    Mismatch,
}

fn read_outcome(target: &str, expected_state: &str) -> Outcome {
    // Базу берём произвольную: в строке запроса приходит только путь,
    // а разбирать его вручную — тот же Url, только хуже
    let Ok(url) = Url::parse(&format!("http://127.0.0.1{target}")) else {
        return Outcome::Elsewhere;
    };

    if url.path() != PATH {
        return Outcome::Elsewhere;
    }

    let mut code = None;
    let mut state = None;
    let mut error = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            _ => {}
        }
    }

    // Сверяем состояние раньше всего: без него неважно, что ещё пришло
    if state.as_deref() != Some(expected_state) {
        return Outcome::Mismatch;
    }

    match (code, error) {
        (_, Some(reason)) => Outcome::Denied(reason),
        (Some(code), None) => Outcome::Code(code),
        (None, None) => Outcome::Denied(tr!("spotify.error.noCode")),
    }
}

/// Читает первую строку запроса и достаёт из неё путь с параметрами.
async fn read_request_target(stream: &mut TcpStream) -> Option<String> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];

    loop {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);

        if let Some(end) = find_line_end(&buffer) {
            let line = std::str::from_utf8(&buffer[..end]).ok()?;
            return request_target(line).map(str::to_owned);
        }
        if buffer.len() > MAX_REQUEST {
            return None;
        }
    }
}

fn find_line_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(2).position(|pair| pair == b"\r\n")
}

/// `GET /callback?code=... HTTP/1.1` — берём середину.
fn request_target(line: &str) -> Option<&str> {
    let mut parts = line.split(' ');
    let method = parts.next()?;
    let target = parts.next()?;

    // Отвечаем только на GET: редирект приходит именно им
    (method == "GET" && target.starts_with('/')).then_some(target)
}

async fn reply(stream: &mut TcpStream, status: u16, body: &str) -> std::io::Result<()> {
    let reason = if status == 200 { "OK" } else { "Error" };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n",
        body.len()
    );

    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    stream.flush().await
}

/// Страница в браузере после возврата. Стиль тот же тёмный, что у виджетов,
/// чтобы переход не выглядел выпадением из приложения.
fn page(title: &str, detail: Option<&str>) -> String {
    let detail = detail.map(escape).unwrap_or_default();
    let title = escape(title);
    let lang = crate::i18n::current();

    format!(
        "<!doctype html><html lang=\"{lang}\"><head><meta charset=\"utf-8\">\
         <title>Windgets</title></head>\
         <body style=\"margin:0;min-height:100vh;display:flex;align-items:center;\
         justify-content:center;background:#0f1117;color:#e8ebf0;\
         font-family:'Segoe UI',system-ui,sans-serif\">\
         <div style=\"text-align:center;padding:32px\">\
         <div style=\"font-size:22px;font-weight:600\">{title}</div>\
         <div style=\"margin-top:8px;color:#98a1b0;font-size:14px\">{detail}</div>\
         </div></body></html>"
    )
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_code_when_state_matches() {
        let outcome = read_outcome("/callback?code=abc123&state=xyz", "xyz");
        assert_eq!(outcome, Outcome::Code("abc123".to_owned()));
    }

    /// Подменённый или устаревший редирект не должен доходить до обмена кода.
    #[test]
    fn rejects_foreign_state() {
        let outcome = read_outcome("/callback?code=abc123&state=other", "xyz");
        assert_eq!(outcome, Outcome::Mismatch);
        let outcome = read_outcome("/callback?code=abc123", "xyz");
        assert_eq!(outcome, Outcome::Mismatch);
    }

    #[test]
    fn reports_user_refusal() {
        let outcome = read_outcome("/callback?error=access_denied&state=xyz", "xyz");
        assert_eq!(outcome, Outcome::Denied("access_denied".to_owned()));
    }

    /// Запрос за иконкой приходит почти всегда и ответом Spotify не является.
    #[test]
    fn ignores_other_paths() {
        assert_eq!(read_outcome("/favicon.ico", "xyz"), Outcome::Elsewhere);
        assert_eq!(read_outcome("/", "xyz"), Outcome::Elsewhere);
    }

    #[test]
    fn parses_request_line() {
        assert_eq!(
            request_target("GET /callback?code=1 HTTP/1.1"),
            Some("/callback?code=1")
        );
        assert_eq!(request_target("POST /callback HTTP/1.1"), None);
        assert_eq!(request_target("мусор"), None);
    }

    #[test]
    fn redirect_uri_matches_what_is_registered_in_dashboard() {
        assert_eq!(redirect_uri(8888), "http://127.0.0.1:8888/callback");
    }

    #[test]
    fn page_escapes_what_spotify_sent_back() {
        let html = page("Отказ", Some("<script>alert(1)</script>"));
        assert!(!html.contains("<script>"), "разметка утекла в страницу");
        assert!(html.contains("&lt;script&gt;"));
    }
}
