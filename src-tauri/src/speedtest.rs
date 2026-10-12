//! Замер скорости канала по NDT7 на серверах Measurement Lab.
//!
//! Запускается кнопкой и только ей: замер выкачивает сотни мегабайт, и делать
//! это фоном раз в минуту значит незаметно съедать канал.
//!
//! ВАЖНО, всё проверено замерами. M-Lab даёт сотни Мбит/с, у
//! Cloudflare тот же порядок. Файловые серверы, выложенные ровно для таких
//! проверок, меряют маршрут, а не канал: `proof.ovh.net` в один поток дал
//! 22 Мбит/с и 300 в восемь, `speedtest.tele2.net` — 27 и 147, а
//! `speed.hetzner.de` не разрешается вовсе, даже через 1.1.1.1.
//!
//! Второе, и главное: у Cloudflare счётчик запросов, и после двух замеров
//! подряд он отвечал 429 минут на пять. M-Lab для замеров и создан, лимит там
//! стоит на подборе сервера, а не на самом замере, — поэтому подобранные
//! серверы мы держим у себя и гоняем тесты по ним, пока живы их токены.
//!
//! Расплата: результаты уходят в открытый датасет M-Lab под CC0. Для виджета
//! это неважно, но знать об этом надо.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{Bytes, Message};

/// Подбор серверов. Отдаёт сразу несколько штук с готовыми токенами доступа
/// в ссылках.
const LOCATE_URL: &str = "https://locate.measurementlab.net/v2/nearest/ndt/ndt7";

/// Хабы Европы — они же выбор по умолчанию.
///
/// ВАЖНО: **без страны подбор берёт ближайшее к нашему адресу, а с включённым
/// VPN ближайшим оказывается его выходной узел.** Замер ушёл бы куда-нибудь в
/// Персидский залив, и цифра говорила бы о канале туда, а не о канале вообще.
/// Крупные хабы — то, через что идёт всё остальное, и мерить осмысленно до них.
///
/// Стран в паре две, и это не про запас: запрос по стране отдаёт до четырёх
/// площадок, а соединений нужно восемь — ровно [`STREAMS`]. Соседние страны в
/// выдачу попадают (Париж к Германии, Брюссель к Нидерландам) — это те же хабы,
/// и мешать они не будут.
const EUROPE: [&str; 2] = ["DE", "NL"];

/// Хабы Северной Америки.
///
/// Берутся, только когда мы там и оказались: выйдя из VPN в Штатах, мерить до
/// Франкфурта значит мерить океан. Пара по той же причине, что и в Европе —
/// четыре площадки на запрос, а замер в четыре потока через туннель и есть та
/// недомерка, ради которой заводили восемь.
const AMERICA: [&str; 2] = ["US", "CA"];

/// Какая пара хабов подходит тому, кто вышел в этой стране.
///
/// Отдельной функцией — ради теста: живьём это проверяется только сменой
/// выходного узла VPN, то есть никогда.
fn hubs_for(country: Option<&str>) -> [&'static str; 2] {
    match country {
        // Канада тут не гостья: из Торонто до Нью-Йорка ближе, чем до
        // Франкфурта, и вторая страна нужна ровно ради восьми площадок
        Some("US") | Some("CA") => AMERICA,
        // Всё прочее — и неизвестность в том числе. Европа тут не «правильный
        // ответ», а безопасный: до её хабов идёт всё остальное
        _ => EUROPE,
    }
}

/// Протокол NDT7. Без него сервер закрывает соединение при рукопожатии.
const SUBPROTOCOL: &str = "net.measurementlab.ndt.v7";

/// Сколько держим подобранные серверы.
///
/// ВАЖНО: спрашивать подбор на каждый замер — это ровно тот же лимит, из-за
/// которого ушли с Cloudflare. Токены в выданных ссылках живут дольше, так что
/// список запрашиваем редко и гоняем замеры по нему.
const SERVERS_FRESH_FOR: Duration = Duration::from_secs(30 * 60);

/// Сколько секунд качаем и отдаём.
///
/// Половина от штатных десяти секунд NDT7: на нынешнем канале это под три
/// сотни мегабайт трафика, и вдвое больше ради второго знака после запятой не
/// стоит того. Разгон TCP заканчивается на первых долях секунды.
const TRANSFER_BUDGET: Duration = Duration::from_secs(5);

/// Сколько соединений открываем разом.
///
/// ВАЖНО: **одно соединение меряет не канал, а один поток TCP.** На прямом
/// проводе разница невелика, а вот через VPN она решает всё: туннель режет
/// одиночный поток окном, шифрованием и своим MTU, и замер показывал впятеро
/// меньше того, что видят Ookla и прочие — они льют в несколько потоков.
///
/// Восемь: по соединению на площадку, а площадок ровно столько отдают два
/// запроса по странам пары хабов. Больше не берём не из вежливости к M-Lab,
/// а потому что
/// восьми хватает: разгон TCP кончается на первых долях секунды, и девятый
/// поток делит тот же канал, ничего к нему не добавляя.
///
/// По соединению на сервер честнее, чем восемь в один: токен в ссылке выдан на
/// один заход.
const STREAMS: usize = 8;

/// Размер сообщения при отдаче.
const UPLOAD_MESSAGE: usize = 1 << 20;

/// Сколько сообщений держим в очереди сокета.
///
/// Отправка возвращается, как только сообщение легло в буфер, а не когда ушло
/// на провод. Без потолка мы набили бы в память десятки мегабайт, которые
/// сервер не увидит, и замер кончился бы на секунду раньше провода.
const UPLOAD_QUEUE: usize = 4;

/// Как часто показываем промежуточную скорость.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// Сколько терпим молчание сервера.
///
/// ВАЖНО: **бюджет передачи сам по себе замер не кончает.** Он проверяется на
/// пришедшем сообщении, а замолчавший сервер сообщений не шлёт — и `next()`
/// ждёт их сколько угодно. Тем же кончается и дочитывание отчётов после
/// закрытия сокета: не пришли закрывающий кадр и `None` — цикл вечен, а с ним
/// вечна и поднятая `running`, то есть запертая кнопка до перезапуска.
///
/// Три секунды: на живом замере сообщения идут десятками в секунду, столько
/// тишины подряд означает мёртвое соединение, а не заминку.
const SILENCE_LIMIT: Duration = Duration::from_secs(3);

/// Потолок на один этап замера.
///
/// Своё молчание сервера ловится сроком выше, а это — от обратного: отправка
/// упирается в приём, и перестань сервер читать, `flush` будет ждать окна TCP
/// сколько угодно. Условие цикла отдачи до следующей проверки при этом не
/// дойдёт вовсе. Потолок кончает такой замер отказом, а не вечно нажатой
/// кнопкой.
const STAGE_LIMIT: Duration = Duration::from_secs(30);

/// Заведомо больше бюджета: обрывать замер по таймауту клиента незачем,
/// он и так остановится сам.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

