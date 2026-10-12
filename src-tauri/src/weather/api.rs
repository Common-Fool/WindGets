//! Обращение к Open-Meteo: прогноз и поиск города.
//!
//! Ключа и учётной записи сервис не требует вовсе — этим он и выбран.
//!
//! Разбор кода погоды живёт тут, а не на фронте, и это отступление от того, как
//! устроены пороги пинга и цвет накопителей: там решает фронт. Причина — тесты.
//! Таблица из тридцати кодов WMO ровно то место, где опечатка живёт годами и
//! молча красит снег в синий, а у фронта в проекте проверок нет вовсе.
//! Прецедент есть: звук отдаёт тип устройства, а значок по нему выбирает фронт.
//! Цвета Rust по-прежнему не называет — отдаёт погоду, а цвет ей назначает
//! карточка.

use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::settings::Place;

const FORECAST: &str = "https://api.open-meteo.com/v1/forecast";
const GEOCODING: &str = "https://geocoding-api.open-meteo.com/v1/search";

/// Сколько городов показываем в списке находок.
///
/// Список из четырёх строк уже приходится листать, а одноимённых городов
/// бывает много — восемь закрывают и то и другое.
const HITS: usize = 8;

/// Короче этого в сеть не ходим: по одной букве геокодер вернёт что угодно.
const MIN_QUERY: usize = 2;

/// Сколько часов показывает раскрытый нынешний день.
///
/// Шесть значений на двенадцать часов вперёд: полсуток — это «когда сегодня
/// собраться», а двенадцать строк подряд в карточку не ставят.
const HOURS: usize = 6;

/// Шаг между ними в часах.
///
/// Два, а не один: погода за час меняется редко, и почасовой список отличался
/// бы от соседней строки одной десятой градуса.
const HOUR_STEP: usize = 2;

/// Часы, которыми показан выбранный день.
///
/// Шаг в четыре часа, а не почасовой список: чужой день читают на «утром дождь,
/// вечером прояснится», и двадцати четырёх строк для этого не нужно. Ночь
/// начиная с четырёх утра, дальше утро, полдень, вечер и ночь.
///
/// Сравниваются они со строкой времени как есть, поэтому пишутся ровно в том
/// виде, в каком приходят от сервиса.
const WINDOWS: [&str; 5] = ["04:00", "08:00", "12:00", "16:00", "20:00"];

/// Сколько дней стоит в ряду под основным блоком.
///
/// Нынешнего дня в ряду нет вовсе — он и есть основной блок, — поэтому у
/// сервиса просим на один день больше.
pub const DAYS: usize = 5;

/// Столько суток просим у сервиса: пять в ряду плюс нынешний.
///
/// Держим строкой, а не форматируем на каждом заходе: `Url::parse_with_params`
/// берёт `&str`, а разъехаться числу и строке не даёт тест.
const DAYS_ASKED: &str = "6";

/// Погода за окном — то, чем карточка красится и выбирает значок.
///
/// Не код WMO: кодов тридцать, а значков и цветов девять, и раскладывать одно
/// в другое дважды (в значке и в акценте) значит однажды разложить по-разному.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Sky {
    Clear,
    PartlyCloudy,
    Overcast,
    Fog,
    Drizzle,
    Rain,
    Sleet,
    Snow,
    Thunder,
}

/// Сутки прогноза: макс, мин и вероятность осадков.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Day {
    pub max: Option<f64>,
    pub min: Option<f64>,
    /// Проценты. `None` — сервис не посчитал, и в карточке будет прочерк.
    pub precipitation: Option<u8>,
}

/// Час прогноза — строка раскрытой панели.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hour {
    /// Время в поясе города, уже подписью: «15:00».
    ///
    /// Режем здесь, а не на фронте: формат задаёт Open-Meteo, и разбирать
    /// чужую строку в двух местах значит однажды разобрать по-разному.
    pub at: String,
    pub sky: Sky,
    pub temperature: Option<f64>,
    /// Вероятность осадков в процентах.
    pub precipitation: Option<u8>,
    /// Скорость ветра в метрах в секунду.
    pub wind: Option<f64>,
    /// День или ночь: у ясного неба от этого зависит значок.
    pub day: bool,
}

/// День в ряду под основным блоком.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ahead {
    /// Дата в поясе города, как пришла: «2026-09-03».
    ///
    /// Имя дня делает фронт, а не мы: «Ср» — это разговор с хозяином машины, и
    /// живёт он там же, где названия погоды. Резать тут нечего — дату сервис
    /// отдаёт готовой.
    pub date: String,
    pub sky: Sky,
    /// Макс, мин и вероятность осадков за сутки.
    pub day: Day,
    /// Тот же день по часам: 04:00, 08:00, 12:00, 16:00 и 20:00.
    ///
    /// Едет вместе с днём, а не спрашивается по нажатию: почасовку на все дни
    /// сервис прислал одним ответом, и второй поход в сеть ради строки, которую
    /// раскрыли мышью, был бы походом за уже полученным.
    pub windows: Vec<Hour>,
}

