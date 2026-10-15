//! Виджет-пульт Spotify.
//!
//! Звук виджет не воспроизводит, и не потому, что не может: Widevine в
//! WebView2 есть, замерен живой пробой и даёт L3, которого Web Playback SDK и
//! хватило бы. Не воспроизводит потому, что незачем — звук из окна виджета
//! вместо десктопного Спотика ничего не улучшает, зато тянет своё устройство
//! Connect, разбор состояния плеера и вторую громкость. Играет десктопный
//! Спотик или веб-плеер, а мы через Web API показываем, что там сейчас, и
//! командуем.

mod api;
mod auth;
mod callback;
mod pkce;
mod store;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use auth::{AccessToken, AuthError};
use store::{Stored, Store};

pub use api::{Action, Player, Playlist};

/// Метка окна пульта. Совпадение с реестром держит тест в конце файла.
const WINDOW: &str = "spotify";

/// Только что нажали кнопку: человек стоит у виджета и ждёт ответа.
///
/// Частим здесь не ради ползунка, а ради подтверждения: Spotify меняет
/// состояние плеера позже, чем отвечает на команду, и первый же опрос после
/// нажатия нередко приносит ещё прежний трек.
const HOT_POLL: Duration = Duration::from_secs(1);

/// Сколько держится эта частота после нажатия.
const HOT_WINDOW: Duration = Duration::from_secs(10);

/// Человек ещё рядом, но кнопок больше не жмёт.
const WARM_POLL: Duration = Duration::from_secs(2);

/// Докуда доходит эта середина, считая от нажатия.
const WARM_WINDOW: Duration = Duration::from_secs(60);

/// Играет, но пульт давно не трогали: трек сменится сам через три-четыре
/// минуты, и увидеть это на пять секунд позже никому не мешает. Ползунок
/// между тиками фронт двигает сам.
const CALM_POLL: Duration = Duration::from_secs(5);

/// Никто не играет — меняться нечему.
///
/// ВАЖНО: прежние две секунды круглые сутки — это сорок тысяч запросов в
/// день, и однажды они кончились отказом 429 на пустом месте: музыку не
/// слушали, виджет не трогали. Лимит у Spotify общий на приложение и считается
/// скользящим окном, а у приложения в режиме разработки он ещё и ниже.
const IDLE_POLL: Duration = Duration::from_secs(10);

/// Окна не видно или идёт вход: в сеть не ходим вовсе, только проверяем,
/// не пора ли снова. Смотрим на видимость, а не на наличие окна — рубильник
/// «спрятать все» их не уничтожает.
const ASLEEP_POLL: Duration = Duration::from_secs(2);

/// После отказа не частим: причина редко проходит за пару секунд.
const ERROR_POLL: Duration = Duration::from_secs(30);

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Сколько ждём, пока человек разберётся со страницей входа.
const AUTH_TIMEOUT: Duration = Duration::from_secs(180);

/// Spotify отвечает на команду раньше, чем меняет состояние плеера. Без этой
/// паузы ответ придёт со старой картинкой и кнопка мигнёт обратно.
const SETTLE_DELAY: Duration = Duration::from_millis(400);

/// Когда переспросить плеер, если с первого раза просимое ещё не сбылось.
///
/// ВАЖНО: **одного взгляда мало.** Отказ «Restriction violated» приходит как
/// раз тогда, когда предыдущая команда ещё применяется, — а значит и новое
/// состояние доходит до плеера позже самого отказа. Пойманный живьём случай:
/// нажали «играть», сразу вслед «паузу»; пауза встала, но через 400 мс Spotify
/// всё ещё показывал играющий плеер, и под пультом краснела строка о запрете
/// того, что на самом деле только что случилось.
const RESTRICTION_RECHECK: Duration = Duration::from_millis(900);

const EVENT: &str = "spotify";


#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum View {
    /// Приложение в дашборде ещё не заведено.
    NeedsClientId,
    /// Client ID есть, входа не было. Отдаём его обратно, чтобы после
    /// отключения не пришлось набирать двадцать два знака заново.
    NeedsAuth { client_id: String },
    /// Идёт диалог в браузере.
    Authorizing,
    /// Подключены, но играть некому. Пульт при этом не пустует: показываем
    /// последний виденный трек, чтобы виджет всегда выглядел плеером.
    ///
    /// Подсказки для пульта без единого трека тут нет: она статична и живёт в
    /// каталоге строк фронта, а не приезжает каждым снимком.
    Idle {
        last: Option<Player>,
    },
    Active {
        player: Player,
    },
    Error {
        reason: String,
    },
}