const SPEEDTEST_EVENT: &str = "speedtest";

/// Файл истории. Отдельный от `settings.json`: у них разная судьба и разные
/// пишущие стороны — настройки правит трей, историю дописывает замер.
const HISTORY_FILE: &str = "speedtest.json";

/// Сколько замеров помним.
///
/// Столько же ячеек по горизонтали у графика. Больше в карточку шириной 440
/// точек не влезает: тридцать ячеек — это по двенадцать точек на каждую, а
/// ниже десяти вертикальные линии сетки сливаются в штриховку.
const HISTORY_LIMIT: usize = 30;

/// Результат замера. `null` — до этого этапа не дошли.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Report {
    /// Мегабиты в секунду.
    pub download: Option<f64>,
    pub upload: Option<f64>,
    /// Миллисекунды.
    pub latency: Option<f64>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    /// Подбор ближайшего сервера. Обычно мгновенный: список лежит наготове.
    Locate,
    Download,
    Upload,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(tag = "status", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum View {
    /// Замера ещё не было.
    #[default]
    Idle,
    /// Идёт замер. Готовое уже в `report`, а во время скачивания там же
    /// промежуточная скорость — чтобы виджет шевелился, а не ждал молча.
    Running { stage: Stage, report: Report },
    Done { report: Report },
    /// Замер сорвался. Что успели измерить — в `report`.
    Failed { reason: String, report: Report },
}

/// Состояние вместе с историей — ровно то, что видит виджет.
///
/// `flatten` кладёт поля `View` на верхний уровень, поэтому у объекта
/// по-прежнему есть `status` рядом с `history`, а не вложенный `view`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    #[serde(flatten)]
    pub view: View,
    /// Прошлые замеры, старые в начале.
    pub history: Vec<Report>,
    /// Город сервера, на котором меряем. Виден, пока идёт замер: канал у всех
    /// один, а вот докуда он меряется — знать полезно.
    pub server: Option<String>,
}

/// Подобранный сервер: ссылки уже с токенами доступа.
#[derive(Debug, Clone)]
struct Server {
    city: String,
    /// Две буквы страны. Нужна одному — первому подбору, который выясняет,
    /// где мы вышли.
    country: String,
    download: String,
    upload: String,
}

#[derive(Default)]
pub struct Speedtest {
    view: Mutex<View>,
    history: Mutex<Vec<Report>>,
    /// Подобранные серверы и когда их подобрали.
    servers: Mutex<Option<(Instant, Vec<Server>)>>,
    running: AtomicBool,
}

impl Speedtest {
    pub fn new(history: Vec<Report>) -> Self {
        Self {
            history: Mutex::new(trimmed(history)),
            ..Self::default()
        }
    }

    fn set(&self, view: View) {
        // Отравленный мьютекс не повод падать: данные внутри осмысленные
        *self
            .view
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = view;
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            view: self
                .view
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone(),
            history: self
                .history
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone(),
            server: self
                .servers
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_ref()
                .and_then(|(_, servers)| servers.first())
                .map(|server| server.city.clone()),
        }
    }

    /// Серверы из прошлого подбора, если они ещё не выдохлись.
    fn fresh_servers(&self) -> Option<Vec<Server>> {
        let servers = self
            .servers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        servers
            .as_ref()
            .filter(|(picked, _)| picked.elapsed() < SERVERS_FRESH_FOR)
            .map(|(_, servers)| servers.clone())
    }

    fn keep_servers(&self, servers: Vec<Server>) {
        *self
            .servers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((Instant::now(), servers));
    }

    /// Забывает подобранное: токены протухли или сервер перестал отвечать.
    fn forget_servers(&self) {
        *self
            .servers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }

    /// Дописывает замер в историю и отдаёт её целиком для записи на диск.
    fn remember(&self, report: Report) -> Vec<Report> {
        let mut history = self
            .history
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        history.push(report);
        let extra = history.len().saturating_sub(HISTORY_LIMIT);
        history.drain(..extra);
        history.clone()
    }
}

/// Оставляет последние [`HISTORY_LIMIT`] замеров.
///
/// Обрезаем и при чтении с диска: файл правят руками, и тысяча строк в нём
/// превратилась бы в тысячу столбиков шириной в волос.
fn trimmed(mut history: Vec<Report>) -> Vec<Report> {
    let extra = history.len().saturating_sub(HISTORY_LIMIT);
    history.drain(..extra);
    history
}

/// Текущее состояние — окно спрашивает его при открытии.
#[tauri::command]
pub fn speedtest_view(state: State<'_, Speedtest>) -> Snapshot {
    state.snapshot()
}

fn history_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("не найден каталог настроек: {error}"))?;
    Ok(dir.join(HISTORY_FILE))
}

/// Читает историю. Любая беда — считаем, что замеров ещё не было: из-за
/// диаграммы приложение стартовать не откажется.
pub fn load_history(app: &AppHandle) -> Vec<Report> {
    let Ok(path) = history_path(app) else {
        return Vec::new();
    };
    let Ok(text) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

fn save_history(app: &AppHandle, history: &[Report]) {
    let write = || -> Result<(), String> {
        let path = history_path(app)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("не создать каталог настроек: {error}"))?;
        }
        let text = serde_json::to_string_pretty(history)
            .map_err(|error| format!("не сериализовать историю: {error}"))?;
        crate::disk::write_atomically(&path, &text)
            .map_err(|error| format!("не записать историю: {error}"))
    };

    if let Err(error) = write() {
        eprintln!("история замеров не сохранилась: {error}");
    }
}

#[tauri::command]
pub fn speedtest_start(app: AppHandle) -> Result<(), String> {
    // Второй заход поверх первого исказил бы оба: они делят один канал
    if app.state::<Speedtest>().running.swap(true, Ordering::SeqCst) {
        return Err(tr!("speedtest.error.busy"));
    }

    tauri::async_runtime::spawn(async move {
        let _busy = Busy(app.clone());
        run(&app).await;
    });

    Ok(())
}

/// Поднятый признак «идёт замер», опускающийся сам.
///
/// ВАЖНО: **строкой после `await` этого не добиться.** Паника внутри задачи
/// уносит её целиком, и такая строка не исполняется вовсе: признак остался бы
/// поднятым до перезапуска приложения, а кнопка замера — запертой навсегда.
/// `Drop` случается и при развёртывании стека.
struct Busy(AppHandle);

impl Drop for Busy {
    fn drop(&mut self) {
        // На выходе из приложения состояния может уже не быть — это не повод
        // падать в чужом `Drop`
        if let Some(state) = self.0.try_state::<Speedtest>() {
            state.running.store(false, Ordering::SeqCst);
        }
    }
}

