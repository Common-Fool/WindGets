//! Обёртка над Spotify Web API — ровно те вызовы, которые нужны пульту.
//!
//! Звук идёт из десктопного Спотика или веб-плеера, виджет ими только
//! управляет: все команды здесь адресованы «активному устройству».

use std::time::Duration;

use reqwest::{Response, StatusCode};
use serde::{Deserialize, Serialize};

const API: &str = "https://api.spotify.com/v1";

/// Сколько молчать после 429, если Spotify не сказал сам.
///
/// Заголовок `Retry-After` он присылает почти всегда, но полагаться на это
/// нельзя: без паузы опрос продолжил бы долбить в ту же стену.
const DEFAULT_COOLDOWN: Duration = Duration::from_secs(30);

/// Сколько плейлистов забираем. Больше в список виджета всё равно не влезет,
/// а листать страницы ради полноты незачем.
const PLAYLIST_LIMIT: u32 = 50;

/// Ниже этой ширины обложка заметно мылится на любой разумной вёрстке.
const COVER_MIN_WIDTH: u32 = 300;

/// Deserialize нужен, чтобы последний трек пережил перезапуск: виджет
/// показывает его, пока Спотифай молчит.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Player {
    pub track: String,
    pub artists: String,
    pub album: String,
    pub cover_url: Option<String>,
    /// Трек или эпизод в Spotify. Политика разработчика требует ссылки
    /// обратно у показанных обложки и названия — нажатие по обложке ведёт сюда
    pub link: Option<String>,
    pub is_playing: bool,
    pub progress_ms: u64,
    pub duration_ms: u64,
    pub shuffle: bool,
    /// `off`, `context` или `track` — как их называет сам Spotify.
    pub repeat: String,
    pub volume: Option<u8>,
    pub device: Option<String>,
    /// У части устройств (например, у некоторых колонок) громкость не
    /// регулируется через API — ползунок тогда прячем.
    pub supports_volume: bool,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            track: String::new(),
            artists: String::new(),
            album: String::new(),
            cover_url: None,
            link: None,
            is_playing: false,
            progress_ms: 0,
            duration_ms: 0,
            shuffle: false,
            repeat: "off".to_owned(),
            volume: None,
            device: None,
            supports_volume: true,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub uri: String,
    pub cover_url: Option<String>,
    pub tracks: u32,
}

/// Команда пульта. Приходит с фронта как размеченный union.
///
/// `rename_all_fields` обязателен: без него `position_ms` так и уехал бы
/// в JSON змеиным регистром и разошёлся бы с типом на фронте.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Action {
    Play,
    Pause,
    Next,
    Previous,
    Shuffle { on: bool },
    Repeat { mode: String },
    Volume { percent: u8 },
    Seek { position_ms: u64 },
    PlayContext { uri: String },
}

#[derive(Debug)]
pub enum ApiError {
    /// Токен протух — вызывающий обновит его и повторит.
    Unauthorized,
    /// Играть некому: Спотифай нигде не открыт.
    NoActiveDevice,
    /// Чаще всего — управление недоступно без Premium.
    Forbidden(String),
    /// «Здесь так нельзя»: Spotify отвечает 403 «Restriction violated».
    ///
    /// Отдельно от [`ApiError::Forbidden`] намеренно: у прав всё однозначно, а
    /// это половину случаев означает не запрет, а опоздание. Тем же отказом
    /// встречают «паузу» на уже остановленном плеере и второе нажатие
    /// «дальше», пока первое ещё в пути, — то есть ровно то, что делает
    /// человек, глядя на картинку, снятую несколько секунд назад.
    Restricted,
    /// Запросов было слишком много. Пока не выйдет срок, ходить бесполезно:
    /// каждый новый запрос считается и продлевает наказание.
    RateLimited(Duration),
    /// До Spotify не достучались вовсе: сеть отвалилась или ответ не поспел.
    ///
    /// Отдельно от [`ApiError::Other`] ради опроса: сломался не пульт, и
    /// краснеть ему незачем. Ловится это на живом канале постоянно — хватает
    /// запустить замер скорости, и запрос к плееру не проходит.
    Unreachable(String),
    Other(String),
}

