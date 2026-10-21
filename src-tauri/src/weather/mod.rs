//! Погода: Open-Meteo без ключа и без учётной записи.
//!
//! Устройство то же, что у прочих опросов: событие в окна и снимок по команде
//! для окна, открытого между тиками. Наружу торчит ещё поиск города — им
//! пользуется раздел «Погода» в настройках.

pub mod api;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Notify;

use crate::settings::{Place, SettingsState};
use api::Report;

/// Метка окна виджета: по ней опрос узнаёт, смотрит ли на него кто-нибудь.
const WIDGET: &str = "weather";

/// Событие, которым погода расходится по окнам.
const WEATHER_EVENT: &str = "weather";

/// Шаг опроса.
///
/// Четыре часа, а не четверть: прогноз на сутки и на пять дней за это время не
/// шевелится, а сервис чужой и бесплатный. Нынешняя температура при этом
/// стареет — и ровно затем в углу карточки стоит кнопка обновления, а первый
/// заход случается при каждом запуске приложения, то есть при каждом включении
/// машины.
const STEP: Duration = Duration::from_secs(4 * 60 * 60);

/// Пауза после отказа. Четверть часа ждать незачем: сеть возвращается быстрее.
const RETRY: Duration = Duration::from_secs(60);

/// Как часто проверяем, не пора ли. В сеть на этом шаге не ходим: смотрим
/// только на видимость окна и на выбранный город, а обе проверки бесплатные.
///
/// Без частой проверки выбранный в настройках город отзывался бы в карточке
/// через четверть часа.
const TICK: Duration = Duration::from_secs(5);

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Сколько отказов подряд терпим, прежде чем сказать о них в карточке.
///
/// Одиночная осечка сети — обычное дело, и пугать ею незачем: на экране
/// останется прошлая погода, которой четверть часа от роду.
const FAILURES_BEFORE_ALARM: u32 = 2;

/// Что показывает карточка.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum View {
    /// Город не выбран — в сеть не ходим вовсе.
    NoPlace,
    /// Город есть, погоды ещё нет.
    Loading { place: String },
    Ready {
        place: String,
        report: Report,
        /// Ответа нет давно: на экране прошлая погода.
        stale: bool,
    },
    Failed { place: String, reason: String },
}

/// Последняя погода и клиент для запросов.
///
/// Клиент лежит рядом с мьютексом, а не внутри: держать `MutexGuard` через
/// `await` нельзя, а команда поиска — асинхронная.
pub struct Weather {
    view: Mutex<View>,
    client: reqwest::Client,
    /// Будильник опроса: город сменили или нажали обновление.
    ///
    /// Нужен с тех пор, как город выбирают прямо в карточке: нажатие по
    /// найденному городу меняет настройки, а карточка до следующего тика
    /// оставалась бы полем поиска — до пяти секунд, будто нажатие не дошло.
    nudge: Arc<Notify>,
    /// Кнопку обновления нажали: идём в сеть, не дожидаясь шага.
    ///
    /// Отдельно от будильника: тот только будит спящего, а это — причина идти
    /// в сеть. Разбуженный сменой города опрос обязан увидеть новый город, а не
    /// сходить за прежним.
    forced: AtomicBool,
}

impl Default for Weather {
    fn default() -> Self {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            // Клиент не собрался — берём тот, что по умолчанию: без таймаута,
            // но живой. Отказаться от виджета целиком было бы хуже.
            .unwrap_or_default();

        Self {
            view: Mutex::new(View::NoPlace),
            client,
            nudge: Arc::new(Notify::new()),
            forced: AtomicBool::new(false),
        }
    }
}