async fn run(app: &AppHandle) {
    let mut report = Report::default();

    publish(app, View::Running { stage: Stage::Locate, report });
    match measure_download(app, report).await {
        Ok(measured) => {
            report.download = Some(measured.rate);
            // Задержку берёт сам сервер из TCP, отдельных проб не нужно вовсе
            report.latency = measured.latency;
        }
        Err(reason) => {
            publish(app, View::Failed { reason, report });
            return;
        }
    }

    publish(app, View::Running { stage: Stage::Upload, report });
    match measure_upload(app, report).await {
        Ok(rate) => report.upload = Some(rate),
        Err(reason) => {
            publish(app, View::Failed { reason, report });
            return;
        }
    }

    // Сначала в историю, потом рассылка: иначе виджет получил бы готовый
    // результат без точки по нему и дорисовал бы её вторым событием
    remember(app, report);
    publish(app, View::Done { report });
}

/// Скачивание, а на непринятом пропуске — второй заход с новым подбором.
///
/// ВАЖНО: **токен в ссылке привязан к адресу и ко времени.** Список серверов мы
/// держим полчаса, и за это время адрес успевает смениться — включили VPN,
/// переехали в другую сеть, — после чего сервер отвечает 401 прямо на
/// рукопожатии. Прежде это выглядело отказом замера, хотя чинится оно ровно
/// новым подбором, и кнопка при этом запиралась на минуту ни за что.
async fn measure_download(app: &AppHandle, report: Report) -> Result<Measured, String> {
    let mut retried = false;

    loop {
        let servers = pick_servers(app).await?;
        publish(app, View::Running { stage: Stage::Download, report });

        // Потолок этапа нужен и здесь: молчание сервера ловит `SILENCE_LIMIT`
        // внутри `pull`, но начинает считать он только с установленного
        // соединения. Сервер, глотающий SYN, держит `connect_async` до
        // таймаута TCP у системы, и весь этап ждёт вместе с ним
        let outcome = tokio::time::timeout(
            STAGE_LIMIT,
            download(&servers, |rate| {
                let partial = Report { download: Some(rate), ..report };
                publish(app, View::Running { stage: Stage::Download, report: partial });
            }),
        )
        .await;

        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(_) => return Err(tr!("speedtest.error.downloadTimeout")),
        };

        match outcome {
            Ok(measured) => return Ok(measured),
            Err(trouble) => {
                // Мёртвый сервер и протухший токен лечатся новым подбором
                app.state::<Speedtest>().forget_servers();

                if trouble.stale() && !retried {
                    retried = true;
                    // Видно, что идёт не замер, а второй подбор
                    publish(app, View::Running { stage: Stage::Locate, report });
                    continue;
                }
                return Err(trouble.reason());
            }
        }
    }
}

/// Отдача по тому же правилу. Сервер берётся из запомненных, то есть тот же,
/// на котором мерили скачивание.
async fn measure_upload(app: &AppHandle, report: Report) -> Result<f64, String> {
    let mut retried = false;

    loop {
        let servers = pick_servers(app).await?;

        match tokio::time::timeout(
            STAGE_LIMIT,
            upload(&servers, |rate| {
                let partial = Report { upload: Some(rate), ..report };
                publish(app, View::Running { stage: Stage::Upload, report: partial });
            }),
        )
        .await
        {
            Ok(Ok(rate)) => return Ok(rate),
            Ok(Err(trouble)) => {
                app.state::<Speedtest>().forget_servers();

                if trouble.stale() && !retried {
                    retried = true;
                    continue;
                }
                return Err(trouble.reason());
            }
            Err(_) => return Err(tr!("speedtest.error.uploadTimeout")),
        }
    }
}

/// Ближайшие серверы: из запомненных, а если их нет — новым подбором.
///
/// Их несколько, потому что и соединений несколько: замер идёт в [`STREAMS`]
/// потоков, по одному на сервер. Меньше подобралось — меньше и потоков, это не
/// беда.
async fn pick_servers(app: &AppHandle) -> Result<Vec<Server>, String> {
    if let Some(servers) = app.state::<Speedtest>().fresh_servers() {
        if !servers.is_empty() {
            return Ok(servers.into_iter().take(STREAMS).collect());
        }
    }

    let servers = locate().await?;
    if servers.is_empty() {
        return Err(tr!("speedtest.error.noServers"));
    }

    app.state::<Speedtest>().keep_servers(servers.clone());
    Ok(servers.into_iter().take(STREAMS).collect())
}

/// Спрашивает у M-Lab серверы того хаба, до которого нам осмысленно мерить.
///
/// Зовётся редко: ссылки приходят с токенами доступа, и лимит у M-Lab стоит
/// именно на этом запросе, а не на самих замерах.
///
/// Заходов три, и первый нужен только чтобы понять, где мы вышли: подбор без
/// страны отдаёт ближайшие к нашему адресу площадки, а через VPN — ближайшие к
/// его выходному узлу. По их стране и выбирается пара хабов.
async fn locate() -> Result<Vec<Server>, String> {
    let mut servers: Vec<Server> = Vec::new();
    let mut trouble: Option<String> = None;

    for hub in hubs_for(whereabouts().await.as_deref()) {
        match locate_in(Some(hub)).await {
            Ok(found) => servers.extend(found),
            // Один хаб не ответил — меряем по второму: отказываться от замера
            // целиком тут не за что
            Err(reason) => {
                eprintln!("[спидтест] подбор в {hub} не прошёл: {reason}");
                trouble.get_or_insert(reason);
            }
        }
    }

    if servers.is_empty() {
        return Err(trouble.unwrap_or_else(|| "M-Lab offered no servers".to_owned()));
    }

    Ok(without_repeats(servers))
}

/// В какой стране мы оказались — по ближайшей к нам площадке.
///
/// Отказ тут не отказ замера: не ответил подбор — считаем, что мы в Европе, и
/// идём прежним путём. Тот же принцип, что у [`locate`] с не ответившим хабом.
async fn whereabouts() -> Option<String> {
    match locate_in(None).await {
        // Ближайшая и есть самая близкая: список приходит от неё
        Ok(found) => found.into_iter().next().map(|server| server.country),
        Err(reason) => {
            eprintln!("[спидтест] где мы — выяснить не вышло: {reason}");
            None
        }
    }
}

/// Выдача подбора.
///
/// Объявлена рядом с [`read_servers`], а не внутри запроса: разбор её проверяет
/// тест на записанном ответе, а из тела функции такой тип не достать.
#[derive(Deserialize)]
struct Answer {
    results: Vec<Found>,
}