/// Отказ команды пульта.
///
/// «Играть некуда» идёт своим видом, а не текстом: на него карточка отвечает
/// не строкой отказа, а слоем запуска — подсказкой и двумя кнопками, которые
/// открывают Спотифай. Различать это по тексту значило бы сверять строки
/// между Rust и фронтом.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", content = "reason", rename_all = "camelCase")]
pub enum ControlError {
    /// Нажали «играть», а Спотифай нигде не открыт: среди устройств пусто.
    Nowhere,
    Failed(String),
}

impl From<String> for ControlError {
    fn from(reason: String) -> Self {
        ControlError::Failed(reason)
    }
}

impl View {
    /// Без Client ID показывать нечего даже кнопке входа.
    fn needs_auth(credentials: &Stored) -> Self {
        match credentials.client_id() {
            Some(client_id) => View::NeedsAuth {
                client_id: client_id.to_owned(),
            },
            None => View::NeedsClientId,
        }
    }
}

/// Куда переходить при запуске Спотифая: в установленное приложение или в веб-плеер.
///
/// Перечисление, а не строка: фронт не должен уметь передать в explorer.exe
/// ничего, кроме этих двух адресов.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Place {
    App,
    Web,
}

impl Place {
    pub fn target(self) -> &'static str {
        match self {
            Place::App => "spotify:",
            Place::Web => "https://open.spotify.com",
        }
    }
}

pub struct Session {
    client: reqwest::Client,
    store: Store,
    /// Токен доступа живёт час и на диск не попадает — восстановим из
    /// refresh token. Мьютекс асинхронный: под ним ходят в сеть.
    access: tokio::sync::Mutex<Option<AccessToken>>,
    view: Mutex<View>,
    /// Пока человек в браузере, опрос молчит: токена ещё нет, а мигать
    /// ошибками каждые две секунды незачем.
    authorizing: AtomicBool,
    /// До этого момента к Spotify не ходим — он сам попросил.
    ///
    /// Общий на опрос и на кнопки: лимит у Spotify один на приложение, и
    /// каждый запрос под ним только продлевает наказание.
    quiet_until: Mutex<Option<Instant>>,
    /// Когда последний раз нажимали на пульте.
    ///
    /// От него считается частота опроса: сразу после нажатия секунда, дальше
    /// две, потом пять. Человек у виджета и виджет на втором мониторе — это
    /// две разные задачи, и одной частотой они не решаются: под первую нужна
    /// отзывчивость, под вторую — не сжечь лимит за ночь.
    touched: Mutex<Option<Instant>>,
}

impl Session {
    pub fn new(app: &AppHandle) -> Self {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("HTTP-клиент собирается");

        Self {
            client,
            store: Store::new(store::load(app)),
            access: tokio::sync::Mutex::new(None),
            view: Mutex::new(View::NeedsClientId),
            authorizing: AtomicBool::new(false),
            quiet_until: Mutex::new(None),
            touched: Mutex::new(None),
        }
    }