impl Weather {
    fn locked(&self) -> MutexGuard<'_, View> {
        // Отравленный мьютекс не повод падать: данные внутри осмысленные
        self.view
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Будит опрос: город сменили.
///
/// Разбудить некого до готовности приложения — тогда молчим: опрос всё равно
/// прочитает город при первом же заходе.
pub fn nudge(app: &AppHandle) {
    if let Some(state) = app.try_state::<Weather>() {
        // `notify_one` держит разрешение, если ждущего сейчас нет: разбуженный
        // посреди запроса опрос увидит его сразу после, а не проспит
        state.nudge.notify_one();
    }
}

#[tauri::command]
pub fn weather_state(state: State<'_, Weather>) -> View {
    state.locked().clone()
}

/// Кнопка обновления в углу карточки.
///
/// Флаг, а не поход в сеть прямо здесь: сетью занят опрос, и второй охотник за
/// тем же ответом означал бы два запроса внахлёст и гонку за то, чей снимок
/// ляжет в карточку.
#[tauri::command]
pub fn weather_refresh(state: State<'_, Weather>) {
    state.forced.store(true, Ordering::Relaxed);
    state.nudge.notify_one();
}

/// Поиск города для раздела настроек.
///
/// Команда асинхронная: запрос уходит в рантайм, а не занимает поток, которым
/// WebView2 принимает IPC. Прецедент — `spotify_connect`, ждущий браузер.
#[tauri::command]
pub async fn weather_search(state: State<'_, Weather>, query: String) -> Result<Vec<api::Found>, String> {
    // Клонируем до первого `await`: клиент внутри — это `Arc`, копия дешёвая
    let client = state.client.clone();
    api::search(&client, &query).await
}

/// Выбранный город из настроек.
fn place(app: &AppHandle) -> Option<Place> {
    app.try_state::<SettingsState>()
        .and_then(|state| state.get().weather)
}

/// Смотрит ли кто-нибудь на виджет.
///
/// Правило звука и процессов, а не буфера обмена. У буфера правило обратное
/// потому, что он копит историю ради возврата к ней позже; погода истории не
/// копит — спрятанное окно ничего не теряет, а показанное догоняет одним
/// запросом.
///
/// Смотрим на видимость, а не на наличие окна: рубильник «спрятать все» окна не
/// уничтожает, и по наличию опрос шёл бы под спрятанной карточкой.
fn watched(app: &AppHandle) -> bool {
    app.get_webview_window(WIDGET)
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false)
}

/// Что известно к началу захода.
#[derive(Debug, Clone, Copy)]
struct Moment {
    has_place: bool,
    watched: bool,
    /// Сколько прошло с прошлого похода в сеть. `None` — не ходили ни разу.
    since: Option<Duration>,
    /// Прошлый поход кончился отказом.
    failing: bool,
    /// Нажали кнопку обновления.
    forced: bool,
}

/// Пора ли спрашивать погоду.
fn due(moment: &Moment) -> bool {
    if !moment.has_place {
        return false;
    }

    // Нажатие бьёт и шаг, и видимость: спрятать окно сразу после нажатия
    // человек, конечно, может, но просил он именно свежую погоду
    if moment.forced {
        return true;
    }

    if !moment.watched {
        return false;
    }

    let Some(since) = moment.since else {
        // Окно только что показали — ждать четыре часа незачем
        return true;
    };

    since >= if moment.failing { RETRY } else { STEP }
}

/// Что показывать, когда ответ не пришёл.
///
/// Прошлый снимок не выбрасываем: погода четвертьчасовой давности — не враньё
/// того же рода, что вчерашний замер спидтеста. Там замер — событие, нажатие
/// кнопки, а тут состояние, которое так быстро не меняется.
fn after_failure(previous: &View, place: String, reason: String, failures: u32) -> View {
    let alarmed = failures >= FAILURES_BEFORE_ALARM;

    // Снимок того же города — держим его на экране, пометив при затянувшемся
    // молчании. Чужой город тут означает, что его только что сменили.
    if let View::Ready { place: shown, report, .. } = previous {
        if *shown == place {
            return View::Ready {
                place,
                report: report.clone(),
                stale: alarmed,
            };
        }
    }

    if alarmed {
        View::Failed { place, reason }
    } else {
        // Первая осечка без снимка: молчим, карточка остаётся в ожидании
        View::Loading { place }
    }
}