#[derive(Deserialize)]
struct Found {
    location: Option<Location>,
    urls: std::collections::HashMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
struct Location {
    city: Option<String>,
    /// Две буквы страны — по ним и выбирается пара хабов.
    country: Option<String>,
}

/// Подбор в одной стране. `None` — ближайшие к нам, кто бы мы ни были.
async fn locate_in(country: Option<&str>) -> Result<Vec<Server>, String> {
    let address = match country {
        // Строкой, а не через `query`: у reqwest он живёт за фичей, которой у
        // нас нет, а страна тут — две буквы из своего же списка
        Some(country) => format!("{LOCATE_URL}?country={country}"),
        None => LOCATE_URL.to_owned(),
    };

    let answer: Answer = client()
        .map_err(|error| format!("HTTP client failed: {error}"))?
        .get(address)
        .send()
        .await
        .map_err(describe)?
        .error_for_status()
        .map_err(describe)?
        .json()
        .await
        .map_err(|error| format!("M-Lab replied with nonsense: {error}"))?;

    Ok(read_servers(answer.results))
}

/// Раскладывает выдачу подбора в серверы.
///
/// Отдельной функцией — ради теста: записанный ответ проверяет и разбор города
/// со страной, и то, что площадка без нужных ссылок выбрасывается целиком.
fn read_servers(results: Vec<Found>) -> Vec<Server> {
    results
        .into_iter()
        .filter_map(|found| {
            let place = found.location.unwrap_or_default();
            Some(Server {
                city: place.city.unwrap_or_else(|| "unknown".to_owned()),
                country: place.country.unwrap_or_default(),
                download: found.urls.get("wss:///ndt/v7/download")?.clone(),
                upload: found.urls.get("wss:///ndt/v7/upload")?.clone(),
            })
        })
        .collect()
}

/// Убирает повторы машин.
///
/// Запросы по соседним странам пересекаются: у Нидерландов в выдаче попадается
/// Брюссель, у Германии — Париж, и одна и та же площадка может прийти дважды.
/// Два соединения к ней вместо двух разных — это не восемь потоков, а семь.
fn without_repeats(servers: Vec<Server>) -> Vec<Server> {
    let mut seen: Vec<String> = Vec::new();

    servers
        .into_iter()
        .filter(|server| {
            let machine = machine_of(&server.download);
            if seen.contains(&machine) {
                return false;
            }

            seen.push(machine);
            true
        })
        .collect()
}

/// Имя машины из ссылки: `wss://mlab1-fra01.../ndt/v7/download?...` → хост.
///
/// Токен в запросе у каждой ссылки свой, поэтому сравнивать их целиком нельзя —
/// одинаковых не бывает вовсе.
fn machine_of(url: &str) -> String {
    url.split_once("://")
        .map_or(url, |(_, rest)| rest)
        .split('/')
        .next()
        .unwrap_or(url)
        .to_owned()
}

/// Что вышло из скачивания.
struct Measured {
    /// Мегабиты в секунду.
    rate: f64,
    /// Миллисекунды: наименьшее время оборота, каким его видит сам сервер.
    latency: Option<f64>,
}

/// Чем кончился заход на сервер.
///
/// Разделены ради одного: протухший токен — не отказ замера, а просьба
/// подобрать сервер заново.
#[derive(Debug)]
enum Trouble {
    /// Ссылку не приняли: токен в ней выдан другому адресу или уже стар.
    Stale(String),
    /// Всё остальное — мёртвый сервер, оборванная связь, молчание.
    Broken(String),
}

impl Trouble {
    fn stale(&self) -> bool {
        matches!(self, Trouble::Stale(_))
    }

    fn reason(self) -> String {
        match self {
            Trouble::Stale(text) | Trouble::Broken(text) => text,
        }
    }
}

/// Отказ ли это токена.
///
/// ВАЖНО: **ссылки M-Lab приходят с токеном доступа внутри, и он привязан ко
/// времени и к адресу, с которого его просили.** Включённый VPN, переезд в
/// другую сеть или просто полчаса простоя — и сервер отвечает 401 прямо на
/// рукопожатии. Отдельной строкой, а не по тексту ошибки: текст английский и
/// приходит от чужой библиотеки.
fn stale_status(status: u16) -> bool {
    status == 401 || status == 403
}

/// Открывает сокет NDT7.
async fn connect(url: &str) -> Result<Socket, Trouble> {
    let mut request = url
        .into_client_request()
        .map_err(|error| Trouble::Broken(tr!("speedtest.error.badLink", error = error)))?;

    // Без этого заголовка сервер закрывает соединение сразу после рукопожатия
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        SUBPROTOCOL
            .parse()
            .map_err(|_| Trouble::Broken(tr!("speedtest.error.badProtocol")))?,
    );

    let (socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(refused)?;

    Ok(socket)
}

/// Разбирает отказ рукопожатия: отдельно тот, что лечится новым подбором.
fn refused(error: tokio_tungstenite::tungstenite::Error) -> Trouble {
    if let tokio_tungstenite::tungstenite::Error::Http(answer) = &error {
        if stale_status(answer.status().as_u16()) {
            return Trouble::Stale(tr!("speedtest.error.tokenRefused"));
        }
    }

    Trouble::Broken(tr!("speedtest.error.noAnswer", error = error))
}

type Socket = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

/// Скачивание: сервер льёт двоичные сообщения, мы их считаем.
///
/// Считаем у себя, а не по отчётам сервера: на приёме клиент видит ровно то,
/// что доехало, и это самая честная цифра из доступных.
async fn download(servers: &[Server], mut progress: impl FnMut(f64)) -> Result<Measured, Trouble> {
    let flow = Arc::new(Flow::new(servers.len()));
    let mut streams = Vec::new();

    for server in servers {
        let url = server.download.clone();
        let flow = Arc::clone(&flow);

        streams.push(tauri::async_runtime::spawn(async move {
            let outcome = pull(&url, &flow).await;
            // Считаться законченным поток обязан при любом исходе, иначе показ
            // скорости внизу ждал бы его вечно
            flow.done();
            outcome
        }));
    }

    // Пока льётся — показываем, сколько уже набежало по всем потокам разом
    while flow.running() {
        tokio::time::sleep(PROGRESS_INTERVAL).await;
        if let Some(rate) = flow.rate() {
            progress(rate);
        }
    }

    let mut latency: Option<f64> = None;
    let mut trouble: Option<Trouble> = None;
    let mut worked = 0_usize;

    for stream in streams {
        match stream.await {
            Ok(Ok(seen)) => {
                worked += 1;
                latency = quicker(latency, seen);
            }
            Ok(Err(cause)) => trouble = keep(trouble, cause),
            // Задача сорвалась — это отказ, но с непонятной причиной
            Err(_) => trouble = keep(trouble, Trouble::Broken(tr!("speedtest.error.brokeOff"))),
        }
    }

    // Хватает одного дошедшего потока: остальные могли упереться в свой сервер,
    // а канал у них общий и цифра всё равно осмысленная
    if worked == 0 {
        return Err(trouble.unwrap_or(Trouble::Broken(tr!("speedtest.error.noData"))));
    }

    match flow.rate() {
        Some(rate) => Ok(Measured { rate, latency }),
        None => Err(Trouble::Broken(tr!("speedtest.error.noData"))),
    }
}

/// Общий счёт на все соединения замера.
///
/// Скорость складывается из всех потоков разом, поэтому и байты, и часы —
/// общие. Часы пускает то соединение, до которого данные дошли первым:
/// рукопожатия у остальных к пропускной способности отношения не имеют.
#[derive(Debug)]
struct Flow {
    received: AtomicU64,
    clock: Mutex<Clock>,
    /// Сколько соединений ещё живо.
    running: AtomicUsize,
}

/// Часы замера: от первого принятого байта до последнего.
#[derive(Debug, Default)]
struct Clock {
    started: Option<Instant>,
    last: Option<Instant>,
}

impl Flow {
    fn new(streams: usize) -> Self {
        Self {
            received: AtomicU64::new(0),
            clock: Mutex::new(Clock::default()),
            running: AtomicUsize::new(streams),
        }
    }

