//! Пинг: три внешних цели одновременно.
//!
//! Цели независимы друг от друга — Cloudflare, Google и OpenDNS держат свои
//! сети и свои стыки. Просевшая одна означает беду у неё, просевшие все три —
//! беду у нас. Прежняя пара «интернет и шлюз» отвечала на тот же вопрос, но
//! ценой второй стороны, у которой и пороги, и смысл были свои.
//!
//! Меряем две вещи: время отклика и джиттер. Первое отвечает «далеко ли»,
//! второе — «ровно ли», и рвутся разговор с игрой именно от второго.

mod icmp;

use std::collections::VecDeque;
use std::net::Ipv4Addr;
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

/// Кого пингуем: имя оператора и его адрес.
///
/// Имя лежит рядом с адресом, а не отдельной картой на фронте: два списка,
/// которые надо держать в одном порядке, однажды разъедутся молча. В карточке
/// стоит имя — «208.67.222.222» вдвое длиннее соседей и разорвал бы колонки, —
/// а адрес остаётся подсказкой.
///
/// Все три держат постоянный anycast-адрес: один и тот же адрес поднят в сотне
/// городов, отвечает ближайший, и разрешать имя не нужно — задержка DNS в
/// замер не попадает.
const TARGETS: [(&str, Ipv4Addr); 3] = [
    ("Cloudflare", Ipv4Addr::new(1, 1, 1, 1)),
    ("Google", Ipv4Addr::new(8, 8, 8, 8)),
    ("OpenDNS", Ipv4Addr::new(208, 67, 222, 222)),
];

const PROBE_INTERVAL: Duration = Duration::from_secs(2);

/// Ждать дольше нет смысла: такой отклик уже означает «связи нет».
const TIMEOUT: Duration = Duration::from_millis(1500);

/// По скольким последним попыткам считаются потери и джиттер.
const WINDOW: usize = 10;

/// Событие, которым замер расходится по окнам.
const PING_EVENT: &str = "ping";

/// Что намеряли по цели.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reading {
    /// Время отклика в миллисекундах. `null` — не ответил.
    pub latency: Option<f64>,
    /// Разброс соседних откликов в миллисекундах. `null` — ответов пока мало.
    pub jitter: Option<f64>,
    /// Доля потерь в последних попытках, от нуля до единицы.
    pub loss: f64,
}

/// Одна цель.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    /// Имя оператора — оно и стоит в карточке.
    pub name: &'static str,
    /// Адрес: живёт в подсказке.
    pub target: String,
    /// `None` — первой попытки ещё не было. Это не то же самое, что «не
    /// ответил»: там прочерк красный, здесь просто нечего показывать.
    pub reading: Option<Reading>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ping {
    pub probes: Vec<Probe>,
}

impl Default for Ping {
    /// До первого замера цели уже названы: иначе карточка на секунду
    /// оставалась бы пустой, а потом раздвигалась под приехавшие имена.
    fn default() -> Self {
        Self {
            probes: TARGETS
                .iter()
                .map(|(name, address)| Probe {
                    name,
                    target: address.to_string(),
                    reading: None,
                })
                .collect(),
        }
    }
}

/// Последний замер. Нужен окну, открытому между тиками: иначе оно ждало бы
/// данных до следующего опроса.
#[derive(Default)]
pub struct PingState(Mutex<Ping>);