/// Всё, что показывает карточка.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// Градусы Цельсия сейчас.
    pub temperature: Option<f64>,
    pub sky: Sky,
    /// День на дворе или ночь: у ясного неба от этого зависит значок.
    pub day: bool,
    /// Вероятность осадков в ближайший час, проценты.
    ///
    /// Из почасовки, а не из `current`: вероятности среди текущих величин у
    /// сервиса нет вовсе, есть выпавшие миллиметры. «0 мм» в сухой день — это
    /// строка, которая никогда ничего не говорит.
    pub precipitation: Option<u8>,
    /// Ветер сейчас, метры в секунду.
    pub wind: Option<f64>,
    /// Нынешний день по часам: шесть значений шагом в два часа.
    pub hours: Vec<Hour>,
    /// Пять дней, начиная с завтрашнего.
    pub days: Vec<Ahead>,
}

/// Найденный город — строка списка в настройках.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    pub name: String,
    /// «Штирия, Австрия» — уточнение под именем.
    pub region: String,
    pub latitude: f64,
    pub longitude: f64,
}

/// Погода по коду WMO.
///
/// Функция тотальная: незнакомый код становится пасмурным. Своего значка «не
/// знаю» у карточки нет, а облако — самое безобидное враньё из возможных.
pub fn sky(code: u16) -> Sky {
    match code {
        0 => Sky::Clear,
        1 | 2 => Sky::PartlyCloudy,
        3 => Sky::Overcast,
        45 | 48 => Sky::Fog,
        51 | 53 | 55 => Sky::Drizzle,
        61 | 63 | 65 | 80 | 81 | 82 => Sky::Rain,
        // Переохлаждённые морось и дождь — это гололёд, и он ближе к снегу, чем
        // к дождю: выходя из дома, знать надо именно про лёд
        56 | 57 | 66 | 67 => Sky::Sleet,
        71 | 73 | 75 | 77 | 85 | 86 => Sky::Snow,
        95 | 96 | 99 => Sky::Thunder,
        _ => Sky::Overcast,
    }
}