    /// Пускает общие часы, если их ещё никто не пустил.
    fn begin(&self) {
        let mut clock = self
            .clock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        clock.started.get_or_insert_with(Instant::now);
    }

    /// Сколько времени заняло принятое.
    ///
    /// ВАЖНО: **конец меряем последним пришедшим байтом, а не «сейчас».**
    /// Потоки прекращают приём ровно по бюджету, а итог считается заметно
    /// позже: цикл показа спит до [`PROGRESS_INTERVAL`], и только после него
    /// дожидаются сами задачи. Делённые на «сейчас», пятисекундные байты
    /// превращались в пять с четвертью секунд, и скорость выходила процентов
    /// на пять ниже настоящей — всегда в одну сторону.
    fn elapsed(&self) -> Option<Duration> {
        let clock = self
            .clock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let started = clock.started?;
        Some(match clock.last {
            Some(last) => last.duration_since(started),
            None => started.elapsed(),
        })
    }

    /// Считает пришедшее и отмечает, что приём дошёл до этой минуты.
    fn add(&self, bytes: usize) {
        self.received.fetch_add(bytes as u64, Ordering::Relaxed);

        let mut clock = self
            .clock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        clock.last = Some(Instant::now());
    }

    fn done(&self) {
        self.running.fetch_sub(1, Ordering::SeqCst);
    }

    fn running(&self) -> bool {
        self.running.load(Ordering::SeqCst) > 0
    }

    /// Скорость по всему, что набежало. `None` — данные ещё не пошли.
    fn rate(&self) -> Option<f64> {
        let elapsed = self.elapsed()?;
        let received = self.received.load(Ordering::Relaxed);

        (received > 0).then(|| megabits(received, elapsed))
    }
}

/// Одно соединение скачивания. Отдаёт время оборота, каким его видит сервер.
async fn pull(url: &str, flow: &Flow) -> Result<Option<f64>, Trouble> {
    let mut socket = connect(url).await?;

    let mut counted = false;
    let mut latency: Option<f64> = None;

    loop {
        // Молчание дольше срока — это мёртвое соединение: бюджет передачи
        // проверяется на пришедшем сообщении и сам такой замер не кончит
        let Some(incoming) = tokio::time::timeout(SILENCE_LIMIT, socket.next())
            .await
            .map_err(|_| Trouble::Broken(tr!("speedtest.error.silent")))?
        else {
            break;
        };

        let message =
            incoming.map_err(|error| Trouble::Broken(tr!("speedtest.error.droppedWith", error = error)))?;

        match message {
            Message::Binary(chunk) => {
                // Первое сообщение только пускает часы: приехало оно за время,
                // которое к скорости отношения не имеет
                if !counted {
                    counted = true;
                    flow.begin();
                    continue;
                }

                flow.add(chunk.len());

                if flow.elapsed().is_some_and(|elapsed| elapsed >= TRANSFER_BUDGET) {
                    let _ = socket.close(None).await;
                    return Ok(latency);
                }
            }
            // Отчёт сервера о том, как он видит соединение
            Message::Text(report) => latency = min_rtt(&report).or(latency),
            Message::Close(_) => break,
            _ => {}
        }
    }

    if counted {
        Ok(latency)
    } else {
        Err(Trouble::Broken(tr!("speedtest.error.noData")))
    }
}

/// Меньшее из двух времён оборота: канал один, и лучшее из измеренных ближе
/// всего к правде.
fn quicker(known: Option<f64>, seen: Option<f64>) -> Option<f64> {
    match (known, seen) {
        (Some(known), Some(seen)) => Some(known.min(seen)),
        (known, seen) => known.or(seen),
    }
}

/// Первая беда и остаётся причиной отказа: следующие — уже её последствия.
fn keep(known: Option<Trouble>, cause: Trouble) -> Option<Trouble> {
    match known {
        // Протухший токен важнее прочего: на нём замер повторяют
        Some(known) if !known.stale() && cause.stale() => Some(cause),
        Some(known) => Some(known),
        None => Some(cause),
    }
}

/// Отдача: льём двоичные сообщения, а скорость берём из отчёта сервера.
///
/// Своим часам тут верить нельзя: отправка возвращается, когда сообщение легло
/// в буфер сокета, а не когда ушло на провод. Сервер же считает то, что до
/// него дошло, — это и есть отдача.
/// Последнее, что сказал о себе каждый поток отдачи.
///
/// ВАЖНО: **у отдачи нет общего счётчика байт, как у скачивания.** Там мы сами
/// считаем принятое и делим на время; здесь скорость называет сервер, и каждый
/// поток свою — сложить их можно только по последнему отчёту каждого. Ради
/// этого и заведена таблица: без неё ход отдачи не показать вовсе, и кольцо
/// половину замера стояло пустым.
struct Rates {
    latest: Mutex<Vec<f64>>,
    running: AtomicUsize,
}

impl Rates {
    fn new(streams: usize) -> Self {
        Self {
            latest: Mutex::new(vec![0.0; streams]),
            running: AtomicUsize::new(streams),
        }
    }

    fn set(&self, stream: usize, rate: f64) {
        if let Ok(mut latest) = self.latest.lock() {
            if let Some(slot) = latest.get_mut(stream) {
                *slot = rate;
            }
        }
    }

    fn done(&self) {
        self.running.fetch_sub(1, Ordering::SeqCst);
    }

    fn running(&self) -> bool {
        self.running.load(Ordering::SeqCst) > 0
    }