#[tauri::command]
pub fn ping_state(state: State<'_, PingState>) -> Ping {
    // Отравленный мьютекс не повод падать: данные внутри осмысленные
    state
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Окно последних попыток — из него считаются потери и джиттер.
///
/// ВАЖНО: **окно одно на оба показателя, а не по своему на каждый.** Прежде
/// ответы жили отдельной очередью, и вытеснялась она только приходом нового
/// ответа: при полном обрыве связи потери честно показывали сто процентов, а
/// рядом стоял разброс, посчитанный по ответам до аварии, — час назад, если
/// обрыв длился час. Первый же ответ после восстановления сравнивался с
/// замером из прошлой жизни и давал скачок, которого не было.
#[derive(Default)]
struct Attempts {
    /// `None` — пакет не вернулся. Потерянный пакет своего времени не имеет и
    /// в счёт разброса не входит, но место в окне занимает наравне с ответом.
    tries: VecDeque<Option<f64>>,
}

impl Attempts {
    fn record(&mut self, latency: Option<f64>) {
        if self.tries.len() == WINDOW {
            self.tries.pop_front();
        }
        self.tries.push_back(latency);
    }

    /// Доля неотвеченных. Пустое окно — потерь нет.
    fn loss(&self) -> f64 {
        if self.tries.is_empty() {
            return 0.0;
        }
        let lost = self.tries.iter().filter(|try_| try_.is_none()).count();
        lost as f64 / self.tries.len() as f64
    }

    /// Средний модуль разности соседних откликов.
    ///
    /// Считаем именно так, а не разбросом вокруг среднего: слышно и видно
    /// скачок между соседними пакетами, а не то, насколько ряд в целом далёк
    /// от своей середины. Медленно уползающий отклик джиттером не является.
    ///
    /// Меньше двух ответов в окне — считать нечего, и в карточке будет
    /// прочерк. Пропавшая связь так и гасит разброс сама: ответы уходят из
    /// окна вместе со всем прочим.
    fn jitter(&self) -> Option<f64> {
        let mut previous: Option<f64> = None;
        let mut sum = 0.0;
        let mut pairs = 0_usize;

        for answer in self.tries.iter().flatten() {
            if let Some(before) = previous {
                sum += (answer - before).abs();
                pairs += 1;
            }
            previous = Some(*answer);
        }

        (pairs > 0).then(|| sum / pairs as f64)
    }
}

/// Запускает опрос. Живёт до конца работы приложения.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut windows: [Attempts; TARGETS.len()] =
            std::array::from_fn(|_| Attempts::default());

        let mut ticker = tokio::time::interval(PROBE_INTERVAL);
        // Затянувшийся замер не повод догонять пропущенные тики пачкой
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            ticker.tick().await;

            // Пинг нужен одной карточке сети: выключен модуль — пакеты не шлём.
            // Задача при этом живёт: включат сеть снова — продолжит с того же
            if !crate::modules::on(&app, "network") {
                continue;
            }

            // Все пакеты уходят разом: последовательно три неудачи заняли бы
            // четыре с половиной секунды при опросе раз в две
            let tasks: Vec<_> = TARGETS.iter().map(|(_, address)| measure(*address)).collect();

            let mut probes = Vec::with_capacity(TARGETS.len());
            for (index, task) in tasks.into_iter().enumerate() {
                // Сорванная задача — это «ответа нет», а не повод падать
                let latency = task.await.unwrap_or(None);

                let attempts = &mut windows[index];
                attempts.record(latency);

                let (name, address) = TARGETS[index];
                probes.push(Probe {
                    name,
                    target: address.to_string(),
                    reading: Some(Reading {
                        latency,
                        jitter: attempts.jitter(),
                        loss: attempts.loss(),
                    }),
                });
            }

            publish(&app, Ping { probes });
        }
    });
}

/// Один замер. Вызов ICMP блокирующий, поэтому уходит в отдельный поток.
fn measure(target: Ipv4Addr) -> tauri::async_runtime::JoinHandle<Option<f64>> {
    tauri::async_runtime::spawn_blocking(move || icmp::echo(target, TIMEOUT))
}