/// Расходится ли снимок по окнам, даже если он совпал с прошлым.
///
/// Нужно ради кнопки обновления: значок крутится, пока не придёт ответ, и снять
/// его карточке нечем, если погода за четыре часа не изменилась ни на градус.
fn publish_always(app: &AppHandle, next: View) {
    send(app, next, true);
}

fn publish(app: &AppHandle, next: View) {
    send(app, next, false);
}

fn send(app: &AppHandle, next: View, always: bool) {
    let state = app.state::<Weather>();
    {
        let mut stored = state.locked();
        // Ничего не поменялось — молчим: между одинаковыми ответами окну нечего
        // показывать заново
        if *stored == next && !always {
            return;
        }
        *stored = next.clone();
    }

    // Неудачу рассылки глотаем: окно могли спрятать прямо в этот момент
    let _ = app.emit(WEATHER_EVENT, next);
}

/// Запускает опрос. Живёт до конца работы приложения.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move { work(app).await });
}

/// Пауза между заходами: шаг проверки или будильник, что случится раньше.
///
/// Через `timeout`, а не `select!`: макрос живёт в возможности `macros` у
/// tokio, которой в зависимостях нет, — включать её ради одной строки незачем.
async fn rest(nudge: &Notify) {
    // Истёкший срок — обычный случай, а не отказ: значит будильник молчал
    let _ = tokio::time::timeout(TICK, nudge.notified()).await;
}