    /// Отмечает нажатие на пульте.
    fn touch(&self) {
        *self
            .touched
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Instant::now());
    }

    /// Сколько прошло с последнего нажатия. `None` — не нажимали вовсе.
    fn touched_ago(&self) -> Option<Duration> {
        self.touched
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .map(|moment| moment.elapsed())
    }

    /// Метка последнего нажатия как она есть — по ней сон узнаёт, что за его
    /// время на пульте что-то нажали.
    fn touched_at(&self) -> Option<Instant> {
        *self
            .touched
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Сколько ещё молчать по просьбе Spotify.
    fn quiet_for(&self) -> Option<Duration> {
        let until = *self
            .quiet_until
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        until.and_then(|moment| moment.checked_duration_since(Instant::now()))
    }

    /// Запоминает просьбу подождать. Более долгую не укорачиваем: два отказа
    /// подряд означают, что первого срока не хватило.
    fn hush(&self, wait: Duration) {
        let mut until = self
            .quiet_until
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let moment = Instant::now() + wait;

        if until.is_none_or(|current| current < moment) {
            *until = Some(moment);
        }
    }

    /// Отказ по лимиту запоминаем, откуда бы он ни пришёл: и опрос, и кнопки
    /// ходят в один и тот же счётчик.
    fn note(&self, error: &api::ApiError) {
        if let api::ApiError::RateLimited(wait) = error {
            self.hush(*wait);
        }
    }

    fn snapshot(&self) -> View {
        // Отравленный мьютекс не повод падать: данные внутри осмысленные
        self.view
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn publish(&self, app: &AppHandle, view: View) {
        *self
            .view
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = view.clone();

        // Неудачу рассылки глотаем: окно могли закрыть прямо сейчас
        let _ = app.emit(EVENT, view);
    }

    fn persist(&self, app: &AppHandle, change: impl FnOnce(&mut Stored) -> Result<(), String>) -> Result<Stored, String> {
        self.store.try_update(change, |next| store::save(app, next))
            .map(|(stored, ())| stored)
            .map_err(|error| {
                eprintln!("учётные данные Spotify не сохранились: {error}");
                tr!("common.notSaved")
            })
    }

    /// Токен доступа, при необходимости обновлённый.
    ///
    /// `force` нужен для повтора после 401: срок ещё не вышел, а токен уже
    /// не принимают — значит, кеш врёт и обновляться надо немедленно.
    async fn access_token(&self, app: &AppHandle, force: bool) -> Result<String, TokenError> {
        let mut cached = self.access.lock().await;

        if !force {
            if let Some(token) = cached.as_ref().filter(|token| token.is_fresh()) {
                return Ok(token.value.clone());
            }
        }

        let credentials = self.store.get();
        let client_id = credentials.client_id().ok_or(TokenError::NeedsAuth)?;
        let refresh_token = credentials.refresh_token().ok_or(TokenError::NeedsAuth)?;

        match auth::refresh(&self.client, client_id, refresh_token).await {
            Ok(tokens) => {
                // Пока ответ был в сети, подключение могли сменить. Кеш и
                // диск принимают ответ только для тех же реквизитов.
                if !self.store.get().same_connection(&credentials) {
                    return Err(TokenError::NeedsAuth);
                }
                if let Some(next) = tokens.refresh {
                    self.persist(app, |current| {
                        if !current.same_connection(&credentials) {
                            return Err(tr!("spotify.error.needsAuth"));
                        }
                        current.refresh_token = Some(next);
                        Ok(())
                    }).map_err(TokenError::Failed)?;
                }

                let value = tokens.access.value.clone();
                *cached = Some(tokens.access);
                Ok(value)
            }
            Err(AuthError::Rejected(detail)) => {
                // Подключение отозвали — хранить мёртвый токен незачем
                eprintln!("Spotify отверг подключение: {detail}");
                *cached = None;
                if self.store.get().same_connection(&credentials) {
                    self.persist(app, |current| {
                        if !current.same_connection(&credentials) {
                            return Err(tr!("spotify.error.needsAuth"));
                        }
                        current.refresh_token = None;
                        Ok(())
                    }).map_err(TokenError::Failed)?;
                }
                Err(TokenError::NeedsAuth)
            }
            Err(AuthError::Transport(detail)) => Err(TokenError::Failed(detail)),
        }
    }

    /// Никто не играет. Показываем последний трек, если он известен.
    fn idle(&self) -> View {
        View::Idle {
            last: self.store.get().last_player,
        }
    }

    /// Запоминает трек на диск — но только при его смене.
    ///
    /// Опрос идёт раз в две секунды, и писать файл с такой частотой незачем.
    /// Признак «играет» гасим: сохранённый трек уже никуда не движется, и
    /// иначе виджет досчитывал бы ползунок от давно прошедшего момента.
    fn remember(&self, app: &AppHandle, player: &Player) {
        let stored = self.store.get();
        if stored.refresh_token().is_none() || stored.last_player.as_ref().is_some_and(|last| {
            last.track == player.track && last.artists == player.artists
        }) {
            return;
        }

        let _ = self.persist(app, |current| {
            // Запоздалый опрос не возвращает трек после отключения или
            // смены подключения; токен свежего снимка оставляем как есть.
            if current.refresh_token().is_none() || current.client_id() != stored.client_id() {
                return Err(tr!("spotify.error.needsAuth"));
            }
            current.last_player = Some(Player { is_playing: false, ..player.clone() });
            Ok(())
        });
    }

    /// Будит первое доступное устройство: без этого «играть» при закрытом
    /// плеере отвечает «нет активного устройства» и ничего не делает.
    async fn wake_a_device(&self, token: &str) -> Result<(), ControlError> {
        let devices = api::devices(&self.client, token).await.map_err(|error| {
            self.note(&error);
            error.message()
        })?;

        let Some(first) = devices.first() else {
            return Err(ControlError::Nowhere);
        };

        api::transfer(&self.client, token, first)
            .await
            .map_err(|error| {
                self.note(&error);
                ControlError::Failed(error.message())
            })
    }

    /// Снимает состояние плеера и переводит его в то, что покажет виджет.
    ///
    /// `None` означает «сказать нечего, оставьте прежнюю картинку». Так
    /// отвечаем на просьбу подождать: на пульте она означала бы красную
    /// строку вместо играющего трека, хотя ничего не сломалось.
    async fn observe(&self, app: &AppHandle) -> Option<View> {
        let credentials = self.store.get();
        if credentials.client_id().is_none() {
            return Some(View::NeedsClientId);
        }
        if credentials.refresh_token().is_none() {
            return Some(View::needs_auth(&credentials));
        }

        let mut forced = false;
        loop {
            let token = match self.access_token(app, forced).await {
                Ok(token) => token,
                Err(TokenError::NeedsAuth) => return Some(View::needs_auth(&credentials)),
                Err(TokenError::Failed(reason)) => return Some(View::Error { reason }),
            };

            return match api::player(&self.client, &token).await {
                Ok(Some(player)) => {
                    self.remember(app, &player);
                    Some(View::Active { player })
                }
                // Пустой ответ и «нет активного устройства» — одно и то же
                // с точки зрения виджета: сейчас никто не играет
                Ok(None) | Err(api::ApiError::NoActiveDevice) => Some(self.idle()),
                Err(api::ApiError::Unauthorized) if !forced => {
                    forced = true;
                    continue;
                }
                Err(api::ApiError::RateLimited(wait)) => {
                    self.hush(wait);
                    None
                }
                // До Spotify не достучались — картинку не трогаем. Пульт
                // краснеет, когда сломалось у него, а не когда занят канал:
                // запущенный замер скорости валил запрос к плееру каждый раз
                Err(api::ApiError::Unreachable(detail)) => {
                    eprintln!("[спотифай] {detail}");
                    None
                }
                Err(error) => Some(View::Error {
                    reason: error.message(),
                }),
            };
        }
    }

    async fn authorize(&self, app: &AppHandle, client_id: String) -> Result<(), String> {
        // Client ID сохраняем сразу: даже если вход сорвётся, набирать его
        // заново не придётся
        {
            let _access = self.access.lock().await;
            self.persist(app, |current| {
                current.client_id = Some(client_id.clone());
                Ok(())
            })?;
        }

        let callback = callback::bind().await?;
        let secrets = pkce::Pkce::generate()?;
        let url = auth::authorize_url(
            &client_id,
            &callback.redirect_uri,
            &secrets.challenge,
            &secrets.state,
        );

        // Открываем системный браузер, а не своё окно: там человек уже вошёл
        // в Спотифай, а пароль во встроенном webview вводить незачем.
        //
        // Через `links.rs`, как любую веб-ссылку, а не плагином opener: тот
        // идёт в `ShellExecute` прямо из потока, где его позвали, а это COM, на
        // котором держится WebView2. Не открылся браузер — вход
        // кончится сроком ожидания ниже, а причина уйдёт в лог
        crate::links::open_link(url);

        let code = tokio::time::timeout(AUTH_TIMEOUT, callback.wait_for_code(&secrets.state))
            .await
            .map_err(|_| tr!("spotify.error.loginTimeout"))??;

        let tokens = auth::exchange_code(
            &self.client,
            &client_id,
            &callback.redirect_uri,
            &code,
            &secrets.verifier,
        )
        .await
        .map_err(|error| error.message())?;

        let refresh = tokens
            .refresh
            .ok_or_else(|| tr!("spotify.error.noRefresh"))?;

        let mut access = self.access.lock().await;
        self.persist(app, |current| {
            current.refresh_token = Some(refresh);
            Ok(())
        })?;
        *access = Some(tokens.access);
        Ok(())
    }
}

/// Поднятый признак «идёт вход», опускающийся сам.
///
/// ВАЖНО: **строкой после `await` этого не добиться.** Паника внутри задачи
/// уносит её целиком, и такая строка не исполняется вовсе: опрос плеера остался
/// бы спящим, а кнопка входа отвечала бы «вход уже начат» до перезапуска
/// приложения. `Drop` случается и при развёртывании стека.
struct Authorizing<'a>(&'a AtomicBool);