    /// Сумма по потокам. `None` — не отчитался пока никто.
    fn total(&self) -> Option<f64> {
        let latest = self.latest.lock().ok()?;
        let total: f64 = latest.iter().sum();
        (total > 0.0).then_some(total)
    }
}

async fn upload(servers: &[Server], mut progress: impl FnMut(f64)) -> Result<f64, Trouble> {
    let rates = Arc::new(Rates::new(servers.len()));
    let mut streams = Vec::new();

    for (index, server) in servers.iter().enumerate() {
        let url = server.upload.clone();
        let rates = Arc::clone(&rates);

        streams.push(tauri::async_runtime::spawn(async move {
            let outcome = push(&url, index, &rates).await;
            // Законченным поток обязан считаться при любом исходе, иначе показ
            // хода ждал бы его вечно — то же правило, что у скачивания
            rates.done();
            outcome
        }));
    }

    // Пока льётся — показываем сумму последних отчётов
    while rates.running() {
        tokio::time::sleep(PROGRESS_INTERVAL).await;
        if let Some(rate) = rates.total() {
            progress(rate);
        }
    }

    let mut total = 0.0;
    let mut trouble: Option<Trouble> = None;
    let mut worked = 0_usize;

    for stream in streams {
        match stream.await {
            Ok(Ok(rate)) => {
                worked += 1;
                total += rate;
            }
            Ok(Err(cause)) => trouble = keep(trouble, cause),
            Err(_) => trouble = keep(trouble, Trouble::Broken(tr!("speedtest.error.uploadBrokeOff"))),
        }
    }

    // Скорости складываются: сервер каждого потока считает только то, что
    // дошло до него, а канал у них общий
    if worked == 0 {
        return Err(trouble.unwrap_or(Trouble::Broken(tr!("speedtest.error.noBytes"))));
    }

    Ok(total)
}

/// Одно соединение отдачи. Скорость берётся из отчёта сервера.
async fn push(url: &str, stream: usize, rates: &Rates) -> Result<f64, Trouble> {
    let mut socket = connect(url).await?;
    // Один буфер на весь замер: `Bytes` делится ссылкой, и мегабайт не
    // переписывается заново на каждое сообщение
    let payload = Bytes::from(vec![0u8; UPLOAD_MESSAGE]);

    let started = Instant::now();
    let mut rate: Option<f64> = None;
    let mut queued = 0usize;

    while started.elapsed() < TRANSFER_BUDGET {
        socket
            .feed(Message::Binary(payload.clone()))
            .await
            .map_err(|error| Trouble::Broken(tr!("speedtest.error.uploadDropped", error = error)))?;

        queued += 1;
        if queued >= UPLOAD_QUEUE {
            queued = 0;
            socket
                .flush()
                .await
                .map_err(|error| Trouble::Broken(tr!("speedtest.error.uploadDropped", error = error)))?;
        }

        // Отчёты сервера разбираем по ходу, но ждать их не станем
        while let Ok(Some(Ok(message))) =
            tokio::time::timeout(Duration::ZERO, socket.next()).await
        {
            if let Message::Text(report) = message {
                rate = throughput(&report).or(rate);
                if let Some(rate) = rate {
                    rates.set(stream, rate);
                }
            }
        }
    }

    let _ = socket.close(None).await;

    // Последние отчёты приходят уже после закрытия — досматриваем их, но не
    // дольше срока: закрывающего кадра можно и не дождаться вовсе
    let drain = async {
        while let Some(Ok(message)) = socket.next().await {
            if let Message::Text(report) = message {
                rate = throughput(&report).or(rate);
                if let Some(rate) = rate {
                    rates.set(stream, rate);
                }
            }
        }
    };
    let _ = tokio::time::timeout(SILENCE_LIMIT, drain).await;

    rate.ok_or_else(|| Trouble::Broken(tr!("speedtest.error.noBytes")))
}

/// Отчёт сервера о соединении. Полей там много, нужны три.
#[derive(Deserialize)]
struct Ndt7Report {
    #[serde(rename = "TCPInfo")]
    tcp_info: Option<TcpInfo>,
}

#[derive(Deserialize)]
struct TcpInfo {
    /// Байты, дошедшие до сервера.
    #[serde(rename = "BytesReceived")]
    bytes_received: Option<u64>,
    /// Микросекунды с начала соединения.
    #[serde(rename = "ElapsedTime")]
    elapsed: Option<u64>,
    /// Микросекунды.
    #[serde(rename = "MinRTT")]
    min_rtt: Option<u64>,
}

fn tcp_info(report: &str) -> Option<TcpInfo> {
    serde_json::from_str::<Ndt7Report>(report).ok()?.tcp_info
}

/// Время оборота, каким его видит сам сервер: это настоящий RTT из TCP, а не
/// время HTTP-запроса с рукопожатием внутри.
fn min_rtt(report: &str) -> Option<f64> {
    let rtt = tcp_info(report)?.min_rtt?;
    (rtt > 0).then(|| rtt as f64 / 1000.0)
}

/// Отдача по счётчикам сервера.
fn throughput(report: &str) -> Option<f64> {
    let info = tcp_info(report)?;
    let bytes = info.bytes_received?;
    let elapsed = info.elapsed?;

    (elapsed > 0).then(|| megabits(bytes, Duration::from_micros(elapsed)))
}

/// Клиент только для замера, общий с телеметрией не годится: там таймаут
/// в доли секунды.
///
/// ВАЖНО: только HTTP/1.1. По ALPN с Cloudflare согласуется HTTP/2, а у него
/// своё окно управления потоком поверх окна TCP, и на задержке в 29 мс оно
/// упирает скачивание в 91 Мбит/с при живых 460. Поймалось сверкой с curl,
/// который HTTP/2 просто не умеет и показывал впятеро больше.
fn client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .http1_only()
        .build()
}

/// Байты за время в мегабиты в секунду.
///
/// Мегабит здесь ровно миллион бит, а не 2²⁰: скорость канала везде считают
/// в десятичных, и расхождение с любым спидтестом было бы в семь процентов.
fn megabits(bytes: u64, elapsed: Duration) -> f64 {
    let seconds = elapsed.as_secs_f64();
    if seconds <= 0.0 {
        return 0.0;
    }
    bytes as f64 * 8.0 / seconds / 1_000_000.0
}

/// Причина отказа человеческими словами.
///
/// Строку от reqwest показывать нельзя: в ней полный адрес запроса, и она
/// длиннее карточки. Свои держим короткими: место у них — одна строка под
/// кнопкой, и переносом она сдвинет всё, что ниже.
///
/// На выбранном языке, как и весь текст карточки; русскими остаются только
/// сообщения в лог — их читает разработчик, а не пользователь.
fn describe(error: reqwest::Error) -> String {
    if error.is_timeout() {
        return tr!("speedtest.error.timeout");
    }
    if error.is_connect() {
        return tr!("speedtest.error.offline");
    }

    match error.status().map(|status| status.as_u16()) {
        Some(429) => tr!("speedtest.error.tooMany"),
        Some(403) => tr!("speedtest.error.cloudflare"),
        Some(status) => tr!("speedtest.error.status", status = status),
        None => tr!("speedtest.error.dropped"),
    }
}

fn publish(app: &AppHandle, view: View) {
    let Some(state) = app.try_state::<Speedtest>() else {
        return;
    };
    state.set(view);

    // Неудачу рассылки глотаем: окно могли закрыть прямо в этот момент
    let _ = app.emit(SPEEDTEST_EVENT, state.snapshot());
}