async fn work(app: AppHandle) {
    let client = app.state::<Weather>().client.clone();
    let nudge = app.state::<Weather>().nudge.clone();

    // Город, для которого лежит нынешний снимок
    let mut known: Option<Place> = None;
    // Когда последний раз ходили в сеть
    let mut last: Option<Instant> = None;
    let mut failures: u32 = 0;

    loop {
        let wanted = place(&app);

        // Город сменился — прошлый снимок больше не про него. Делаем это и при
        // спрятанном окне: сеть тут не нужна, а показать при следующем открытии
        // погоду прежнего города было бы враньём.
        if wanted != known {
            known = wanted;
            last = None;
            failures = 0;

            publish(
                &app,
                match &known {
                    Some(place) => View::Loading {
                        place: place.name.clone(),
                    },
                    None => View::NoPlace,
                },
            );
        }

        // Флаг снимаем здесь, а не после похода: нажатие во время запроса
        // должно дать ещё один заход, а не потеряться в нём
        let forced = app
            .state::<Weather>()
            .forced
            .swap(false, Ordering::Relaxed);

        let moment = Moment {
            has_place: known.is_some(),
            watched: watched(&app),
            since: last.map(|at| at.elapsed()),
            failing: failures > 0,
            forced,
        };

        if !due(&moment) {
            rest(&nudge).await;
            continue;
        }

        // Город тут есть заведомо: без него `due` не пропускает
        let Some(asked) = known.clone() else {
            rest(&nudge).await;
            continue;
        };

        last = Some(Instant::now());

        let outcome = api::fetch(&client, &asked).await;
        // Город сменили или убрали, пока шёл запрос: ответ про прежний, и
        // опубликованный, он показал бы чужую погоду. Новый город снимок
        // сбросит и заберёт сразу — заход начнётся с проверки наверху
        if place(&app) != known {
            continue;
        }

        match outcome {
            Ok(report) => {
                failures = 0;
                let next = View::Ready {
                    place: asked.name,
                    report,
                    stale: false,
                };

                // Нажатие обязано ответить окну даже тем же самым снимком:
                // иначе значок обновления крутился бы до самого таймаута
                if forced {
                    publish_always(&app, next);
                } else {
                    publish(&app, next);
                }
            }
            Err(reason) => {
                failures = failures.saturating_add(1);
                let previous = app.state::<Weather>().locked().clone();
                let next = after_failure(&previous, asked.name, reason, failures);

                if forced {
                    publish_always(&app, next);
                } else {
                    publish(&app, next);
                }
            }
        }

        rest(&nudge).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::Sky;

    fn moment(watched: bool, since: Option<Duration>, failing: bool) -> Moment {
        Moment {
            has_place: true,
            watched,
            since,
            failing,
            forced: false,
        }
    }

    fn report() -> Report {
        Report {
            temperature: Some(21.0),
            sky: Sky::Clear,
            day: true,
            precipitation: Some(10),
            wind: Some(3.4),
            hours: Vec::new(),
            days: Vec::new(),
        }
    }

    fn ready(place: &str, stale: bool) -> View {
        View::Ready {
            place: place.to_owned(),
            report: report(),
            stale,
        }
    }

    /// Спрятанное окно в чужой бесплатный сервис не ходит, и города без выбора
    /// тоже: правило звука и процессов.
    #[test]
    fn a_hidden_window_is_never_asked_about() {
        // Ни разу не ходили, но смотреть некому
        assert!(!due(&moment(false, None, false)));
        // И даже когда шаг давно вышел
        assert!(!due(&moment(false, Some(STEP * 2), false)));

        assert!(!due(&Moment {
            has_place: false,
            watched: true,
            since: None,
            failing: false,
            forced: false,
        }));
    }

    /// Кнопка обновления бьёт и шаг, и видимость окна — но не отсутствие
    /// города: без него в сеть не ходят вовсе, и нажимать там нечего.
    #[test]
    fn the_button_beats_the_step() {
        let just_asked = Moment {
            has_place: true,
            watched: true,
            since: Some(Duration::from_secs(1)),
            failing: false,
            forced: true,
        };
        assert!(due(&just_asked));

        assert!(due(&Moment {
            watched: false,
            ..just_asked
        }));

        assert!(!due(&Moment {
            has_place: false,
            ..just_asked
        }));
    }

    /// Открытое окно спрашивает сразу, а дальше держит шаг.
    #[test]
    fn the_step_is_kept_between_answers() {
        assert!(due(&moment(true, None, false)), "первый заход не ждёт");
        assert!(!due(&moment(true, Some(STEP / 2), false)));
        assert!(due(&moment(true, Some(STEP), false)));
    }

    /// После отказа ждём минуту, а не четверть часа: сеть возвращается быстрее.
    #[test]
    fn a_failure_is_retried_sooner_than_the_step() {
        assert!(!due(&moment(true, Some(RETRY / 2), true)));
        assert!(due(&moment(true, Some(RETRY), true)));
        // Без отказа та же минута ничего не значит
        assert!(!due(&moment(true, Some(RETRY), false)));
    }

    /// Потерянный ответ оставляет на экране прошлую погоду, а не прочерки.
    #[test]
    fn a_lost_answer_keeps_the_last_one() {
        let shown = ready("Грац", false);

        // Первая осечка проходит незамеченной
        let once = after_failure(&shown, "Грац".to_owned(), "нет связи".to_owned(), 1);
        assert_eq!(once, shown);

        // Затянувшееся молчание помечает снимок, но не стирает его
        let twice = after_failure(&shown, "Грац".to_owned(), "нет связи".to_owned(), 2);
        assert_eq!(twice, ready("Грац", true));
    }

    /// Снимка нет — первая осечка молчит, а набравшиеся становятся отказом.
    #[test]
    fn without_a_snapshot_the_alarm_takes_two_failures() {
        let waiting = View::Loading {
            place: "Грац".to_owned(),
        };

        assert_eq!(
            after_failure(&waiting, "Грац".to_owned(), "нет связи".to_owned(), 1),
            waiting
        );

        assert_eq!(
            after_failure(&waiting, "Грац".to_owned(), "нет связи".to_owned(), 2),
            View::Failed {
                place: "Грац".to_owned(),
                reason: "нет связи".to_owned(),
            }
        );
    }

    /// Снимок чужого города на экране не остаётся: город только что сменили.
    #[test]
    fn a_snapshot_of_another_city_is_never_kept() {
        let shown = ready("Грац", false);
        let next = after_failure(&shown, "Краков".to_owned(), "нет связи".to_owned(), 1);

        assert_eq!(
            next,
            View::Loading {
                place: "Краков".to_owned()
            }
        );
    }

    /// Разметка уходит на фронт как есть.
    #[test]
    fn view_arrives_in_camel_case() {
        let json = serde_json::to_string(&View::NoPlace).expect("пишется");
        assert_eq!(json, r#"{"status":"noPlace"}"#);

        let json = serde_json::to_string(&ready("Грац", true)).expect("пишется");
        assert!(json.contains(r#""status":"ready""#), "{json}");
        assert!(json.contains(r#""stale":true"#), "{json}");
        assert!(json.contains(r#""place":"Грац""#), "{json}");
    }

    /// Метка окна обязана совпадать с реестром: разойдись они, виджет замолчал
    /// бы навсегда и молча — опрос считал бы, что на него никто не смотрит.
    #[test]
    fn window_label_matches_the_registry() {
        assert!(crate::widgets::find(WIDGET).is_some(), "виджета нет в реестре");
    }

    /// Пустой запрос до сети не доходит вовсе.
    ///
    /// Клиент тут настоящий, но походом он и не пахнет: проверка стоит раньше.
    #[tokio::test]
    async fn an_empty_query_never_leaves_the_house() {
        let client = reqwest::Client::new();

        assert!(api::search(&client, "").await.expect("не отказ").is_empty());
        assert!(api::search(&client, "  ").await.expect("не отказ").is_empty());
        assert!(api::search(&client, "М").await.expect("не отказ").is_empty());
    }

    /// Живой круг: спрашиваем настоящую погоду и настоящий город.
    ///
    /// Запуск: `cargo test --manifest-path src-tauri/Cargo.toml -- --ignored`
    #[tokio::test]
    #[ignore = "ходит в Open-Meteo"]
    async fn reads_real_weather() {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("клиент собрался");

        let graz = Place {
            name: "Грац".to_owned(),
            region: "Австрия".to_owned(),
            latitude: 47.0707,
            longitude: 15.4395,
        };

        let report = api::fetch(&client, &graz).await.expect("погода пришла");

        let temperature = report.temperature.expect("температура на месте");

        assert!(
            (-60.0..=60.0).contains(&temperature),
            "температура вышла невероятной: {temperature}"
        );

        let wind = report.wind.expect("ветра сейчас нет");
        // Метры в секунду, а не километры в час: сотня тут означала бы, что
        // `wind_speed_unit` из запроса потерялся
        assert!((0.0..60.0).contains(&wind), "ветер в чужих единицах: {wind}");

        assert_eq!(report.days.len(), api::DAYS, "пять дней ряда не пришли");
        for day in &report.days {
            assert_eq!(day.date.len(), 10, "дата дня не дата: {}", day.date);
            assert!(day.day.max.is_some(), "у дня {} нет максимума", day.date);
            assert_eq!(day.windows.len(), 5, "у дня {} не пять окон", day.date);
        }

        assert_eq!(report.hours.len(), 6, "нынешний день по часам не пришёл");
        for hour in &report.hours {
            assert_eq!(hour.at.len(), 5, "время часа не подпись: {}", hour.at);

            let wind = hour.wind.expect("ветра у часа нет");
            // Метры в секунду, а не километры в час: сотня тут означала бы,
            // что `wind_speed_unit` из запроса потерялся
            assert!((0.0..60.0).contains(&wind), "ветер в чужих единицах: {wind}");
        }
    }

    /// Живой поиск: город находится по имени, и координаты у него те самые.
    #[tokio::test]
    #[ignore = "ходит в геокодер Open-Meteo"]
    async fn finds_a_city_by_name() {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("клиент собрался");

        let hits = api::search(&client, "Москва").await.expect("поиск прошёл");
        let first = hits.first().expect("хоть одна находка");

        assert!((first.latitude - 55.75).abs() < 1.0, "нашлось не то: {first:?}");
        assert!(!first.region.is_empty(), "город без уточнения не различить");
    }
}