impl Drop for Authorizing<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Команды, которые сами начинают воспроизведение. Только им есть смысл
/// будить устройство: «пауза» на закрытом плеере ничего не значит.
fn starts_playback(action: &Action) -> bool {
    matches!(action, Action::Play | Action::PlayContext { .. })
}

/// Сбылось ли просимое на самом деле — с двумя взглядами на плеер.
///
/// Первый взгляд ловит команду, которая была исполнена ещё до нашей (человек
/// нажал по картинке, снятой несколько секунд назад). Второй нужен для
/// обратного случая: наша команда всё-таки прошла, но Spotify показал её
/// позже, чем отказал. Свежую картинку публикуем в любом случае — она полезна
/// и тогда, когда просимое не сбылось.
async fn confirmed(
    app: &AppHandle,
    session: &Session,
    action: &Action,
    before: Option<&Player>,
) -> bool {
    for pause in [SETTLE_DELAY, RESTRICTION_RECHECK] {
        tokio::time::sleep(pause).await;

        let Some(view) = session.observe(app).await else {
            continue;
        };

        let done = matches!(
            &view,
            View::Active { player } if already_done(action, before, player)
        );
        session.publish(app, view);

        if done {
            return true;
        }
    }

    false
}

/// Сбылось ли то, о чём просили.
///
/// ВАЖНО: **отказ «Restriction violated» половину случаев означает опоздание,
/// а не запрет.** Тем же 403 Spotify встречает «паузу» на уже остановленном
/// плеере, «play» на играющем и второе нажатие «дальше», пока первое ещё в
/// пути. Человек при этом не сделал ничего дурного: он нажал по картинке,
/// которой несколько секунд, — или нажал дважды, не дождавшись ответа. Красная
/// строка под пультом в этом случае врёт, поэтому вместо неё смотрим, что на
/// плеере на самом деле.
///
/// Настоящие запреты — «назад» в начале списка, перемешивание там, где его нет
/// — этой проверки не проходят, и строка остаётся при них.
fn already_done(action: &Action, before: Option<&Player>, now: &Player) -> bool {
    match action {
        Action::Play | Action::PlayContext { .. } => now.is_playing,
        Action::Pause => !now.is_playing,
        Action::Shuffle { on } => now.shuffle == *on,
        Action::Repeat { mode } => now.repeat == api::repeat_mode(mode),
        // Трек сменился — значит переключение прошло, чьё бы нажатие ни было
        // первым. Не с чем сравнить — считаем, что не прошло
        Action::Next | Action::Previous => {
            before.is_some_and(|was| !was.track.is_empty() && was.track != now.track)
        }
        // Громкость и перемотка: у них нет состояния, к которому можно
        // придраться, и отказ по ним всегда настоящий
        Action::Volume { .. } | Action::Seek { .. } => false,
    }
}