/// Кладёт замер в историю и рассылает её вместе с готовым результатом.
///
/// Только целый замер: у оборванного нет отдачи, и столбик по нему врал бы о
/// канале наравне с настоящими.
fn remember(app: &AppHandle, report: Report) {
    let Some(state) = app.try_state::<Speedtest>() else {
        return;
    };

    let history = state.remember(report);
    save_history(app, &history);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Отказ по токену обязан отличаться от прочих: на нём замер повторяют с
    /// новым подбором, а не показывают отказ. Раньше включённый VPN давал 401
    /// и запертую на минуту кнопку.
    #[test]
    fn a_refused_pass_is_told_apart_from_a_dead_server() {
        assert!(stale_status(401));
        assert!(stale_status(403));

        // Всё прочее лечится не подбором: 500 — беда на самом сервере,
        // 429 — просьба подождать
        assert!(!stale_status(500));
        assert!(!stale_status(429));
        assert!(!stale_status(200));

        assert!(Trouble::Stale("токен".to_owned()).stale());
        assert!(!Trouble::Broken("connection dropped".to_owned()).stale());
    }

    /// Время оборота у канала одно, а меряют его несколько потоков: берём
    /// лучшее из измеренных, оно ближе всего к правде.
    #[test]
    fn the_quickest_round_trip_wins() {
        assert_eq!(quicker(Some(12.0), Some(8.0)), Some(8.0));
        assert_eq!(quicker(None, Some(8.0)), Some(8.0));
        assert_eq!(quicker(Some(12.0), None), Some(12.0));
        assert_eq!(quicker(None, None), None);
    }

    /// Причиной отказа остаётся первая беда — кроме протухшего токена: на нём
    /// замер повторяют, и потерять его среди «связь оборвалась» нельзя.
    #[test]
    fn a_refused_pass_outweighs_the_rest() {
        let broken = keep(None, Trouble::Broken("connection dropped".to_owned()));
        assert!(!broken.as_ref().expect("беда есть").stale());

        let with_stale = keep(broken, Trouble::Stale("пропуск".to_owned()));
        assert!(with_stale.as_ref().expect("беда есть").stale());

        // А обратно — нет: за протухшим токеном следующая беда уже неважна
        let after = keep(with_stale, Trouble::Broken("ещё одна".to_owned()));
        assert!(after.expect("беда есть").stale());
    }

    /// Счёт общий на все потоки: скорость складывается из них разом.
    #[test]
    fn the_streams_add_up() {
        let flow = Flow::new(2);
        assert_eq!(flow.rate(), None, "до первых данных скорости нет");

        flow.begin();
        flow.add(1_000_000);
        flow.add(1_000_000);

        let rate = flow.rate().expect("скорость посчиталась");
        assert!(rate > 0.0, "вышло {rate}");

        assert!(flow.running());
        flow.done();
        flow.done();
        assert!(!flow.running(), "все потоки кончились");
    }

    /// Часы замера кончаются последним принятым байтом, а не последним
    /// взглядом на них: итог считается уже после сна цикла показа, и от
    /// «сейчас» скорость выходила бы ниже настоящей.
    #[test]
    fn the_clock_stops_with_the_last_byte() {
        let flow = Flow::new(1);
        flow.begin();
        flow.add(1_000_000);

        let measured = flow.elapsed().expect("часы пущены");
        std::thread::sleep(Duration::from_millis(30));

        assert_eq!(flow.elapsed(), Some(measured), "часы шли и после приёма");
    }

    fn server(url: &str) -> Server {
        Server {
            city: "Франкфурт".to_owned(),
            country: "DE".to_owned(),
            download: url.to_owned(),
            upload: url.to_owned(),
        }
    }

    /// Пара хабов выбирается по тому, где мы вышли. Живьём это проверяется
    /// только сменой выходного узла VPN, то есть никогда.
    #[test]
    fn the_hubs_follow_where_we_came_out() {
        assert_eq!(hubs_for(Some("US")), AMERICA);
        assert_eq!(hubs_for(Some("CA")), AMERICA);

        // Европа — не «правильный ответ», а безопасный: до её хабов идёт всё
        // остальное. Ирландия сюда попадает не потому, что она хуже, а потому,
        // что своей пары у неё нет
        assert_eq!(hubs_for(Some("IE")), EUROPE);
        assert_eq!(hubs_for(Some("DE")), EUROPE);

        // Тот самый случай, ради которого хабы и заводили: выход VPN в Катаре
        assert_eq!(hubs_for(Some("QA")), EUROPE);

        // Подбор не ответил — идём прежним путём, а не отказываемся от замера
        assert_eq!(hubs_for(None), EUROPE);
    }

    /// Записанная выдача подбора: страна нужна первому запросу, город —
    /// карточке, а площадка без нужных ссылок в замер не годится вовсе.
    #[test]
    fn a_recorded_answer_reads_into_servers() {
        let answer: Answer = serde_json::from_str(
            r#"{"results":[
                {"location":{"city":"Dublin","country":"IE"},
                 "urls":{"wss:///ndt/v7/download":"wss://mlab2-dub01.example/d?t=1",
                         "wss:///ndt/v7/upload":"wss://mlab2-dub01.example/u?t=1"}},
                {"location":{"city":"London","country":"GB"},
                 "urls":{"wss:///ndt/v7/download":"wss://mlab3-lhr07.example/d?t=2",
                         "wss:///ndt/v7/upload":"wss://mlab3-lhr07.example/u?t=2"}},
                {"location":{"city":"Ниоткуда","country":"XX"},
                 "urls":{"ws:///ndt/v7/download":"ws://plain.example/d"}}
            ]}"#,
        )
        .expect("ответ разбирается");

        let servers = read_servers(answer.results);

        assert_eq!(servers.len(), 2, "площадка без wss в замер не годится");
        assert_eq!(servers[0].city, "Dublin");
        assert_eq!(servers[0].country, "IE");
        assert_eq!(servers[1].country, "GB");
    }

    /// Места в ответе может не быть вовсе — это не повод падать и не повод
    /// показать пустоту вместо города.
    #[test]
    fn a_server_without_a_place_still_counts() {
        let answer: Answer = serde_json::from_str(
            r#"{"results":[{"urls":{
                "wss:///ndt/v7/download":"wss://mlab1-fra01.example/d?t=1",
                "wss:///ndt/v7/upload":"wss://mlab1-fra01.example/u?t=1"}}]}"#,
        )
        .expect("ответ разбирается");

        let servers = read_servers(answer.results);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].city, "unknown");
        assert_eq!(servers[0].country, "");
    }

    /// Одна площадка приходит из двух стран сразу: у Германии в выдаче Париж,
    /// у Нидерландов Брюссель. Два соединения к ней — это не восемь потоков.
    #[test]
    fn the_same_machine_is_taken_once() {
        let servers = vec![
            server("wss://mlab1-fra01.mlab-oti.measurement-lab.org/ndt/v7/download?access_token=1"),
            // Та же машина, но токен в ссылке другой — целиком строки не равны
            server("wss://mlab1-fra01.mlab-oti.measurement-lab.org/ndt/v7/download?access_token=2"),
            server("wss://mlab2-ams02.mlab-oti.measurement-lab.org/ndt/v7/download?access_token=3"),
        ];

        let kept = without_repeats(servers);
        assert_eq!(kept.len(), 2, "повтор площадки не убрался");
        assert!(kept[1].download.contains("ams02"));
    }

    #[test]
    fn the_machine_is_the_host_part() {
        assert_eq!(
            machine_of("wss://mlab1-fra01.measurement-lab.org/ndt/v7/download?access_token=x"),
            "mlab1-fra01.measurement-lab.org"
        );
        // Мусор вместо ссылки не должен ронять подбор
        assert_eq!(machine_of("совсем не ссылка"), "совсем не ссылка");
    }

    #[test]
    fn megabits_are_decimal() {
        // 100 МБ за 8 секунд — это 100 Мбит/с
        assert!((megabits(100_000_000, Duration::from_secs(8)) - 100.0).abs() < 1e-9);
    }

    /// Ноль времени случается на пустом ответе — делить на него нельзя.
    #[test]
    fn zero_time_gives_zero_speed() {
        assert_eq!(megabits(1000, Duration::ZERO), 0.0);
    }

    /// Разметка уходит на фронт как есть: snake_case там уже однажды всплывал.
    #[test]
    fn view_arrives_in_camel_case() {
        let json = serde_json::to_string(&View::Running {
            stage: Stage::Download,
            report: Report {
                download: Some(452.8),
                upload: None,
                latency: Some(12.0),
            },
        })
        .expect("сериализуется");

        assert!(json.contains(r#""status":"running""#), "{json}");
        assert!(json.contains(r#""stage":"download""#), "{json}");
        assert!(json.contains(r#""upload":null"#), "{json}");
    }

    #[test]
    fn idle_view_has_no_payload() {
        let json = serde_json::to_string(&View::default()).expect("сериализуется");
        assert_eq!(json, r#"{"status":"idle"}"#);
    }

    /// `flatten` поверх размеченного перечисления легко сломать переносом
    /// поля: `status` обязан остаться на верхнем уровне рядом с историей,
    /// иначе фронт увидит состояние без имени.
    #[test]
    fn snapshot_keeps_status_beside_history() {
        let json = serde_json::to_string(&Snapshot {
            view: View::Done {
                report: Report {
                    download: Some(452.8),
                    upload: Some(178.1),
                    latency: Some(12.0),
                },
            },
            history: vec![Report::default()],
            server: Some("Грац".to_owned()),
        })
        .expect("сериализуется");

        assert!(json.contains(r#""status":"done""#), "{json}");
        assert!(json.contains(r#""history":[{"#), "{json}");
        assert!(!json.contains(r#""view""#), "вложенного view быть не должно");
    }

    fn measurement(download: f64) -> Report {
        Report {
            download: Some(download),
            upload: Some(100.0),
            latency: Some(12.0),
        }
    }

    /// История не растёт бесконечно, и уходит из неё самое старое.
    #[test]
    fn history_keeps_only_the_latest() {
        let state = Speedtest::default();

        for step in 0..(HISTORY_LIMIT + 3) {
            state.remember(measurement(step as f64));
        }

        let history = state.snapshot().history;
        assert_eq!(history.len(), HISTORY_LIMIT);
        assert_eq!(history[0].download, Some(3.0), "старейшие должны уйти");
        assert_eq!(
            history[HISTORY_LIMIT - 1].download,
            Some((HISTORY_LIMIT + 2) as f64)
        );
    }

    /// Файл правят руками, и длинный список превратился бы в столбики
    /// шириной в волос.
    #[test]
    fn loaded_history_is_trimmed_too() {
        let long: Vec<Report> = (0..40).map(|step| measurement(step as f64)).collect();
        let state = Speedtest::new(long);

        assert_eq!(state.snapshot().history.len(), HISTORY_LIMIT);
    }

    #[test]
    fn history_survives_a_round_trip() {
        let history = vec![measurement(452.8), measurement(431.2)];
        let text = serde_json::to_string(&history).expect("сериализуется");
        let read: Vec<Report> = serde_json::from_str(&text).expect("читается обратно");

        assert_eq!(read.len(), 2);
        assert_eq!(read[0].download, Some(452.8));
        assert_eq!(read[1].upload, Some(100.0));
    }

    /// Оборванная запись или правка руками не должны мешать запуску.
    #[test]
    fn broken_history_reads_as_empty() {
        let read: Vec<Report> = serde_json::from_str("[{").unwrap_or_default();
        assert!(read.is_empty());
    }

    /// Сквозная проверка на живой сети. Ест трафик, поэтому под флагом.
    ///
    /// Запуск: `cargo test --manifest-path src-tauri/Cargo.toml -- --ignored`
    #[tokio::test]
    #[ignore = "нужна работающая сеть, тратит трафик"]
    async fn measures_the_real_channel() {
        let found = locate().await.expect("M-Lab отвечает");
        assert!(!found.is_empty(), "сервер не нашёлся");

        // Континенты в одном замере смешивать нельзя: поток, ушедший за океан,
        // потянет средний результат вниз. Совпадения со списком пары при этом
        // не требуем — запрос по стране штатно отдаёт и соседей: Париж к
        // Германии, Брюссель к Нидерландам, и это те же хабы
        let hubs = hubs_for(found.first().map(|server| server.country.as_str()));
        let other = if hubs == AMERICA { EUROPE } else { AMERICA };
        for server in &found {
            assert!(
                !other.contains(&server.country.as_str()),
                "{} ({}) с чужого материка: меряем через {hubs:?}",
                server.city,
                server.country
            );
        }

        // Столько же потоков, сколько в живом замере: одиночный поток меряет
        // не канал, а один сокет, и цифры отсюда сверяют со сторонними
        let servers: Vec<Server> = found.into_iter().take(STREAMS).collect();

        let down = download(&servers, |_| {}).await.expect("скачивание не прошло");
        assert!(
            down.rate > 1.0,
            "скорость подозрительно мала: {} Мбит/с",
            down.rate
        );

        let latency = down.latency.expect("сервер отчитался о времени оборота");
        assert!(
            (0.5..500.0).contains(&latency),
            "неправдоподобная задержка: {latency} мс"
        );

        // Ход отдачи тут никому не нужен: тест смотрит на итог
        let up = upload(&servers, |_| {})
            .await
            .expect("отдача не прошла");
        assert!(up > 0.5, "отдача подозрительно мала: {up} Мбит/с");

        // Тест ручной, поэтому числа печатаем: сверять их глазами с любым
        // сторонним спидтестом — единственный способ поймать грубую ошибку
        // в арифметике, которую утверждения не заметят
        eprintln!(
            "{} и ещё {}: {:.1} мс, скачивание {:.1}, отдача {:.1} Мбит/с",
            servers[0].city,
            servers.len() - 1,
            latency,
            down.rate,
            up
        );
    }
}