impl ApiError {
    pub fn message(&self) -> String {
        match self {
            ApiError::Unauthorized => tr!("spotify.error.expired"),
            ApiError::NoActiveDevice => tr!("spotify.error.noDevice"),
            ApiError::Forbidden(detail) => detail.clone(),
            ApiError::Restricted => tr!("spotify.error.restricted"),
            ApiError::RateLimited(wait) => tr!("spotify.error.wait", seconds = wait.as_secs()),
            ApiError::Unreachable(detail) => detail.clone(),
            ApiError::Other(detail) => detail.clone(),
        }
    }
}

/// Сколько Spotify просит молчать. Заголовок `Retry-After` у него в секундах.
fn cooldown(response: &Response) -> Duration {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_COOLDOWN)
}

/// Текущее состояние плеера. `None` — Spotify ответил «нечего показывать»
/// (пустой ответ 204), то есть ни одно устройство сейчас не играет.
pub async fn player(client: &reqwest::Client, token: &str) -> Result<Option<Player>, ApiError> {
    let response = client
        .get(format!("{API}/me/player"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(transport)?;

    // Именно так Spotify сообщает «плеер молчит»: пустое тело, а не JSON
    if response.status() == StatusCode::NO_CONTENT {
        return Ok(None);
    }

    let body = read_body(response).await?;
    let playback: PlaybackDto =
        serde_json::from_str(&body).map_err(|error| ApiError::Other(tr!("spotify.error.playerGarbled", error = error)))?;

    Ok(playback.into_player())
}

pub async fn playlists(client: &reqwest::Client, token: &str) -> Result<Vec<Playlist>, ApiError> {
    let response = client
        .get(format!("{API}/me/playlists?limit={PLAYLIST_LIMIT}"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(transport)?;

    let body = read_body(response).await?;
    let page: PlaylistPageDto = serde_json::from_str(&body)
        .map_err(|error| ApiError::Other(tr!("spotify.error.playlistsGarbled", error = error)))?;

    Ok(page
        .items
        .into_iter()
        .flatten()
        .filter_map(PlaylistDto::into_playlist)
        .collect())
}

/// Устройства, готовые играть.
///
/// Нужны, когда активного нет: тогда «играть» означает сперва выбрать, где.
pub async fn devices(client: &reqwest::Client, token: &str) -> Result<Vec<String>, ApiError> {
    let response = client
        .get(format!("{API}/me/player/devices"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(transport)?;

    let body = read_body(response).await?;
    let list: DeviceListDto = serde_json::from_str(&body)
        .map_err(|error| ApiError::Other(tr!("spotify.error.devicesGarbled", error = error)))?;

    // Устройство без идентификатора адресовать нечем — оно нам бесполезно
    Ok(list
        .devices
        .into_iter()
        .filter_map(|device| device.id)
        .collect())
}

/// Переносит воспроизведение на устройство и сразу запускает его.
pub async fn transfer(
    client: &reqwest::Client,
    token: &str,
    device_id: &str,
) -> Result<(), ApiError> {
    let response = client
        .put(format!("{API}/me/player"))
        .bearer_auth(token)
        .json(&serde_json::json!({ "device_ids": [device_id], "play": true }))
        .send()
        .await
        .map_err(transport)?;

    if response.status().is_success() {
        return Ok(());
    }

    Err(failure(response).await)
}

pub async fn control(
    client: &reqwest::Client,
    token: &str,
    action: &Action,
) -> Result<(), ApiError> {
    let response = build(client, action)
        .bearer_auth(token)
        .send()
        .await
        .map_err(transport)?;

    if response.status().is_success() {
        return Ok(());
    }

    Err(failure(response).await)
}

fn build(client: &reqwest::Client, action: &Action) -> reqwest::RequestBuilder {
    match action {
        Action::Play => empty(client.put(format!("{API}/me/player/play"))),
        Action::Pause => empty(client.put(format!("{API}/me/player/pause"))),
        Action::Next => empty(client.post(format!("{API}/me/player/next"))),
        Action::Previous => empty(client.post(format!("{API}/me/player/previous"))),
        Action::Shuffle { on } => empty(client.put(format!("{API}/me/player/shuffle?state={on}"))),
        Action::Repeat { mode } => empty(
            client.put(format!("{API}/me/player/repeat?state={}", repeat_mode(mode))),
        ),
        Action::Volume { percent } => empty(client.put(format!(
            "{API}/me/player/volume?volume_percent={}",
            percent.min(&100)
        ))),
        Action::Seek { position_ms } => {
            empty(client.put(format!("{API}/me/player/seek?position_ms={position_ms}")))
        }
        Action::PlayContext { uri } => client
            .put(format!("{API}/me/player/play"))
            .json(&serde_json::json!({ "context_uri": uri })),
    }
}

/// Команда без тела обязана объявить нулевую длину явно.
///
/// Пустое тело reqwest считает отсутствующим и `Content-Length` не шлёт
/// вовсе, а Spotify на такой PUT отвечает «411 Length Required» и ничего
/// не делает. Ловится только на живом сервисе.
fn empty(request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    request.header(reqwest::header::CONTENT_LENGTH, 0)
}

/// Значения повтора заданы протоколом; чужое молча превращаем в `off`,
/// чтобы опечатка на фронте не уходила в запрос.
pub(super) fn repeat_mode(mode: &str) -> &str {
    match mode {
        "track" | "context" => mode,
        _ => "off",
    }
}

async fn read_body(response: Response) -> Result<String, ApiError> {
    let status = response.status();
    let wait = cooldown(&response);
    let body = response
        .text()
        .await
        .map_err(|error| ApiError::Other(tr!("spotify.error.unreadable", error = error)))?;

    if status.is_success() {
        Ok(body)
    } else {
        Err(classify(status, wait, &body))
    }
}

/// Разбирает неудачный ответ целиком — вместе с заголовком паузы.
async fn failure(response: Response) -> ApiError {
    let status = response.status();
    let wait = cooldown(&response);
    let body = response.text().await.unwrap_or_default();

    classify(status, wait, &body)
}

/// Отказ, случившийся до ответа: сети нет, ответ не поспел, соединение сбросили.
///
/// Все они значат одно — до Spotify не достучались, — и различать их на
/// карточке нечем: место под текстом ровно одна строка.
fn transport(error: reqwest::Error) -> ApiError {
    if error.is_timeout() {
        ApiError::Unreachable(tr!("spotify.error.timeout"))
    } else {
        ApiError::Unreachable(tr!("spotify.error.offline"))
    }
}

/// Отказ Spotify человеческими словами: английская строка под пультом не
/// годится ни по длине, ни по смыслу.
fn refused(message: &str) -> String {
    message
        .trim_start_matches("Player command failed: ")
        .to_owned()
}

/// Разбор отказа. Причина у Spotify лежит в `error.reason`, и именно она
/// отличает «некуда играть» от «нет прав».
fn classify(status: StatusCode, wait: Duration, body: &str) -> ApiError {
    let detail = serde_json::from_str::<ApiErrorEnvelope>(body).ok().map(|envelope| envelope.error);
    let reason = detail.as_ref().and_then(|error| error.reason.clone());
    let message = detail
        .as_ref()
        .and_then(|error| error.message.clone())
        .unwrap_or_else(|| status.to_string());

    if reason.as_deref() == Some("NO_ACTIVE_DEVICE") {
        return ApiError::NoActiveDevice;
    }

    match status {
        StatusCode::UNAUTHORIZED => ApiError::Unauthorized,
        StatusCode::FORBIDDEN => match reason.as_deref() {
            Some("PREMIUM_REQUIRED") => {
                ApiError::Forbidden(tr!("spotify.error.premium"))
            }
            _ if message.contains("Restriction violated") => ApiError::Restricted,
            _ => ApiError::Forbidden(refused(&message)),
        },
        StatusCode::NOT_FOUND => ApiError::NoActiveDevice,
        StatusCode::TOO_MANY_REQUESTS => ApiError::RateLimited(wait),
        _ => ApiError::Other(message),
    }
}

/* ==================== Разбор ответов ==================== */

#[derive(Deserialize)]
struct ApiErrorEnvelope {
    error: ApiErrorDto,
}

#[derive(Deserialize)]
struct ApiErrorDto {
    message: Option<String>,
    reason: Option<String>,
}

#[derive(Deserialize)]
struct PlaybackDto {
    #[serde(default)]
    is_playing: bool,
    #[serde(default)]
    progress_ms: Option<u64>,
    #[serde(default)]
    shuffle_state: Option<bool>,
    #[serde(default)]
    repeat_state: Option<String>,
    #[serde(default)]
    device: Option<DeviceDto>,
    #[serde(default)]
    item: Option<ItemDto>,
}

impl PlaybackDto {
    fn into_player(self) -> Option<Player> {
        // Устройство может отвечать, не имея на руках трека — показывать
        // тогда нечего, это то же «плеер молчит»
        let item = self.item?;
        let device = self.device.unwrap_or_default();

        let artists = if item.artists.is_empty() {
            // У подкаста вместо исполнителей — передача
            item.show.as_ref().and_then(|show| show.name.clone()).unwrap_or_default()
        } else {
            item.artists
                .iter()
                .filter_map(|artist| artist.name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        };

        let album = item
            .album
            .as_ref()
            .and_then(|album| album.name.clone())
            .or_else(|| item.show.as_ref().and_then(|show| show.name.clone()))
            .unwrap_or_default();

        // Обложка у трека лежит в альбоме, у эпизода — прямо в нём самом
        let images = item
            .album
            .as_ref()
            .map(|album| album.images.as_slice())
            .filter(|images| !images.is_empty())
            .unwrap_or(item.images.as_slice());

        Some(Player {
            track: item.name.unwrap_or_default(),
            artists,
            album,
            cover_url: pick_cover(images),
            // Только свой адрес: строка уходит в браузер, а пришла из сети
            link: item
                .external_urls
                .and_then(|urls| urls.spotify)
                .filter(|url| url.starts_with("https://open.spotify.com/")),
            is_playing: self.is_playing,
            progress_ms: self.progress_ms.unwrap_or(0),
            duration_ms: item.duration_ms.unwrap_or(0),
            shuffle: self.shuffle_state.unwrap_or(false),
            repeat: self.repeat_state.unwrap_or_else(|| "off".to_owned()),
            volume: device.volume_percent,
            device: device.name,
            supports_volume: device.supports_volume.unwrap_or(true),
        })
    }
}

#[derive(Default, Deserialize)]
struct DeviceDto {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    volume_percent: Option<u8>,
    #[serde(default)]
    supports_volume: Option<bool>,
}

#[derive(Deserialize)]
struct ItemDto {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    duration_ms: Option<u64>,
    #[serde(default)]
    artists: Vec<ArtistDto>,
    #[serde(default)]
    album: Option<AlbumDto>,
    /// Эпизод подкаста: у него нет альбома и исполнителей
    #[serde(default)]
    show: Option<ShowDto>,
    #[serde(default, deserialize_with = "images")]
    images: Vec<ImageDto>,
    #[serde(default)]
    external_urls: Option<ExternalUrlsDto>,
}

#[derive(Deserialize)]
struct ExternalUrlsDto {
    #[serde(default)]
    spotify: Option<String>,
}

#[derive(Deserialize)]
struct ArtistDto {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct AlbumDto {
    #[serde(default)]
    name: Option<String>,
    #[serde(default, deserialize_with = "images")]
    images: Vec<ImageDto>,
}

#[derive(Deserialize)]
struct ShowDto {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Clone, Deserialize)]
struct ImageDto {
    url: String,
    #[serde(default)]
    width: Option<u32>,
}

/// Обложки, которых может не быть вовсе.
///
/// ВАЖНО: **`#[serde(default)]` спасает от пропущенного ключа, но не от
/// `null`.** Spotify присылает `"images": null` там, где обложки нет — у
/// свежесозданного пустого плейлиста, например, — и на таком ответе разбор
/// падает с «invalid type: null, expected a sequence». Падает при этом не одна
/// запись, а весь список: пользователь не открыл бы плейлисты вовсе из-за
/// единственного без картинки.
fn images<'de, D>(deserializer: D) -> Result<Vec<ImageDto>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<Vec<ImageDto>>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Deserialize)]
struct DeviceListDto {
    #[serde(default)]
    devices: Vec<DeviceEntryDto>,
}

#[derive(Deserialize)]
struct DeviceEntryDto {
    #[serde(default)]
    id: Option<String>,
}

#[derive(Deserialize)]
struct PlaylistPageDto {
    #[serde(default)]
    items: Vec<Option<PlaylistDto>>,
}

#[derive(Deserialize)]
struct PlaylistDto {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    uri: Option<String>,
    #[serde(default, deserialize_with = "images")]
    images: Vec<ImageDto>,
    #[serde(default)]
    tracks: Option<TracksDto>,
}

impl PlaylistDto {
    fn into_playlist(self) -> Option<Playlist> {
        Some(Playlist {
            id: self.id?,
            name: self.name.unwrap_or_default(),
            uri: self.uri?,
            cover_url: pick_cover(&self.images),
            tracks: self.tracks.map(|tracks| tracks.total).unwrap_or(0),
        })
    }
}

#[derive(Deserialize)]
struct TracksDto {
    #[serde(default)]
    total: u32,
}

/// Самая мелкая из достаточно крупных, иначе — самая крупная из имеющихся.
/// Spotify отдаёт 640/300/64: качать 640 ради миниатюры плейлиста незачем,
/// а довольствоваться 64 в шапке виджета — мыло.
fn pick_cover(images: &[ImageDto]) -> Option<String> {
    let big_enough = images
        .iter()
        .filter(|image| image.width.unwrap_or(0) >= COVER_MIN_WIDTH)
        .min_by_key(|image| image.width.unwrap_or(u32::MAX));

    big_enough
        .or_else(|| images.iter().max_by_key(|image| image.width.unwrap_or(0)))
        .map(|image| image.url.clone())
}

#[cfg(test)]
mod tests {

    /// Отказ на команду приходит от Spotify по-английски и с приставкой,
    /// которой человеку знать незачем. Строке жить под пультом одной строкой.
    ///
    /// «Restriction violated» сюда не доходит вовсе: его отбирает `classify`
    /// отдельным видом отказа, потому что решать по нему приходится не
    /// текстом, а сверкой с плеером.
    #[test]
    fn a_refused_command_speaks_russian() {
        // Незнакомую жалобу оставляем как есть, сняв только приставку
        assert_eq!(
            refused("Player command failed: Device not found"),
            "Device not found"
        );
        assert_eq!(refused("Something else"), "Something else");
    }
    use super::*;

    fn images(widths: &[u32]) -> Vec<ImageDto> {
        widths
            .iter()
            .map(|width| ImageDto {
                url: format!("https://i.scdn.co/{width}"),
                width: Some(*width),
            })
            .collect()
    }

    #[test]
    fn cover_prefers_smallest_sharp_enough() {
        let picked = pick_cover(&images(&[640, 300, 64]));
        assert_eq!(picked.as_deref(), Some("https://i.scdn.co/300"));
    }

    #[test]
    fn cover_falls_back_to_largest_when_all_are_small() {
        let picked = pick_cover(&images(&[64, 160]));
        assert_eq!(picked.as_deref(), Some("https://i.scdn.co/160"));
    }

    #[test]
    fn cover_absent_when_there_are_no_images() {
        assert_eq!(pick_cover(&[]), None);
    }

    #[test]
    fn reads_a_normal_track() {
        let body = r#"{
            "is_playing": true,
            "progress_ms": 42000,
            "shuffle_state": true,
            "repeat_state": "context",
            "device": { "name": "DESKTOP", "volume_percent": 65, "supports_volume": true },
            "item": {
                "name": "Aerials",
                "duration_ms": 236000,
                "artists": [{ "name": "System of a Down" }],
                "album": { "name": "Toxicity", "images": [{ "url": "https://i.scdn.co/x", "width": 640 }] }
            }
        }"#;

        let playback: PlaybackDto = serde_json::from_str(body).expect("ответ разбирается");
        let player = playback.into_player().expect("трек на месте");

        assert_eq!(player.track, "Aerials");
        assert_eq!(player.artists, "System of a Down");
        assert_eq!(player.album, "Toxicity");
        assert_eq!(player.progress_ms, 42000);
        assert_eq!(player.duration_ms, 236000);
        assert!(player.is_playing);
        assert!(player.shuffle);
        assert_eq!(player.repeat, "context");
        assert_eq!(player.volume, Some(65));
        assert_eq!(player.device.as_deref(), Some("DESKTOP"));
    }

    #[test]
    fn joins_several_artists() {
        let body = r#"{
            "is_playing": true,
            "item": {
                "name": "Under Pressure",
                "artists": [{ "name": "Queen" }, { "name": "David Bowie" }]
            }
        }"#;
        let playback: PlaybackDto = serde_json::from_str(body).expect("ответ разбирается");
        let player = playback.into_player().expect("трек на месте");
        assert_eq!(player.artists, "Queen, David Bowie");
    }

    /// У эпизода подкаста нет ни альбома, ни исполнителей — форма ответа
    /// другая, и разбор об неё спотыкаться не должен.
    #[test]
    fn survives_a_podcast_episode() {
        let body = r#"{
            "is_playing": true,
            "progress_ms": 1000,
            "item": {
                "name": "Выпуск 12",
                "duration_ms": 3600000,
                "show": { "name": "Подкаст" },
                "images": [{ "url": "https://i.scdn.co/ep", "width": 640 }]
            }
        }"#;

        let playback: PlaybackDto = serde_json::from_str(body).expect("ответ разбирается");
        let player = playback.into_player().expect("эпизод на месте");

        assert_eq!(player.track, "Выпуск 12");
        assert_eq!(player.artists, "Подкаст");
        assert_eq!(player.cover_url.as_deref(), Some("https://i.scdn.co/ep"));
    }

    /// Устройство отвечает, но трека нет — показывать нечего.
    #[test]
    fn no_item_means_nothing_to_show() {
        let body = r#"{ "is_playing": false, "device": { "name": "DESKTOP" } }"#;
        let playback: PlaybackDto = serde_json::from_str(body).expect("ответ разбирается");
        assert!(playback.into_player().is_none());
    }

    /// Пауза из заголовка нужна только 429, остальным отказам она безразлична.
    fn classify_body(status: StatusCode, body: &str) -> ApiError {
        classify(status, DEFAULT_COOLDOWN, body)
    }

    #[test]
    fn recognises_missing_device() {
        let body = r#"{"error":{"status":404,"message":"Player command failed","reason":"NO_ACTIVE_DEVICE"}}"#;
        assert!(matches!(
            classify_body(StatusCode::NOT_FOUND, body),
            ApiError::NoActiveDevice
        ));
    }

    #[test]
    fn recognises_expired_token() {
        let body = r#"{"error":{"status":401,"message":"The access token expired"}}"#;
        assert!(matches!(
            classify_body(StatusCode::UNAUTHORIZED, body),
            ApiError::Unauthorized
        ));
    }

    /// «Restriction violated» — свой вид отказа, а не разновидность запрета
    /// по правам: половину случаев им отвечают на команду, которая уже
    /// исполнена, и решать это должен вызывающий.
    #[test]
    fn a_violated_restriction_is_told_apart_from_a_ban() {
        let body = r#"{"error":{"status":403,"message":"Player command failed: Restriction violated","reason":"UNKNOWN"}}"#;
        let error = classify_body(StatusCode::FORBIDDEN, body);

        assert!(matches!(error, ApiError::Restricted), "вышло {error:?}");
        assert_eq!(error.message(), "Spotify не разрешил это действие");
    }

    #[test]
    fn explains_premium_requirement_in_plain_words() {
        let body = r#"{"error":{"status":403,"message":"Player command failed","reason":"PREMIUM_REQUIRED"}}"#;
        let message = classify_body(StatusCode::FORBIDDEN, body).message();
        assert!(message.contains("Premium"), "получилось: {message}");
    }

    /// 429 обязан приходить со сроком: без него опрос продолжил бы долбить
    /// в ту же стену, а каждый запрос под лимитом продлевает наказание.
    #[test]
    fn rate_limit_carries_the_wait() {
        let body = r#"{"error":{"status":429,"message":"API rate limit exceeded"}}"#;
        let error = classify(StatusCode::TOO_MANY_REQUESTS, Duration::from_secs(17), body);

        match error {
            ApiError::RateLimited(wait) => assert_eq!(wait, Duration::from_secs(17)),
            other => panic!("429 разобрался как {other:?}"),
        }
    }

    #[test]
    fn classify_survives_body_that_is_not_json() {
        let error = classify_body(StatusCode::BAD_GATEWAY, "<html>502</html>");
        assert!(matches!(error, ApiError::Other(_)));
    }

    #[test]
    fn playlists_skip_broken_entries() {
        let body = r#"{
            "items": [
                null,
                { "name": "Без идентификатора" },
                { "id": "1", "name": "Работа", "uri": "spotify:playlist:1",
                  "images": [{ "url": "https://i.scdn.co/p", "width": 300 }],
                  "tracks": { "total": 42 } }
            ]
        }"#;

        let page: PlaylistPageDto = serde_json::from_str(body).expect("страница разбирается");
        let list: Vec<Playlist> = page
            .items
            .into_iter()
            .flatten()
            .filter_map(PlaylistDto::into_playlist)
            .collect();

        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Работа");
        assert_eq!(list[0].tracks, 42);
    }

    /// Плейлист без обложки приходит с `"images": null`, и один такой не имеет
    /// права утащить за собой весь список: `#[serde(default)]` от `null` не
    /// спасает, спасает свой разбор.
    #[test]
    fn a_playlist_without_a_cover_does_not_break_the_list() {
        let body = r#"{
            "items": [
                { "id": "1", "name": "Пустой", "uri": "spotify:playlist:1",
                  "images": null, "tracks": { "total": 0 } }
            ]
        }"#;

        let page: PlaylistPageDto = serde_json::from_str(body).expect("страница разбирается");
        let list: Vec<Playlist> = page
            .items
            .into_iter()
            .flatten()
            .filter_map(PlaylistDto::into_playlist)
            .collect();

        assert_eq!(list.len(), 1);
        assert_eq!(list[0].cover_url, None, "обложке взяться неоткуда");
    }

    #[test]
    fn unknown_repeat_mode_becomes_off() {
        assert_eq!(repeat_mode("track"), "track");
        assert_eq!(repeat_mode("context"), "context");
        assert_eq!(repeat_mode("всё подряд"), "off");
    }

    #[test]
    fn actions_arrive_from_the_front_as_tagged_union() {
        let action: Action = serde_json::from_str(r#"{"kind":"volume","percent":70}"#)
            .expect("команда громкости разбирается");
        assert!(matches!(action, Action::Volume { percent: 70 }));

        let action: Action = serde_json::from_str(r#"{"kind":"playContext","uri":"spotify:playlist:1"}"#)
            .expect("команда плейлиста разбирается");
        assert!(matches!(action, Action::PlayContext { .. }));
    }

    /// Spotify отвечает «411 Length Required» на PUT без `Content-Length`,
    /// а reqwest для пустого тела заголовок не ставит. Проверка дешёвая,
    /// а без неё пульт молча перестаёт работать целиком.
    #[test]
    fn commands_without_body_declare_zero_length() {
        let client = reqwest::Client::new();
        let commands = [
            Action::Play,
            Action::Pause,
            Action::Next,
            Action::Previous,
            Action::Shuffle { on: true },
            Action::Repeat {
                mode: "track".to_owned(),
            },
            Action::Volume { percent: 40 },
            Action::Seek { position_ms: 1000 },
        ];

        for action in commands {
            let request = build(&client, &action)
                .build()
                .expect("запрос собирается");
            let length = request
                .headers()
                .get(reqwest::header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok());

            assert_eq!(length, Some("0"), "{action:?} уйдёт без длины тела");
        }
    }

    /// У команды с телом длину проставляет сам reqwest.
    #[test]
    fn playing_a_context_sends_json() {
        let client = reqwest::Client::new();
        let request = build(
            &client,
            &Action::PlayContext {
                uri: "spotify:playlist:1".to_owned(),
            },
        )
        .build()
        .expect("запрос собирается");

        assert!(request.body().is_some(), "тело команды потерялось");
        assert_eq!(
            request
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("application/json")
        );
    }

    /// Поля внутри вариантов тоже приезжают в верблюжьем регистре.
    #[test]
    fn seek_position_arrives_in_camel_case() {
        let action: Action = serde_json::from_str(r#"{"kind":"seek","positionMs":12345}"#)
            .expect("перемотка разбирается");
        assert!(matches!(action, Action::Seek { position_ms: 12345 }));
    }
}