/// Ответ прогноза.
///
/// Всё насквозь необязательное, и это не перестраховка: вероятность осадков
/// приходит `null` на части моделей, а `daily` пропадает целиком, если сервис
/// ответил отказом в теле с кодом 200. Пустое место обязано стать прочерком, а
/// не нулём.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Answer {
    current: Option<CurrentBlock>,
    daily: Option<DailyBlock>,
    hourly: Option<HourlyBlock>,
    /// Отказ приходит телом с обычным кодом ответа.
    reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CurrentBlock {
    /// «2026-09-01T14:30» в поясе города. По нему находится, с какого часа
    /// начинать почасовой прогноз.
    time: Option<String>,
    temperature_2m: Option<f64>,
    weather_code: Option<u16>,
    is_day: Option<u8>,
    /// Метры в секунду — единицу задаёт `wind_speed_unit` в запросе.
    wind_speed_10m: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct HourlyBlock {
    time: Vec<String>,
    temperature_2m: Vec<Option<f64>>,
    weather_code: Vec<Option<u16>>,
    precipitation_probability: Vec<Option<u8>>,
    wind_speed_10m: Vec<Option<f64>>,
    is_day: Vec<Option<u8>>,
}

impl HourlyBlock {
    /// Нынешний день по часам: шесть значений шагом в два часа.
    ///
    /// Строки времени идут в поясе города и в формате ISO, а он сортируется
    /// как текст — сравнивать их можно прямо строками, разбор даты не нужен.
    ///
    /// Начинается со **следующего** часа, а не с нынешнего: погоду этого часа
    /// карточка уже показывает крупной цифрой, и повторять её первой строкой
    /// значит потерять два часа прогноза.
    fn from(&self, now: Option<&str>) -> Vec<Hour> {
        let start = match now {
            // Времени нет — берём с начала: сутки вперёд всё равно лучше, чем
            // пустая панель
            None => 0,
            Some(now) => self
                .time
                .iter()
                .position(|at| at.as_str() > now)
                .unwrap_or(self.time.len()),
        };

        self.time
            .iter()
            .enumerate()
            .skip(start)
            .step_by(HOUR_STEP)
            .take(HOURS)
            .map(|(at, time)| self.hour(at, time))
            .collect()
    }

    /// Выбранные сутки окнами по четыре часа.
    ///
    /// День приходит датой из `daily`, а не считается от нынешней: сутки в
    /// ответе идут по порядку, и календарь с високосными годами и концами
    /// месяцев считать незачем. И не по часам машины: пояс тут городской
    /// (`timezone=auto`), и день у города за океаном начинается не тогда же,
    /// когда у нас.
    fn windows(&self, date: &str) -> Vec<Hour> {
        self.time
            .iter()
            .enumerate()
            .filter(|(_, at)| {
                at.get(..10) == Some(date)
                    && at.get(11..16).is_some_and(|hour| WINDOWS.contains(&hour))
            })
            .map(|(at, time)| self.hour(at, time))
            .collect()
    }

    /// Час, который идёт сейчас, — последний из тех, что уже наступили.
    ///
    /// Нужен ровно ради вероятности осадков: среди текущих величин сервиса её
    /// нет, а в почасовке она есть.
    fn current(&self, now: Option<&str>) -> Option<Hour> {
        let at = match now {
            None => 0,
            Some(now) => self
                .time
                .iter()
                .rposition(|at| at.as_str() <= now)
                // Прогноз начинается позже нынешнего часа — берём первый: он
                // ближе всех к правде из того, что есть
                .unwrap_or(0),
        };

        self.time.get(at).map(|time| self.hour(at, time))
    }

    /// Час по его месту в ответе.
    ///
    /// Один на обе зоны: собирайся строка в двух местах, ветер однажды достался
    /// бы только одной из них.
    fn hour(&self, at: usize, time: &str) -> Hour {
        Hour {
            // «2026-09-01T15:00» → «15:00». Строка чужая, поэтому режем
            // осторожно: не тот формат — показываем как есть
            at: time.get(11..16).unwrap_or(time).to_owned(),
            sky: sky(self.weather_code.get(at).copied().flatten().unwrap_or(3)),
            temperature: self.temperature_2m.get(at).copied().flatten(),
            precipitation: self.precipitation_probability.get(at).copied().flatten(),
            wind: self.wind_speed_10m.get(at).copied().flatten(),
            day: self.is_day.get(at).copied().flatten().unwrap_or(1) != 0,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct DailyBlock {
    /// «2026-09-03» в поясе города, по одной на сутки.
    time: Vec<String>,
    weather_code: Vec<Option<u16>>,
    temperature_2m_max: Vec<Option<f64>>,
    temperature_2m_min: Vec<Option<f64>>,
    precipitation_probability_max: Vec<Option<u8>>,
}

impl DailyBlock {
    /// Пять дней, начиная с завтрашнего.
    ///
    /// Нынешний день пропускаем: он и есть основной блок карточки, и повторять
    /// его первой клеткой ряда значит отдать пятую часть ряда под то, что стоит
    /// строкой выше крупной цифрой.
    ///
    /// Список ведут даты, а не числа температур: пустой `daily` обязан дать
    /// пустой ряд, а не пять клеток с прочерками.
    fn ahead(&self, hourly: &HourlyBlock) -> Vec<Ahead> {
        self.time
            .iter()
            .enumerate()
            .skip(1)
            .take(DAYS)
            .map(|(at, date)| Ahead {
                date: date.clone(),
                sky: sky(self.weather_code.get(at).copied().flatten().unwrap_or(3)),
                day: self.day(at),
                windows: hourly.windows(date),
            })
            .collect()
    }

    /// Сутки по их месту в ответе. За краем — пустые: у прогноза на один день
    /// завтра просто нет, и это не повод падать.
    fn day(&self, at: usize) -> Day {
        Day {
            max: self.temperature_2m_max.get(at).copied().flatten(),
            min: self.temperature_2m_min.get(at).copied().flatten(),
            precipitation: self
                .precipitation_probability_max
                .get(at)
                .copied()
                .flatten(),
        }
    }
}

/// Раскладывает ответ в то, что показывает карточка.
pub fn read(answer: Answer) -> Result<Report, String> {
    if let Some(reason) = answer.reason {
        // Английский текст сервиса в карточку не пускаем — ему там жить одной
        // строкой в окне шириной 440 точек
        eprintln!("[погода] сервис отказал: {reason}");
        return Err(tr!("weather.error.refused"));
    }

    let current = answer.current.unwrap_or_default();
    let daily = answer.daily.unwrap_or_default();
    let hourly = answer.hourly.unwrap_or_default();

    Ok(Report {
        temperature: current.temperature_2m,
        sky: sky(current.weather_code.unwrap_or(3)),
        // Поля нет — считаем, что день: ночным значком среди бела дня карточка
        // выглядела бы сломанной
        day: current.is_day.unwrap_or(1) != 0,
        precipitation: hourly
            .current(current.time.as_deref())
            .and_then(|hour| hour.precipitation),
        wind: current.wind_speed_10m,
        hours: hourly.from(current.time.as_deref()),
        days: daily.ahead(&hourly),
    })
}

/// Адрес прогноза.
///
/// Собираем разбором, а не строкой: в значениях запятые, а в имени города
/// кириллица — кодировать это руками значит однажды забыть.
pub fn forecast_url(place: &Place) -> String {
    let url = Url::parse_with_params(
        FORECAST,
        &[
            ("latitude", place.latitude.to_string().as_str()),
            ("longitude", place.longitude.to_string().as_str()),
            // Ветер тут же, а не из почасовки: у основного блока он стоит
            // рядом с нынешней температурой, и час назад — это уже не «сейчас»
            (
                "current",
                "temperature_2m,weather_code,is_day,wind_speed_10m",
            ),
            // Код погоды у дней нужен ряду: клетка дня различается значком так
            // же, как строка часа
            (
                "daily",
                "weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max",
            ),
            // Ближайшие часы для раскрытой панели. Едут тем же запросом:
            // отдельный поход в сеть ради панели, которую открывают раз в день,
            // ничем не оправдан
            (
                "hourly",
                "temperature_2m,weather_code,precipitation_probability,wind_speed_10m,is_day",
            ),
            // ВАЖНО: без этого ветер приходит в километрах в час, и подпись
            // «м/с» над ними была бы враньём, которое ничем себя не выдаёт
            ("wind_speed_unit", "ms"),
            // Шесть суток: пять клеток ряда плюс нынешний день, которого в
            // ряду нет. Почасовка приезжает на все шесть — из неё режутся и
            // ближайшие двенадцать часов, и окна каждого дня ряда
            ("forecast_days", DAYS_ASKED),
            // ВАЖНО: без этого сутки в прогнозе нарезаны по UTC, и «сегодня» у
            // города в другом поясе съезжает. Пояс берём города, а не машины:
            // город бывает чужим
            ("timezone", "auto"),
        ],
    );

    match url {
        Ok(url) => url.to_string(),
        // Адрес — константа, промахнуться тут можно только правкой кода
        Err(error) => unreachable!("адрес прогноза не собрался: {error}"),
    }
}

/// Адрес поиска города.
///
/// Имена городов и стран приходят на языке интерфейса: сервис переводит их
/// сам, а нет перевода — отдаёт английское или местное имя. Выбранный город
/// сохраняется с тем именем, под которым его нашли, и на смене языка не
/// переименовывается.
pub fn search_url(query: &str) -> String {
    let count = HITS.to_string();
    let url = Url::parse_with_params(
        GEOCODING,
        &[
            ("name", query),
            ("count", count.as_str()),
            ("language", crate::i18n::current()),
            ("format", "json"),
        ],
    );

    match url {
        Ok(url) => url.to_string(),
        Err(error) => unreachable!("адрес поиска не собрался: {error}"),
    }
}

/// Ответ геокодера.
///
/// ВАЖНО: когда ничего не нашлось, ключа `results` в ответе нет вовсе. Это не
/// отказ, а пустой список — отсюда `default` вместо `Option`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Hits {
    results: Vec<Hit>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Hit {
    name: String,
    latitude: f64,
    longitude: f64,
    country: Option<String>,
    /// Область или воеводство.
    admin1: Option<String>,
}

/// Уточнение под именем города.
///
/// Область добавляем, только когда она отличается от самого имени: у городов
/// вроде Москвы иначе выходит «Москва · Москва, Россия».
fn label(hit: &Hit) -> String {
    let mut parts: Vec<&str> = Vec::new();

    if let Some(region) = hit.admin1.as_deref() {
        if !region.is_empty() && region != hit.name {
            parts.push(region);
        }
    }

    if let Some(country) = hit.country.as_deref() {
        if !country.is_empty() {
            parts.push(country);
        }
    }

    parts.join(", ")
}

pub fn found(hits: Hits) -> Vec<Found> {
    hits.results
        .iter()
        // Город без имени в списке не выбрать: строка вышла бы пустой
        .filter(|hit| !hit.name.trim().is_empty())
        .map(|hit| Found {
            name: hit.name.clone(),
            region: label(hit),
            latitude: hit.latitude,
            longitude: hit.longitude,
        })
        .collect()
}

/// Короткая причина отказа на выбранном языке.
///
/// Тексту жить одной строкой в углу карточки шириной 440 точек, и переносом он
/// сдвинул бы всё, что выше.
fn describe(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        tr!("weather.error.timeout")
    } else if error.is_connect() {
        tr!("weather.error.offline")
    } else if error.is_decode() {
        tr!("weather.error.garbled")
    } else {
        tr!("weather.error.noAnswer")
    }
}

pub async fn fetch(client: &reqwest::Client, place: &Place) -> Result<Report, String> {
    let answer: Answer = client
        .get(forecast_url(place))
        .send()
        .await
        .map_err(|error| describe(&error))?
        .json()
        .await
        .map_err(|error| describe(&error))?;

    read(answer)
}

/// Ищет город по названию.
///
/// Пустой запрос до сети не доходит вовсе: он всё равно ничего не значит, а
/// лишний поход в чужой бесплатный сервис ничем не оправдан.
pub async fn search(client: &reqwest::Client, query: &str) -> Result<Vec<Found>, String> {
    let query = query.trim();
    if query.chars().count() < MIN_QUERY {
        return Ok(Vec::new());
    }

    let hits: Hits = client
        .get(search_url(query))
        .send()
        .await
        .map_err(|error| describe(&error))?
        .json()
        .await
        .map_err(|error| describe(&error))?;

    Ok(found(hits))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(url: &str) -> Vec<(String, String)> {
        Url::parse(url)
            .expect("адрес разбирается")
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect()
    }

    fn value(url: &str, key: &str) -> String {
        params(url)
            .into_iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
            .unwrap_or_else(|| panic!("в адресе нет «{key}»: {url}"))
    }

    fn graz() -> Place {
        Place {
            name: "Грац".to_owned(),
            region: "Австрия".to_owned(),
            latitude: 47.0707,
            longitude: 15.4395,
        }
    }

    /// Спрашиваем ровно то, что показывает карточка, — ни полем больше.
    ///
    /// Сверяем парами, а не подстрокой: запятые в значениях уезжают в `%2C`, и
    /// поиск подстроки прошёл бы мимо.
    #[test]
    fn forecast_url_asks_for_exactly_what_the_card_shows() {
        let url = forecast_url(&graz());

        assert_eq!(value(&url, "latitude"), "47.0707");
        assert_eq!(value(&url, "longitude"), "15.4395");
        assert_eq!(
            value(&url, "current"),
            "temperature_2m,weather_code,is_day,wind_speed_10m"
        );
        assert_eq!(
            value(&url, "daily"),
            "weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max"
        );
        assert_eq!(
            value(&url, "hourly"),
            "temperature_2m,weather_code,precipitation_probability,wind_speed_10m,is_day"
        );
        assert_eq!(value(&url, "timezone"), "auto");
    }

    /// Суток просим на одни больше, чем клеток в ряду: нынешний день в ряду не
    /// стоит, но его почасовка нужна основному блоку.
    #[test]
    fn one_more_day_is_asked_for_than_the_row_shows() {
        assert_eq!(value(&forecast_url(&graz()), "forecast_days"), DAYS_ASKED);
        assert_eq!(DAYS_ASKED.parse::<usize>(), Ok(DAYS + 1));
    }

    /// Без этой пары ветер приходит в километрах в час, а подпись в панели
    /// стоит «м/с»: враньё, которое ничем себя не выдаёт.
    #[test]
    fn wind_is_asked_for_in_metres_per_second() {
        assert_eq!(value(&forecast_url(&graz()), "wind_speed_unit"), "ms");
    }

    /// Пробел и кириллица в имени города обязаны пережить сборку адреса.
    #[test]
    fn search_url_survives_a_space_and_cyrillic() {
        let url = search_url("Нижний Новгород");

        assert_eq!(value(&url, "name"), "Нижний Новгород");
        assert_eq!(value(&url, "count"), "8");
        assert_eq!(value(&url, "language"), "ru");
    }

    const RECORDED: &str = r#"{
        "current": {
            "time": "2026-09-01T14:30",
            "temperature_2m": 23.7, "weather_code": 61, "is_day": 0,
            "wind_speed_10m": 4.2
        },
        "daily": {
            "time": ["2026-09-01", "2026-09-02", "2026-09-03",
                     "2026-09-04", "2026-09-05", "2026-09-06"],
            "weather_code": [61, 3, 0, 71, 95, 45],
            "temperature_2m_max": [24.1, 21.8, 25.0, -1.0, 19.4, 17.7],
            "temperature_2m_min": [14.3, 12.0, 13.5, -7.2, 11.1, 9.8],
            "precipitation_probability_max": [40, 65, 5, 80, 90, 10]
        },
        "hourly": {
            "time": ["2026-09-01T13:00", "2026-09-01T14:00", "2026-09-01T15:00",
                     "2026-09-01T16:00", "2026-09-01T17:00", "2026-09-01T18:00"],
            "temperature_2m": [22.0, 23.7, 24.2, 23.9, 22.4, 21.0],
            "weather_code": [3, 61, 61, 2, 0, 0],
            "precipitation_probability": [10, 40, 55, 20, 5, 5],
            "wind_speed_10m": [2.1, 2.8, 3.4, 3.0, 1.9, 1.5],
            "is_day": [1, 1, 1, 1, 0, 0]
        }
    }"#;

    #[test]
    fn parses_a_recorded_answer() {
        let answer: Answer = serde_json::from_str(RECORDED).expect("ответ разбирается");
        let report = read(answer).expect("это не отказ");

        assert_eq!(report.temperature, Some(23.7));
        assert_eq!(report.sky, Sky::Rain);
        assert!(!report.day, "is_day равен нулю — это ночь");
        assert_eq!(report.wind, Some(4.2));

        // Вероятность нынешнего часа, 14:00, а не следующего
        assert_eq!(report.precipitation, Some(40));
    }

    /// Нынешний день идёт шагом в два часа и начинается со следующего: погоду
    /// этого часа карточка уже показывает крупной цифрой.
    #[test]
    fn the_hours_run_two_at_a_time() {
        let answer: Answer = serde_json::from_str(RECORDED).expect("ответ разбирается");
        let report = read(answer).expect("это не отказ");

        // Запись коротка нарочно — на ней проверяется шаг, а не длина списка
        let times: Vec<&str> = report.hours.iter().map(|hour| hour.at.as_str()).collect();
        assert_eq!(times, ["15:00", "17:00"]);

        let first = &report.hours[0];
        assert_eq!(first.temperature, Some(24.2));
        assert_eq!(first.sky, Sky::Rain);
        assert_eq!(first.precipitation, Some(55));
        assert_eq!(first.wind, Some(3.4));
        assert!(first.day);

        // Ночной час размечен ночью: у ясного неба от этого зависит значок
        assert!(!report.hours[1].day);
    }

    /// Час за часом на трое суток, где температура равна номеру часа в списке.
    ///
    /// Метка нужна затем, что подписью час показывает только «08:00», и из
    /// каких он суток — по ней уже не видно.
    fn three_days() -> HourlyBlock {
        let count: usize = 24 * 3;
        HourlyBlock {
            time: (0..count)
                .map(|at| format!("2026-09-{:02}T{:02}:00", 1 + at / 24, at % 24))
                .collect(),
            temperature_2m: (0..count).map(|at| Some(at as f64)).collect(),
            ..HourlyBlock::default()
        }
    }

    /// Часов берём ровно столько, сколько показывает карточка, — не больше.
    #[test]
    fn the_card_never_gets_more_hours_than_it_shows() {
        let times: Vec<String> = (0..48).map(|at| format!("2026-09-01T{at:02}:00")).collect();
        let hourly = HourlyBlock {
            time: times,
            ..HourlyBlock::default()
        };

        let hours = hourly.from(Some("2026-09-01T00:30"));
        assert_eq!(hours.len(), HOURS);
        // Шесть значений шагом в два часа — это двенадцать часов вперёд
        assert_eq!(hours[0].at, "01:00");
        assert_eq!(hours[HOURS - 1].at, "11:00");
    }

    /// Времени в ответе нет — часы начинаются с начала списка, а не пропадают:
    /// сдвинутые на час градусы лучше прочерков.
    #[test]
    fn without_a_clock_the_hours_start_from_the_beginning() {
        let answer: Answer = serde_json::from_str(RECORDED).expect("ответ разбирается");
        let hourly = answer.hourly.expect("часы на месте");

        let first = hourly.from(None);
        assert_eq!(first.first().map(|hour| hour.at.as_str()), Some("13:00"));
    }

    /// Последний час суток не должен обрывать список на полуслове: часы идут
    /// подряд через полночь, потому и просим прогноз не на одни сутки.
    #[test]
    fn hours_run_across_midnight() {
        let times: Vec<String> = (0..48)
            .map(|at| format!("2026-09-{:02}T{:02}:00", 1 + at / 24, at % 24))
            .collect();
        let hourly = HourlyBlock {
            time: times,
            ..HourlyBlock::default()
        };

        let evening = hourly.from(Some("2026-09-01T23:10"));
        assert_eq!(evening.len(), HOURS);
        assert_eq!(evening[0].at, "00:00");
        assert_eq!(evening[1].at, "02:00");
    }

    /// Прогноз кончился раньше, чем настал нынешний час, — это пустой список,
    /// а не паника и не часы из позавчера.
    #[test]
    fn an_exhausted_forecast_leaves_the_hours_empty() {
        let hourly = HourlyBlock {
            time: vec!["2026-09-01T10:00".to_owned()],
            ..HourlyBlock::default()
        };

        assert!(hourly.from(Some("2026-09-02T10:00")).is_empty());
    }

    /// Окна берутся у своего дня, а не у соседнего: в ответе они лежат подряд,
    /// и день различается только датой в строке времени.
    #[test]
    fn the_windows_belong_to_the_day_that_asked() {
        let hourly = three_days();

        let second = hourly.windows("2026-09-02");
        let times: Vec<&str> = second.iter().map(|hour| hour.at.as_str()).collect();
        assert_eq!(times, WINDOWS);

        // Двадцать восьмой час списка — это 04:00 вторых суток
        let marks: Vec<f64> = second.iter().filter_map(|hour| hour.temperature).collect();
        assert_eq!(marks, vec![28.0, 32.0, 36.0, 40.0, 44.0]);

        // День, которого в ответе нет, — это пустые окна, а не чужие
        assert!(hourly.windows("2026-09-09").is_empty());
    }

    /// Окна едут вместе с днём ряда, а не спрашиваются потом по нажатию.
    #[test]
    fn every_day_of_the_row_carries_its_own_windows() {
        let daily = DailyBlock {
            time: (1..=6).map(|at| format!("2026-09-{at:02}")).collect(),
            ..DailyBlock::default()
        };

        let row = daily.ahead(&three_days());

        // Почасовки хватило на двое суток из пяти в ряду — у остальных окна
        // пусты, и это не поломка, а конец прогноза
        assert_eq!(row[0].windows.len(), WINDOWS.len(), "у завтра нет окон");
        assert_eq!(row[0].windows[0].temperature, Some(28.0));
        assert!(row[2].windows.is_empty(), "окна взялись из ниоткуда");
    }

    /// Ряд начинается с завтрашнего дня: нынешний и есть основной блок.
    #[test]
    fn the_row_starts_tomorrow() {
        let answer: Answer = serde_json::from_str(RECORDED).expect("ответ разбирается");
        let report = read(answer).expect("это не отказ");

        assert_eq!(report.days.len(), DAYS);

        assert_eq!(report.days[0].date, "2026-09-02");
        assert_eq!(report.days[0].sky, Sky::Overcast);
        assert_eq!(report.days[0].day.max, Some(21.8));
        assert_eq!(report.days[0].day.min, Some(12.0));
        assert_eq!(report.days[0].day.precipitation, Some(65));

        // Последний в ряду — шестой день ответа
        assert_eq!(report.days[DAYS - 1].date, "2026-09-06");
        assert_eq!(report.days[DAYS - 1].sky, Sky::Fog);
    }

    /// Клеток в ряду не больше, чем он показывает, даже если суток прислали
    /// больше запрошенного.
    #[test]
    fn the_row_never_outruns_the_card() {
        let daily = DailyBlock {
            time: (1..=9).map(|at| format!("2026-09-{at:02}")).collect(),
            ..DailyBlock::default()
        };

        assert_eq!(daily.ahead(&HourlyBlock::default()).len(), DAYS);
    }

    /// Дни без дат — это пустой ряд, а не пять клеток из одних прочерков.
    #[test]
    fn days_without_dates_are_no_row_at_all() {
        let daily = DailyBlock {
            temperature_2m_max: vec![Some(7.0), Some(9.0)],
            ..DailyBlock::default()
        };

        assert!(daily.ahead(&HourlyBlock::default()).is_empty());
    }

    /// Прогноз на одни сутки — это ряд без клеток, а не паника.
    #[test]
    fn a_single_day_forecast_is_not_a_panic() {
        let daily = DailyBlock {
            time: vec!["2026-09-01".to_owned()],
            temperature_2m_max: vec![Some(7.0)],
            ..DailyBlock::default()
        };

        assert!(daily.ahead(&HourlyBlock::default()).is_empty());
    }

    /// Строка времени приходит от чужого сервиса: не тот формат — показываем
    /// как есть, а не режем по живому.
    #[test]
    fn a_strange_time_is_shown_as_it_came() {
        let hourly = HourlyBlock {
            time: vec!["скоро".to_owned()],
            ..HourlyBlock::default()
        };

        let hours = hourly.from(None);
        assert_eq!(hours[0].at, "скоро");
        // И небо у него не выдумано: незнакомый код — облако
        assert_eq!(hours[0].sky, Sky::Overcast);
    }

    /// Правило проекта «отсутствующее показываем прочерком, а не нулём»
    /// проверяем там, где оно рождается.
    #[test]
    fn nulls_stay_empty() {
        let answer: Answer = serde_json::from_str(
            r#"{"current":{"temperature_2m":null,"weather_code":3,"is_day":1},
                "daily":{"time":["2026-09-01","2026-09-02"],
                         "temperature_2m_max":[null,null],"temperature_2m_min":[null,null],
                         "precipitation_probability_max":[null,null]}}"#,
        )
        .expect("ответ разбирается");

        let report = read(answer).expect("это не отказ");
        assert_eq!(report.temperature, None);
        assert_eq!(report.wind, None);
        assert_eq!(report.precipitation, None);
        assert_eq!(report.days[0].day.max, None);
        assert_eq!(report.days[0].day.precipitation, None);
    }

    /// Пустой ответ — это прочерки, а не отказ: карточка рисуется всегда.
    #[test]
    fn an_empty_answer_is_still_a_report() {
        let report = read(Answer::default()).expect("это не отказ");
        assert_eq!(report.temperature, None);
        assert_eq!(report.sky, Sky::Overcast);
        assert!(report.day);
        assert!(report.hours.is_empty(), "часы взялись из пустого ответа");
        assert!(report.days.is_empty(), "дни взялись из пустого ответа");
    }

    /// Отказ приходит телом с обычным кодом ответа — на прочерки он не похож.
    #[test]
    fn a_refusal_in_the_body_is_an_error() {
        let answer: Answer =
            serde_json::from_str(r#"{"error":true,"reason":"Latitude must be in range"}"#)
                .expect("ответ разбирается");

        assert!(read(answer).is_err());
    }

    /// Каждый код WMO обязан во что-то раскладываться, включая незнакомые.
    #[test]
    fn every_wmo_code_maps_to_a_sky() {
        for code in 0..=120u16 {
            let _ = sky(code);
        }

        assert_eq!(sky(0), Sky::Clear);
        assert_eq!(sky(1), Sky::PartlyCloudy);
        assert_eq!(sky(2), Sky::PartlyCloudy);
        assert_eq!(sky(3), Sky::Overcast);
        assert_eq!(sky(45), Sky::Fog);
        assert_eq!(sky(48), Sky::Fog);
        assert_eq!(sky(53), Sky::Drizzle);
        assert_eq!(sky(65), Sky::Rain);
        assert_eq!(sky(82), Sky::Rain);
        assert_eq!(sky(57), Sky::Sleet);
        assert_eq!(sky(67), Sky::Sleet);
        assert_eq!(sky(75), Sky::Snow);
        assert_eq!(sky(86), Sky::Snow);
        assert_eq!(sky(95), Sky::Thunder);
        assert_eq!(sky(99), Sky::Thunder);

        // Незнакомое небо — облако: своего значка «не знаю» у карточки нет
        assert_eq!(sky(4), Sky::Overcast);
        assert_eq!(sky(1000), Sky::Overcast);
    }

    /// Ничего не нашлось — это пустой список, а не отказ: ключа `results` в
    /// таком ответе нет вовсе.
    #[test]
    fn an_answer_without_results_is_an_empty_list() {
        let hits: Hits =
            serde_json::from_str(r#"{"generationtime_ms":0.5}"#).expect("ответ разбирается");
        assert!(found(hits).is_empty());
    }

    /// Область идёт в подпись, только когда отличается от имени города.
    #[test]
    fn hits_are_labelled_for_the_list() {
        let hits: Hits = serde_json::from_str(
            r#"{"results":[
                {"name":"Москва","latitude":55.75,"longitude":37.61,
                 "country":"Россия","admin1":"Москва"},
                {"name":"Грац","latitude":47.07,"longitude":15.44,
                 "country":"Австрия","admin1":"Штирия"},
                {"name":"Где-то","latitude":1.0,"longitude":2.0}
            ]}"#,
        )
        .expect("ответ разбирается");

        let list = found(hits);
        assert_eq!(list.len(), 3);
        // Область совпала с именем — повторять её незачем
        assert_eq!(list[0].region, "Россия");
        assert_eq!(list[1].region, "Штирия, Австрия");
        // Ни страны, ни области — подпись остаётся пустой, а не «, »
        assert_eq!(list[2].region, "");
    }

    /// Разметка уходит на фронт как есть: snake_case там уже однажды всплывал.
    #[test]
    fn a_report_arrives_in_camel_case() {
        let answer: Answer = serde_json::from_str(RECORDED).expect("ответ разбирается");
        let json = serde_json::to_string(&read(answer).expect("это не отказ")).expect("пишется");

        assert!(json.contains(r#""sky":"rain""#), "{json}");
        assert!(json.contains(r#""temperature":23.7"#), "{json}");
        assert!(json.contains(r#""precipitation":40"#), "{json}");
        assert!(json.contains(r#""wind":4.2"#), "{json}");
        assert!(json.contains(r#""at":"15:00""#), "{json}");
        assert!(json.contains(r#""date":"2026-09-02""#), "{json}");
        assert!(json.contains(r#""windows""#), "{json}");
    }

    /// Малооблачно должно доехать до фронта одним словом, а не двумя.
    #[test]
    fn a_two_word_sky_arrives_in_camel_case() {
        let json = serde_json::to_string(&Sky::PartlyCloudy).expect("пишется");
        assert_eq!(json, r#""partlyCloudy""#);
    }
}