enum TokenError {
    /// Подключаться надо заново — руками, через браузер.
    NeedsAuth,
    Failed(String),
}

impl TokenError {
    fn message(&self) -> String {
        match self {
            TokenError::NeedsAuth => tr!("spotify.error.needsAuth"),
            TokenError::Failed(detail) => detail.clone(),
        }
    }
}

/// Опрос плеера. Живёт до конца работы приложения.
///
/// Шаг у него неровный: постоянный интервал слишком дорого обходится, когда
/// смотреть не на что.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let delay = poll_once(&app).await;
            nap(&app, delay).await;
        }
    });
}

/// Сон между заходами — шагами, а не одним куском.
///
/// ВАЖНО: **нажатие на пульте обязано обрывать сон.** Кнопка меняет плеер
/// сразу, но новую картинку приносит следующий заход опроса, и целым куском
/// он ждал бы своего срока — до десяти секунд при остановленной музыке.
/// Снаружи это выглядит как «трек переключился, а виджет не заметил», и на
/// это пожаловались с живых окон. Шаг сна равен [`HOT_POLL`]: чаще проверять
/// незачем, чаще самой частой частоты опрос всё равно не пойдёт.
///
/// Пустое пробуждение таймера в секунду стоит пустяк — в сеть оно не ходит.
async fn nap(app: &AppHandle, delay: Duration) {
    let Some(session) = app.try_state::<Session>() else {
        tokio::time::sleep(delay).await;
        return;
    };

    let touched = session.touched_at();
    let mut left = delay;

    while !left.is_zero() {
        let step = left.min(HOT_POLL);
        tokio::time::sleep(step).await;
        left -= step;

        if session.touched_at() != touched {
            return;
        }
    }
}

/// Один заход опроса. Отдаёт срок до следующего.
async fn poll_once(app: &AppHandle) -> Duration {
    let Some(session) = app.try_state::<Session>() else {
        return ASLEEP_POLL;
    };

    // Пока человек в браузере, токена ещё нет
    if session.authorizing.load(Ordering::Relaxed) {
        return ASLEEP_POLL;
    }

    // Окно выключено в настройках или спрятано рубильником — спрашивать не для
    // кого. Прежде опрос шёл всегда, и именно этим виджет однажды сжёг лимит
    // за ночь, ничего никому не показывая.
    //
    // Смотрим на видимость, а не на наличие: «спрятать все» окна не
    // уничтожает, и по одному лишь наличию опрос продолжался бы под
    // спрятанным пультом.
    let watched = app
        .get_webview_window(WINDOW)
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false);

    if !watched {
        return ASLEEP_POLL;
    }

    if let Some(left) = session.quiet_for() {
        return left;
    }

    let asked_at = Instant::now();

    match session.observe(app).await {
        Some(view) => {
            let next = cadence(&view, session.touched_ago());

            // ВАЖНО: снимок, начатый до нажатия, показывать нельзя — он
            // старше того, что уже видно в окне. Кнопка «пауза» от такого
            // мигала плей-паузой-плеем: сперва фронт показывал ожидаемое,
            // потом сюда приезжал заказанный ещё до нажатия ответ с играющим
            // плеером, и только следующий тик возвращал правду
            if session.touched_at().is_none_or(|moment| moment < asked_at) {
                session.publish(app, view);
            }

            next
        }
        // Промолчали по просьбе Spotify — столько и ждём
        None => session.quiet_for().unwrap_or(IDLE_POLL),
    }
}