fn publish(app: &AppHandle, state: Ping) {
    if let Some(current) = app.try_state::<PingState>() {
        let mut stored = current
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *stored = state.clone();
    }

    // Неудачу рассылки глотаем: окно могли закрыть прямо в этот момент
    let _ = app.emit(PING_EVENT, state);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_window_has_no_loss() {
        assert_eq!(Attempts::default().loss(), 0.0);
    }

    #[test]
    fn loss_counts_only_the_last_attempts() {
        let mut attempts = Attempts::default();

        // Одна потеря из десяти — десять процентов
        attempts.record(None);
        for _ in 0..9 {
            attempts.record(Some(10.0));
        }
        assert!((attempts.loss() - 0.1).abs() < 1e-9);

        // Одиннадцатая попытка вытесняет первую, и потеря уходит из окна
        attempts.record(Some(10.0));
        assert_eq!(attempts.loss(), 0.0);
    }

    /// Ровный канал джиттера не имеет — иначе цифра пугала бы на пустом месте.
    #[test]
    fn a_steady_line_has_no_jitter() {
        let mut attempts = Attempts::default();
        for _ in 0..5 {
            attempts.record(Some(30.0));
        }
        assert_eq!(attempts.jitter(), Some(0.0));
    }

    /// Пила из соседних замеров — это и есть джиттер, и он равен размаху.
    #[test]
    fn a_sawtooth_is_all_jitter() {
        let mut attempts = Attempts::default();
        for step in 0..4 {
            attempts.record(Some(if step % 2 == 0 { 30.0 } else { 40.0 }));
        }
        assert_eq!(attempts.jitter(), Some(10.0));
    }

    /// Медленный уход отклика джиттером не является: соседи отличаются мало,
    /// хотя первый и последний замеры далеки друг от друга.
    #[test]
    fn a_slow_drift_is_not_jitter() {
        let mut attempts = Attempts::default();
        for step in 0..10 {
            attempts.record(Some(30.0 + step as f64));
        }
        assert_eq!(attempts.jitter(), Some(1.0));
    }

    /// Одного ответа для разброса мало, и выдумывать ноль тут нельзя: ноль
    /// означал бы идеально ровный канал.
    #[test]
    fn one_answer_is_not_enough_for_jitter() {
        let mut attempts = Attempts::default();
        assert_eq!(attempts.jitter(), None);

        attempts.record(Some(30.0));
        assert_eq!(attempts.jitter(), None);

        attempts.record(Some(31.0));
        assert_eq!(attempts.jitter(), Some(1.0));
    }

    /// Потерянный пакет своего времени не имеет: он не должен ни обнулять
    /// разброс, ни считаться нулевым откликом.
    #[test]
    fn a_lost_packet_leaves_no_trace_in_the_jitter() {
        let mut attempts = Attempts::default();
        attempts.record(Some(30.0));
        attempts.record(None);
        attempts.record(Some(30.0));

        assert_eq!(attempts.jitter(), Some(0.0));
        assert!((attempts.loss() - 1.0 / 3.0).abs() < 1e-9);
    }

    /// Связь пропала совсем — разброс обязан погаснуть вместе с откликом.
    /// Число, снятое до аварии, рядом со стопроцентными потерями — это враньё
    /// ровно в том месте, ради которого на карточку и смотрят.
    #[test]
    fn a_dead_link_puts_the_jitter_out() {
        let mut attempts = Attempts::default();
        for _ in 0..WINDOW {
            attempts.record(Some(20.0));
        }
        assert_eq!(attempts.jitter(), Some(0.0));

        for _ in 0..WINDOW {
            attempts.record(None);
        }

        assert_eq!(attempts.loss(), 1.0);
        assert_eq!(attempts.jitter(), None, "разброс пережил обрыв связи");
    }

    /// Первый ответ после обрыва сравнивать не с чем: всё, что было до
    /// аварии, из окна уже вышло.
    #[test]
    fn the_first_answer_after_an_outage_stands_alone() {
        let mut attempts = Attempts::default();
        attempts.record(Some(20.0));
        for _ in 0..WINDOW {
            attempts.record(None);
        }

        attempts.record(Some(80.0));

        assert_eq!(attempts.jitter(), None, "разброс посчитан через обрыв");
    }

    /// Разметка уходит на фронт как есть: snake_case там уже однажды всплывал.
    #[test]
    fn probes_arrive_in_camel_case() {
        let json = serde_json::to_string(&Ping {
            probes: vec![Probe {
                name: "Cloudflare",
                target: "1.1.1.1".to_owned(),
                reading: Some(Reading {
                    latency: Some(12.5),
                    jitter: Some(1.25),
                    loss: 0.0,
                }),
            }],
        })
        .expect("сериализуется");

        assert!(json.contains(r#""name":"Cloudflare""#), "{json}");
        assert!(json.contains(r#""target":"1.1.1.1""#), "{json}");
        assert!(json.contains(r#""latency":12.5"#), "{json}");
        assert!(json.contains(r#""jitter":1.25"#), "{json}");
    }

    /// До замера цели уже названы, а показаний у них нет: карточка не должна
    /// раздвигаться под приехавшие имена.
    #[test]
    fn targets_are_named_before_the_first_probe() {
        let initial = Ping::default();
        assert_eq!(initial.probes.len(), TARGETS.len());

        for probe in &initial.probes {
            assert!(probe.reading.is_none(), "показание взялось до замера");
            assert!(!probe.name.is_empty());
        }

        let json = serde_json::to_string(&initial).expect("сериализуется");
        assert!(json.contains(r#""reading":null"#), "{json}");
    }

    /// Имена и адреса не повторяются: одна и та же цель дважды означала бы
    /// колонку, которая ничего не добавляет.
    #[test]
    fn targets_are_all_different() {
        for (index, (name, address)) in TARGETS.iter().enumerate() {
            for (other_name, other_address) in TARGETS.iter().skip(index + 1) {
                assert_ne!(name, other_name, "имя «{name}» занято дважды");
                assert_ne!(address, other_address, "адрес {address} занят дважды");
            }
        }
    }

    /// Сквозная проверка на живой сети: все три цели отвечают за отведённое
    /// время и правдоподобно.
    ///
    /// Запуск: `cargo test --manifest-path src-tauri/Cargo.toml -- --ignored`
    #[test]
    #[ignore = "нужна работающая сеть"]
    fn reaches_the_real_network() {
        for (name, address) in TARGETS {
            let latency = icmp::echo(address, TIMEOUT)
                .unwrap_or_else(|| panic!("{name} ({address}) не ответил"));

            assert!(
                (0.1..500.0).contains(&latency),
                "неправдоподобный отклик у {name}: {latency} мс"
            );
        }
    }
}