/// Шаг опроса под то, что сейчас на пульте и как давно его трогали.
///
/// Частота ступенчатая и сама успокаивается: секунда сразу после нажатия, две
/// в ближайшую минуту, пять дальше. Виджет висит на мониторе сутками, а
/// человек стоит у него минутами — под первое нужна экономность, под второе
/// отзывчивость, и одним числом это не выражается.
///
/// ВАЖНО: **само по себе «играет» частить не повод, а нажатие — повод даже на
/// паузе.** На паузе Spotify отвечает не пустотой, а полным телом с
/// `is_playing: false`, и пульт остаётся `Active` — то есть по одному лишь
/// варианту `View` пауза неотличима от игры. Пока десктопный Спотик открыт,
/// такая пауза держится часами, и две секунды на ней возвращали ровно то, от
/// чего [`IDLE_POLL`] и заводили: сорок тысяч запросов в сутки и отказ 429 на
/// пустом месте. Зато нажатую при остановленной музыке кнопку надо
/// подтвердить так же быстро, как при играющей.
fn cadence(view: &View, touched_ago: Option<Duration>) -> Duration {
    let fresh = |window: Duration| touched_ago.is_some_and(|since| since < window);

    match view {
        View::Error { .. } => ERROR_POLL,
        View::Active { player } if player.is_playing => {
            if fresh(HOT_WINDOW) {
                HOT_POLL
            } else if fresh(WARM_WINDOW) {
                WARM_POLL
            } else {
                CALM_POLL
            }
        }
        _ if fresh(HOT_WINDOW) => HOT_POLL,
        _ => IDLE_POLL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Форму отказа читает `failureOf` в `src/lib/spotify.ts`: разойдись имя
    /// поля или вида — «играть некуда» пришло бы строкой отказа, и слой запуска
    /// не всплыл бы никогда.
    #[test]
    fn control_errors_keep_the_shape_the_card_reads() {
        assert_eq!(
            serde_json::to_value(ControlError::Nowhere).unwrap(),
            serde_json::json!({ "kind": "nowhere" })
        );
        assert_eq!(
            serde_json::to_value(ControlError::Failed("нет".to_owned())).unwrap(),
            serde_json::json!({ "kind": "failed", "reason": "нет" })
        );
    }

    /// Опрос молчит, пока окна нет, и находит его по метке. Разойдись она с
    /// реестром — пульт замолчал бы навсегда, причём тихо.
    #[test]
    fn window_label_matches_the_registry() {
        assert!(
            crate::widgets::find(WINDOW).is_some(),
            "метка окна «{WINDOW}» не совпадает с реестром виджетов"
        );
    }

    fn playing() -> View {
        View::Active {
            player: Player {
                is_playing: true,
                ..Player::default()
            },
        }
    }

    fn paused() -> View {
        View::Active {
            player: Player {
                is_playing: false,
                ..Player::default()
            },
        }
    }

    fn idle() -> View {
        View::Idle { last: None }
    }

    /// Нетронутый пульт: играющий опрашиваем чаще молчащего, а сломанное —
    /// реже всего, причина отказа за две секунды не проходит.
    #[test]
    fn cadence_spares_the_limit_when_there_is_nothing_to_watch() {
        let long_ago = Some(WARM_WINDOW * 2);

        let quiet = cadence(&idle(), long_ago);
        let active = cadence(&playing(), long_ago);
        let broken = cadence(
            &View::Error {
                reason: String::new(),
            },
            long_ago,
        );

        assert!(active < quiet, "играющий пульт опрашиваем чаще молчащего");
        assert!(quiet < broken, "сломанное опрашиваем реже всего");
    }

    /// Пауза — это тот же `Active` с живым треком, и без нажатий частить на
    /// ней нельзя: пульт стоит на мониторе сутками, а лимит у Spotify общий
    /// на приложение.
    #[test]
    fn a_paused_player_is_polled_as_rarely_as_a_silent_one() {
        assert_eq!(
            cadence(&paused(), Some(WARM_WINDOW * 2)),
            IDLE_POLL,
            "на паузе меняться нечему"
        );
        assert_eq!(
            cadence(&paused(), None),
            IDLE_POLL,
            "не нажимали вовсе — тем более"
        );
    }

    /// Свежее нажатие поднимает частоту до секунды — и на играющем пульте, и
    /// на остановленном: подтверждения ждут одинаково.
    #[test]
    fn a_fresh_press_speeds_the_poll_up() {
        let just_now = Some(Duration::from_secs(1));

        assert_eq!(cadence(&playing(), just_now), HOT_POLL);
        assert_eq!(cadence(&paused(), just_now), HOT_POLL);
        assert_eq!(cadence(&idle(), just_now), HOT_POLL);
    }

    /// Частота сама успокаивается: секунда, две, пять — по мере того, как
    /// нажатие уходит в прошлое.
    #[test]
    fn the_poll_calms_down_on_its_own() {
        let steps = [
            (Duration::from_secs(1), HOT_POLL),
            (HOT_WINDOW + Duration::from_secs(1), WARM_POLL),
            (WARM_WINDOW + Duration::from_secs(1), CALM_POLL),
        ];

        for (ago, expected) in steps {
            assert_eq!(
                cadence(&playing(), Some(ago)),
                expected,
                "через {ago:?} после нажатия шаг должен быть {expected:?}"
            );
        }
    }

    /// Нажатая по устаревшей картинке кнопка не должна краснеть: «пауза» на
    /// уже остановленном плеере — это опоздание пульта, а не запрет.
    #[test]
    fn a_command_that_was_already_true_counts_as_done() {
        let stopped = Player {
            is_playing: false,
            ..Player::default()
        };
        let going = Player {
            is_playing: true,
            ..Player::default()
        };

        assert!(already_done(&Action::Pause, None, &stopped));
        assert!(already_done(&Action::Play, None, &going));
        assert!(!already_done(&Action::Pause, None, &going));
        assert!(!already_done(&Action::Play, None, &stopped));
    }

    /// Второе нажатие «дальше» обгоняет первое и получает тот же отказ. Судим
    /// по треку: сменился — значит переключение прошло.
    #[test]
    fn a_skip_counts_as_done_when_the_track_has_changed() {
        let was = Player {
            track: "Daydream".to_owned(),
            ..Player::default()
        };
        let now = Player {
            track: "Sunny".to_owned(),
            ..Player::default()
        };

        assert!(already_done(&Action::Next, Some(&was), &now));
        assert!(!already_done(&Action::Next, Some(&was), &was));
        // Не с чем сравнивать — не выдумываем успех
        assert!(!already_done(&Action::Next, None, &now));
    }

    /// Настоящие запреты этой проверки не проходят: у громкости и перемотки
    /// нет состояния, по которому можно решить «уже сделано».
    #[test]
    fn a_real_refusal_stays_a_refusal() {
        let player = Player::default();

        assert!(!already_done(&Action::Volume { percent: 50 }, None, &player));
        assert!(!already_done(&Action::Seek { position_ms: 0 }, None, &player));
    }

    /// Перемешивание и повтор сверяются со своим же состоянием, причём режим
    /// повтора приводится к тому виду, в котором его знает Spotify.
    #[test]
    fn shuffle_and_repeat_check_their_own_state() {
        let player = Player {
            shuffle: true,
            repeat: "context".to_owned(),
            ..Player::default()
        };

        assert!(already_done(&Action::Shuffle { on: true }, None, &player));
        assert!(!already_done(&Action::Shuffle { on: false }, None, &player));
        assert!(already_done(
            &Action::Repeat {
                mode: "context".to_owned()
            },
            None,
            &player
        ));
        assert!(!already_done(
            &Action::Repeat {
                mode: "track".to_owned()
            },
            None,
            &player
        ));
    }

    /// Отказ перебивает любую свежесть нажатия: под 429 частый опрос только
    /// продлевает наказание.
    #[test]
    fn a_broken_remote_stays_slow_even_right_after_a_press() {
        let view = View::Error {
            reason: String::new(),
        };

        assert_eq!(cadence(&view, Some(Duration::from_secs(1))), ERROR_POLL);
    }

    /// "app" и "web" разбираются в Place, чужая строка ("file") — ошибка.
    #[test]
    fn place_deserializes_app_and_web_and_rejects_unknown() {
        assert_eq!(serde_json::from_str::<Place>("\"app\"").unwrap(), Place::App);
        assert_eq!(serde_json::from_str::<Place>("\"web\"").unwrap(), Place::Web);
        assert!(serde_json::from_str::<Place>("\"file\"").is_err());
    }

    /// Адрес Web проходит проверку безопасности ссылок, а адрес App ровно "spotify:".
    #[test]
    fn place_targets_are_safe_and_exact() {
        assert!(crate::links::is_safe_link(Place::Web.target()));
        assert_eq!(Place::App.target(), "spotify:");
    }
}

/* ==================== Команды ==================== */

/// Текущее состояние — окно спрашивает его при открытии.
#[tauri::command]
pub fn spotify_view(session: State<'_, Session>) -> View {
    session.snapshot()
}

#[tauri::command]
pub async fn spotify_connect(
    app: AppHandle,
    session: State<'_, Session>,
    client_id: String,
) -> Result<(), String> {
    let client_id = client_id.trim().to_owned();
    if client_id.is_empty() {
        return Err("Client ID пуст — возьмите его в Spotify Developer Dashboard".to_owned());
    }

    // swap отдаёт прежнее значение: если там уже true, вход идёт в другой
    // вкладке и второй сервер на том же порту не поднимется
    if session.authorizing.swap(true, Ordering::SeqCst) {
        return Err("вход уже начат — закончите его в браузере".to_owned());
    }
    let started = Authorizing(&session.authorizing);
    session.publish(&app, View::Authorizing);

    let outcome = session.authorize(&app, client_id).await;
    // Дальше идёт обычный опрос, и молчать ему больше незачем
    drop(started);

    // Состояние пересчитываем в любом случае: после отказа виджет должен
    // вернуться к кнопке входа, а не застрять в «идёт авторизация»
    if let Some(view) = session.observe(&app).await {
        session.publish(&app, view);
    }

    outcome
}

/// Забывает подключение, но оставляет Client ID: войти снова — одна кнопка.
///
/// Последний трек уходит вместе с токеном: политика разработчика Spotify
/// (I.1.b) требует после отключения удалить данные пользователя, а что он
/// слушал — именно они. Client ID — свой у человека, но это настройка, а не
/// данные из Spotify.
#[tauri::command]
pub async fn spotify_disconnect(
    app: AppHandle,
    session: State<'_, Session>,
) -> Result<(), String> {
    // Тот же порядок замков, что у обновления: access, затем Store.
    // Отключение не даст запоздалому refresh снова наполнить кеш.
    let mut access = session.access.lock().await;
    let credentials = session.persist(&app, |current| {
        current.refresh_token = None;
        current.last_player = None;
        Ok(())
    })?;
    *access = None;
    session.publish(&app, View::needs_auth(&credentials));
    Ok(())
}

#[tauri::command]
pub async fn spotify_control(
    app: AppHandle,
    session: State<'_, Session>,
    action: Action,
) -> Result<(), ControlError> {
    // Кнопка под лимитом бесполезна вдвойне: команда не пройдёт, а запрос
    // зачтётся и продлит паузу
    if let Some(left) = session.quiet_for() {
        return Err(format!("Spotify просит подождать {} с", left.as_secs()).into());
    }

    let mut forced = false;
    let mut woken = false;

    // Что было на пульте до команды: по нему потом отличают сменившийся трек
    // от несменившегося, если Spotify ответит «здесь так нельзя»
    let before = match session.snapshot() {
        View::Active { player } => Some(player),
        View::Idle { last, .. } => last,
        _ => None,
    };

    loop {
        let token = session
            .access_token(&app, forced)
            .await
            .map_err(|error| error.message())?;

        match api::control(&session.client, &token, &action).await {
            Ok(()) => break,
            Err(api::ApiError::Unauthorized) if !forced => {
                forced = true;
                continue;
            }
            // Пульт нажали, когда плеер закрыт: выбираем устройство сами,
            // иначе пришлось бы идти в Спотифай и включать что-то руками
            Err(api::ApiError::NoActiveDevice) if !woken && starts_playback(&action) => {
                woken = true;
                session.wake_a_device(&token).await?;
                // Перенос воспроизведения Spotify подтверждает раньше, чем
                // выполняет: команда сразу вслед за ним снова упирается в
                // «нет активного устройства», и это видит уже пользователь
                tokio::time::sleep(SETTLE_DELAY).await;
                continue;
            }
            // «Здесь так нельзя» — но, может, оно уже и сделано: проверяем по
            // самому плееру, а не по картинке, с которой человек нажимал
            Err(api::ApiError::Restricted) => {
                session.touch();

                return if confirmed(&app, &session, &action, before.as_ref()).await {
                    Ok(())
                } else {
                    Err(api::ApiError::Restricted.message().into())
                };
            }
            Err(error) => {
                session.note(&error);
                return Err(error.message().into());
            }
        }
    }

    // Нажали — значит человек у виджета и ждёт подтверждения: опрос переходит
    // на секундный шаг и обрывает свой сон, не дожидаясь срока
    session.touch();

    // Не ждём очередного тика: пульт должен отзываться сразу
    tokio::time::sleep(SETTLE_DELAY).await;
    if let Some(view) = session.observe(&app).await {
        session.publish(&app, view);
    }

    Ok(())
}

#[tauri::command]
pub async fn spotify_playlists(
    app: AppHandle,
    session: State<'_, Session>,
) -> Result<Vec<Playlist>, String> {
    if let Some(left) = session.quiet_for() {
        return Err(format!("Spotify просит подождать {} с", left.as_secs()));
    }

    let mut forced = false;
    loop {
        let token = session
            .access_token(&app, forced)
            .await
            .map_err(|error| error.message())?;

        match api::playlists(&session.client, &token).await {
            Ok(list) => return Ok(list),
            Err(api::ApiError::Unauthorized) if !forced => {
                forced = true;
                continue;
            }
            Err(error) => {
                session.note(&error);
                return Err(error.message());
            }
        }
    }
}

/// Открывает Спотифай в приложении или веб-плеере.
///
/// Команда синхронная, ответа не ждёт, отказ только в лог — как `open_link`.
#[tauri::command]
pub fn spotify_open(place: Place) {
    crate::links::launch(place.target());
}
