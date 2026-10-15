//! Буфер обмена: последние двадцать скопированных строк.
//!
//! Системная история (Win+V) живёт всплывающим окном, которое надо вызывать и
//! которое открывается поверх работы. Виджет держит те же строки на виду
//! постоянно, и вернуть любую в буфер — один щелчок.
//!
//! Устройство то же, что у звука: фоновый поток опрашивает систему, шлёт
//! событие в окно и принимает команды каналом. Причина другая — COM тут ни при
//! чём, буфер это user32, — но правило то же: `OpenClipboard` ждёт чужое
//! приложение, а приём IPC у WebView2 идёт главным потоком.
//!
//! В этом файле поэтому нет ни одного вызова Windows: только состояние,
//! команды и правила, которые можно проверить тестом.

#[cfg(windows)]
mod board;

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::oneshot;

use crate::settings::SettingsState;
use crate::widgets;

/// Метка окна виджета: по ней находим и настройку, и владельца буфера.
const WIDGET: &str = "clipboard";

/// Событие, которым список расходится по окнам.
const CLIPBOARD_EVENT: &str = "clipboard";

/// Шаг опроса. За полсекунды успевает смениться разве что содержимое, которого
/// никто не собирался копировать.
const STEP: Duration = Duration::from_millis(500);

/// Сколько записей помним.
const CAPACITY: usize = 20;

/// Длина строки для показа. Куда больше, чем влезает в окно шириной 440 точек:
/// остаток достаётся подсказке под курсором.
const PREVIEW_CHARS: usize = 200;

/// Потолок длины записи.
///
/// Правило то же, что у спидтеста: в историю идёт только то, что можно вернуть
/// целиком. Обрезанная запись при копировании обратно соврала бы молча, а
/// двадцать таких держали бы в памяти неизвестно сколько.
const MAX_CHARS: usize = 64 * 1024;

/// Ищет конец текста до декодирования, читая не больше двух единиц на знак
/// и завершающего нуля. Нет нуля в пределах — строку целиком не берём.
fn bounded_text_length(wide: &[u16]) -> Option<usize> {
    wide.iter().take(MAX_CHARS * 2 + 1).position(|unit| *unit == 0)
}

/// Запись, какой она лежит у нас: с полным текстом.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    id: u64,
    text: String,
}

/// Запись, какой её видит окно.
///
/// Полного текста тут нет намеренно: двадцать записей по 64 КБ — это мегабайт
/// на каждое событие, а нужен он только для возврата в буфер, то есть на этой
/// стороне.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Shown {
    pub id: u64,
    /// Одна строка для показа: пробелы сжаты, длина обрезана.
    pub preview: String,
    /// Ограниченный предпросмотр GPT с исходными переводами строк.
    pub formatted_preview: String,
    /// Строки исходной записи, в том числе пустые; CRLF — один перевод.
    pub lines: usize,
    /// Сколько знаков в записи целиком — для подсказки под курсором.
    pub chars: usize,
    /// Эта запись лежит в буфере прямо сейчас.
    pub current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    /// Свежая первой.
    pub entries: Vec<Shown>,
    /// Сколько записей помещается всего — для счётчика в подвале.
    pub capacity: usize,
}

impl Default for View {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            capacity: CAPACITY,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Action {
    /// Вернуть запись в буфер обмена.
    Copy { id: u64 },
    /// Убрать запись из списка. Самого буфера это не касается.
    Remove { id: u64 },
    /// Забыть всё и опустошить сам буфер.
    Clear,
}

/// Список записей.
///
/// Вся логика этого виджета живёт тут, и вся она проверяется тестом: Windows
/// нужна только чтобы узнать, что содержимое сменилось, и прочитать его.
#[derive(Debug, Default)]
struct History {
    entries: VecDeque<Entry>,
    /// Какая запись лежит в буфере прямо сейчас.
    current: Option<u64>,
    next_id: u64,
}

impl History {
    /// Записывает свежее содержимое буфера. Возвращает «список изменился».
    fn record(&mut self, text: &str) -> bool {
        // Пустое и пробельное — это не запись, а промах мимо нужного текста
        if text.trim().is_empty() {
            return false;
        }
        if text.chars().count() > MAX_CHARS {
            return false;
        }

        // Тот же текст, скопированный ещё раз, переезжает наверх, а не ложится
        // второй строкой: он и правда самый свежий
        if let Some(at) = self.entries.iter().position(|entry| entry.text == text) {
            let entry = self.entries.remove(at).expect("позиция только что найдена");
            self.current = Some(entry.id);
            self.entries.push_front(entry);
            return true;
        }

        let id = self.next_id;
        self.next_id += 1;

        self.entries.push_front(Entry {
            id,
            text: text.to_owned(),
        });
        self.entries.truncate(CAPACITY);
        self.current = Some(id);

        true
    }

    fn text_of(&self, id: u64) -> Option<String> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.text.clone())
    }

    /// Отмечает запись как лежащую в буфере.
    fn mark(&mut self, id: u64) -> bool {
        if self.current == Some(id) {
            return false;
        }
        self.current = Some(id);
        true
    }

    /// Убирает одну запись.
    ///
    /// Обратно она не всплывёт: в список мы кладём только на смену номера
    /// последовательности, а он от удаления не меняется.
    fn remove(&mut self, id: u64) -> bool {
        let Some(at) = self.entries.iter().position(|entry| entry.id == id) else {
            return false;
        };

        self.entries.remove(at);
        // Убрали ту, что в буфере: пометке некуда деться, но опустошать сам
        // буфер из-за одной строки списка было бы слишком
        if self.current == Some(id) {
            self.current = None;
        }

        true
    }

    fn clear(&mut self) -> bool {
        if self.entries.is_empty() && self.current.is_none() {
            return false;
        }

        self.entries.clear();
        self.current = None;
        true
    }

    fn view(&self) -> View {
        View {
            entries: self
                .entries
                .iter()
                .map(|entry| Shown {
                    id: entry.id,
                    preview: preview(&entry.text),
                    formatted_preview: formatted_preview(&entry.text),
                    lines: line_count(&entry.text),
                    chars: entry.text.chars().count(),
                    current: self.current == Some(entry.id),
                })
                .collect(),
            capacity: CAPACITY,
        }
    }
}

/// Строка для показа: пробельные последовательности сжаты в один пробел,
/// длина обрезана.
///
/// Переводы строк съедаются намеренно: скопированный кусок кода иначе показал
/// бы в строке свой отступ и ничего больше.
fn preview(text: &str) -> String {
    let mut shown = String::new();
    let mut length = 0;

    for word in text.split_whitespace() {
        // Пробел между словами занимает место в отведённой длине наравне с
        // буквами, иначе обрезка промахивалась бы на число слов.
        //
        // Места ему нужно на знак больше, чем самому пробелу: занявший
        // последнее место, он остаётся висеть на конце — следующее слово в
        // строку уже не влезет ни одной буквой
        if length > 0 {
            if length + 1 >= PREVIEW_CHARS {
                break;
            }
            shown.push(' ');
            length += 1;
        }

        for character in word.chars() {
            if length == PREVIEW_CHARS {
                break;
            }
            shown.push(character);
            length += 1;
        }
    }

    shown
}

/// GPT сохраняет переносы и отступы только в ограниченной проекции.
/// Полный текст по-прежнему остаётся в History, а Claude читает прежний preview.
fn formatted_preview(text: &str) -> String {
    let mut source = text.chars().peekable();
    let mut shown = String::new();
    for _ in 0..PREVIEW_CHARS {
        let Some(character) = source.next() else { break; };
        if character == '\r' {
            if source.peek() == Some(&'\n') { source.next(); }
            shown.push('\n');
        } else {
            shown.push(character);
        }
    }
    shown
}

fn line_count(text: &str) -> usize {
    let mut lines = 1;
    let mut previous_cr = false;
    for character in text.chars() {
        if character == '\r' || (character == '\n' && !previous_cr) { lines += 1; }
        previous_cr = character == '\r';
    }
    lines
}

/// Короткий код отказа; текст для пользователя берёт фронт из каталога.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ControlError {
    Unavailable,
    Missing,
    CopyFailed,
    ClearFailed,
}

struct Order {
    action: Action,
    reply: oneshot::Sender<Result<(), ControlError>>,
}

struct Outcome {
    changed: bool,
    result: Result<(), ControlError>,
}

/// Возвращает запись через переданный writer: тесты используют только демоданные.
/// Замок отпущен до ожидания буфера. Если отказ не менял содержимое, пометка
/// остаётся; после частичной очистки её уже нельзя показывать как достоверную.
fn copy_with(
    clipboard: &Clipboard,
    id: u64,
    seen: &mut u32,
    write: impl FnOnce(&str) -> (u32, Result<(), String>),
) -> Outcome {
    let Some(text) = clipboard.locked().text_of(id) else {
        return Outcome { changed: false, result: Err(ControlError::Missing) };
    };
    let (sequence, result) = write(&text);
    match result {
        Ok(()) => {
            *seen = sequence;
            Outcome { changed: clipboard.locked().mark(id), result: Ok(()) }
        }
        Err(error) => {
            eprintln!("[буфер] {error}");
            let changed = if sequence != *seen {
                // EmptyClipboard мог удаться, а следующий SetClipboardData — нет.
                let mut history = clipboard.locked();
                history.current.take().is_some()
            } else {
                false
            };
            Outcome { changed, result: Err(ControlError::CopyFailed) }
        }
    }
}

/// Список записей и труба к потоку буфера.
#[derive(Default)]
pub struct Clipboard {
    history: Mutex<History>,
    orders: OnceLock<Sender<Order>>,
}

impl Clipboard {
    fn locked(&self) -> MutexGuard<'_, History> {
        // Отравленный мьютекс не повод падать: данные внутри осмысленные
        self.history
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Снимок для окна, открытого между тиками: иначе оно ждало бы первого
/// копирования, хотя список уже полон.
#[tauri::command]
pub fn clipboard_state(state: State<'_, Clipboard>) -> View {
    state.locked().view()
}

/// Ждёт ответ потока через oneshot, не задерживая главный поток WebView2.
#[tauri::command]
pub async fn clipboard_control(state: State<'_, Clipboard>, action: Action) -> Result<(), ControlError> {
    let orders = state.orders.get().ok_or(ControlError::Unavailable)?;
    let (reply, outcome) = oneshot::channel();
    orders.send(Order { action, reply }).map_err(|_| ControlError::Unavailable)?;
    outcome.await.map_err(|_| ControlError::Unavailable)?
}

/// Секрет копируется независимо от включения истории и видимости её окна.
/// Ожидание занятого буфера и повторы идут в пуле, не в главном потоке.
#[tauri::command]
pub async fn clipboard_copy_secret(window: tauri::WebviewWindow, text: String) -> Result<(), ControlError> {
    #[cfg(windows)]
    {
        tauri::async_runtime::spawn_blocking(move || {
            let owner = window.hwnd().map_err(|_| ControlError::Unavailable)?;
            board::write_secret(&text, owner).map_err(|error| {
                eprintln!("[буфер] секрет не скопирован: {error}");
                ControlError::CopyFailed
            })
        }).await.map_err(|_| ControlError::Unavailable)?
    }
    #[cfg(not(windows))]
    {
        let _ = (window, text);
        Err(ControlError::Unavailable)
    }
}

/// Поднимает поток буфера. Живёт до конца работы приложения.
pub fn spawn(app: AppHandle) {
    let (orders, inbox) = mpsc::channel();

    if app.state::<Clipboard>().orders.set(orders).is_err() {
        eprintln!("[буфер] поток уже поднят");
        return;
    }

    if let Err(error) = std::thread::Builder::new()
        .name("clipboard".to_owned())
        .spawn(move || work(app, inbox))
    {
        eprintln!("[буфер] поток не поднялся: {error}");
    }
}

/// Записываем ли мы буфер вообще.
///
/// ВАЖНО: смотрим на настройку, а не на видимость окна, — и это отличие от
/// звука, а не оплошность. Спрятанное окно звука опрашивать незачем, а история
/// буфера копится ровно для того, чтобы вернуться к ней позже: дыры в ней
/// обессмысливают виджет целиком. Зато выключенный в настройках виджет не
/// должен подсматривать за буфером втихую — и, выключаясь, забывает всё, что
/// успел записать.
fn recording(app: &AppHandle) -> bool {
    let Some(widget) = widgets::find(WIDGET) else {
        return false;
    };
    let Some(settings) = app.try_state::<SettingsState>() else {
        return false;
    };

    settings.get().widget(widget).enabled
}

fn publish(app: &AppHandle) {
    let view = app.state::<Clipboard>().locked().view();
    // Неудачу рассылки глотаем: окно могли спрятать прямо в этот момент
    let _ = app.emit(CLIPBOARD_EVENT, view);
}

#[cfg(windows)]
fn work(app: AppHandle, inbox: Receiver<Order>) {
    use std::sync::mpsc::RecvTimeoutError;

    // С чего начали, то и пропускаем: содержимое буфера на момент запуска
    // скопировано неизвестно когда, и первой записью истории оно было бы
    // враньём — «вот что ты копировал только что»
    let mut seen = board::sequence();

    loop {
        // Разбираем всю накопившуюся очередь и только потом рассылаем: правило
        // взято у звука, где полоса громкости топила окно событиями. Тут
        // очередь короче, но нажатие по строке и щелчок по крестику рядом
        // ложатся в неё вдвоём охотно.
        let mut changed = match inbox.recv_timeout(STEP) {
            Ok(order) => obey_order(&app, order, &mut seen),
            Err(RecvTimeoutError::Timeout) => false,
            // Приложение уходит: держатель отправителя умер вместе с ним
            Err(RecvTimeoutError::Disconnected) => return,
        };

        while let Ok(order) = inbox.try_recv() {
            changed |= obey_order(&app, order, &mut seen);
        }

        if recording(&app) {
            changed |= poll(&app, &mut seen);
        } else {
            // Виджет выключили: забываем список и держим номер свежим, чтобы
            // при включении обратно не приняли за новое то, что скопировали
            // при выключенном виджете
            seen = board::sequence();
            changed |= app.state::<Clipboard>().locked().clear();
        }

        if changed {
            publish(&app);
        }
    }
}

/// Смотрит, не сменилось ли содержимое буфера. Возвращает «список изменился».
#[cfg(windows)]
fn poll(app: &AppHandle, seen: &mut u32) -> bool {
    let now = board::sequence();
    if now == *seen {
        return false;
    }
    *seen = now;

    match board::read() {
        Some(text) => app.state::<Clipboard>().locked().record(&text),
        // Картинка, файлы или пометка «не для истории»: список не трогаем
        None => false,
    }
}

/// Отвечает только после попытки исполнения, даже если список не изменился.
#[cfg(windows)]
fn obey_order(app: &AppHandle, order: Order, seen: &mut u32) -> bool {
    let outcome = obey(app, order.action, seen);
    let _ = order.reply.send(outcome.result);
    outcome.changed
}

#[cfg(windows)]
fn obey(app: &AppHandle, action: Action, seen: &mut u32) -> Outcome {
    let clipboard = app.state::<Clipboard>();
    match action {
        Action::Copy { id } => copy_with(&clipboard, id, seen, |text| {
            let result = board::write(text, owner(app));
            // На успехе пропускаем свою запись; на отказе видим частичную очистку.
            (board::sequence(), result)
        }),
        Action::Remove { id } => Outcome { changed: clipboard.locked().remove(id), result: Ok(()) },
        Action::Clear => {
            // Прежнее поведение: забываем историю даже при отказе очистки ОС.
            let result = board::empty(owner(app)).map_err(|error| {
                eprintln!("[буфер] {error}");
                ControlError::ClearFailed
            });
            *seen = board::sequence();
            Outcome { changed: clipboard.locked().clear(), result }
        }
    }
}

/// Окно виджета как владелец буфера.
///
/// Нужен там, где идёт `EmptyClipboard`: с пустым владельцем он сбрасывает
/// владельца буфера в ноль, и `SetClipboardData` вправе после этого отказать.
#[cfg(windows)]
fn owner(app: &AppHandle) -> Option<windows::Win32::Foundation::HWND> {
    app.get_webview_window(WIDGET)?.hwnd().ok()
}

#[cfg(not(windows))]
fn work(_app: AppHandle, _inbox: Receiver<Order>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_length_requires_a_terminator_within_the_limit() {
        assert_eq!(bounded_text_length(&[]), None);
        assert_eq!(bounded_text_length(&[0, 65]), Some(0));
        assert_eq!(bounded_text_length(&[65, 66, 0, 67]), Some(2));
        assert_eq!(bounded_text_length(&[65, 66]), None);
        let mut wide = vec![65; MAX_CHARS * 2 + 2];
        wide[MAX_CHARS * 2 + 1] = 0;
        assert_eq!(bounded_text_length(&wide), None, "ноль за потолком не читаем");
        wide[MAX_CHARS * 2] = 0;
        assert_eq!(bounded_text_length(&wide), Some(MAX_CHARS * 2));
    }

    #[test]
    fn bounded_length_keeps_surrogate_pairs_at_the_character_limit() {
        let text = "😀".repeat(MAX_CHARS);
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let length = bounded_text_length(&wide).expect("пары и ноль помещаются");
        let decoded = String::from_utf16_lossy(&wide[..length]);
        assert!(History::default().record(&decoded));
        // Грубый предел в UTF-16 не подменяет точную проверку числа знаков.
        let mut ascii = vec![65; MAX_CHARS + 1];
        ascii.push(0);
        let length = bounded_text_length(&ascii).expect("ограниченный проход допустил строку");
        assert!(!History::default().record(&String::from_utf16_lossy(&ascii[..length])));
    }

    fn filled(texts: &[&str]) -> History {
        let mut history = History::default();
        for text in texts {
            history.record(text);
        }
        history
    }

    /// Свежее сверху: список читают сверху вниз, а не наоборот.
    #[test]
    fn newest_comes_first() {
        let history = filled(&["первое", "второе", "третье"]);

        let texts: Vec<&str> = history.entries.iter().map(|entry| entry.text.as_str()).collect();
        assert_eq!(texts, ["третье", "второе", "первое"]);
    }

    /// Повтор снаружи переезжает наверх, а не ложится второй строкой: он и
    /// правда самый свежий, а две одинаковые строки в списке — просто шум.
    #[test]
    fn a_repeat_moves_up_instead_of_doubling() {
        let history = filled(&["первое", "второе", "первое"]);

        assert_eq!(history.entries.len(), 2);
        assert_eq!(history.entries[0].text, "первое");
        assert_eq!(history.entries[1].text, "второе");
    }

    /// И сохраняет свой номер: строка не должна прыгать под курсором и
    /// перерисовываться как новая.
    #[test]
    fn a_repeat_keeps_its_id() {
        let mut history = filled(&["первое", "второе"]);
        let id = history.entries[1].id;

        history.record("первое");

        assert_eq!(history.entries[0].id, id);
        assert_eq!(history.current, Some(id));
    }

    /// Двадцать первая вытесняет самую старую.
    #[test]
    fn the_oldest_falls_out() {
        let mut history = History::default();
        for number in 0..=CAPACITY {
            history.record(&format!("запись {number}"));
        }

        assert_eq!(history.entries.len(), CAPACITY);
        assert_eq!(history.entries[0].text, format!("запись {CAPACITY}"));
        assert_eq!(history.entries[CAPACITY - 1].text, "запись 1");
    }

    /// Пустое и пробельное — промах мимо нужного текста, а не запись.
    #[test]
    fn blank_text_is_not_recorded() {
        let mut history = History::default();

        assert!(!history.record(""));
        assert!(!history.record("   \r\n\t "));
        assert!(history.entries.is_empty());
    }

    /// Слишком длинное не пишем вовсе: обрезанная запись при копировании
    /// обратно соврала бы молча.
    #[test]
    fn oversized_text_is_not_recorded() {
        let mut history = History::default();

        assert!(history.record(&"я".repeat(MAX_CHARS)));
        assert!(!history.record(&"я".repeat(MAX_CHARS + 1)));
        assert_eq!(history.entries.len(), 1);
    }

    /// Потолок считается в знаках, а не в байтах: у кириллицы их вдвое больше,
    /// и по байтам предел наступал бы на половине обещанного.
    #[test]
    fn the_limit_counts_characters_not_bytes() {
        let text = "я".repeat(MAX_CHARS);
        assert!(text.len() > MAX_CHARS, "иначе тест ничего не проверяет");

        assert!(History::default().record(&text));
    }

    #[test]
    fn preview_squeezes_whitespace() {
        assert_eq!(
            preview("  fn main() {\n    println!(\"привет\");\n}\n"),
            "fn main() { println!(\"привет\"); }"
        );
    }

    /// Обрезка идёт по знакам и не разрывает символ посередине.
    #[test]
    fn preview_stops_at_the_limit() {
        let shown = preview(&"ё".repeat(PREVIEW_CHARS * 2));

        assert_eq!(shown.chars().count(), PREVIEW_CHARS);
        assert!(shown.chars().all(|character| character == 'ё'));
    }

    /// Обрезка посреди пробела не оставляет его висеть на конце.
    #[test]
    fn preview_never_ends_with_a_space() {
        let shown = preview(&"аб ".repeat(PREVIEW_CHARS));

        assert_eq!(shown.chars().count(), PREVIEW_CHARS);
        assert!(!shown.ends_with(' '));
    }

    /// Слово, кончающееся ровно на предпоследнем месте, — тот самый случай,
    /// когда пробел успевал встать последним знаком, а следующее слово уже не
    /// влезало ни одной буквой.
    #[test]
    fn a_word_ending_one_short_of_the_limit_takes_no_space_with_it() {
        let long = "а".repeat(PREVIEW_CHARS - 1);
        let shown = preview(&format!("{long} второе"));

        assert_eq!(shown.chars().count(), PREVIEW_CHARS - 1);
        assert!(!shown.ends_with(' '), "пробел повис на конце: {shown:?}");
    }

    /// Копирование обратно — это пометка, а не запись: список не шелохнётся.
    #[test]
    fn copying_back_only_moves_the_mark() {
        let mut history = filled(&["первое", "второе", "третье"]);
        let id = history.entries[2].id;

        assert!(history.mark(id));

        assert_eq!(history.entries.len(), 3);
        assert_eq!(history.entries[0].text, "третье");
        assert_eq!(history.current, Some(id));
        // Повторное нажатие по той же строке ничего не меняет
        assert!(!history.mark(id));
    }

    #[test]
    fn removing_the_current_entry_puts_out_the_mark() {
        let mut history = filled(&["первое", "второе"]);
        let id = history.entries[0].id;

        assert!(history.remove(id));
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.current, None);

        // Той же записи второй раз нет
        assert!(!history.remove(id));
    }

    #[test]
    fn clearing_an_empty_list_changes_nothing() {
        let mut history = History::default();
        assert!(!history.clear());

        history.record("что-нибудь");
        assert!(history.clear());
        assert!(history.entries.is_empty());
        assert_eq!(history.current, None);
    }

    /// Разметка уходит на фронт как есть: snake_case там уже однажды всплывал.
    #[test]
    fn view_arrives_in_camel_case() {
        let json = serde_json::to_string(&filled(&["привет"]).view()).expect("сериализуется");

        assert!(json.contains(r#""preview":"привет""#), "{json}");
        assert!(json.contains(r#""chars":6"#), "{json}");
        assert!(json.contains(r#""current":true"#), "{json}");
        assert!(json.contains(r#""capacity":20"#), "{json}");
    }

    /// Пустой список приходит со своим потолком, а не с нулём: счётчик в
    /// подвале показывает «0 из 20» с первой отрисовки.
    #[test]
    fn an_empty_view_still_knows_its_capacity() {
        let json = serde_json::to_string(&View::default()).expect("сериализуется");
        assert_eq!(json, r#"{"entries":[],"capacity":20}"#);
    }

    /// Команды приходят с фронта размеченными по `kind`.
    #[test]
    fn actions_are_tagged_by_kind() {
        let copy: Action = serde_json::from_str(r#"{"kind":"copy","id":7}"#).expect("разбирается");
        assert!(matches!(copy, Action::Copy { id: 7 }));

        let remove: Action =
            serde_json::from_str(r#"{"kind":"remove","id":3}"#).expect("разбирается");
        assert!(matches!(remove, Action::Remove { id: 3 }));

        let clear: Action = serde_json::from_str(r#"{"kind":"clear"}"#).expect("разбирается");
        assert!(matches!(clear, Action::Clear));
    }

    #[test]
    fn formatted_preview_preserves_lines_and_indentation() {
        let text = "Первая\r\n  Вторая\rТретья\n";
        assert_eq!(formatted_preview(text), "Первая\n  Вторая\nТретья\n");
        assert_eq!(preview(text), "Первая Вторая Третья");
        assert_eq!(line_count(text), 4);
    }

    #[test]
    fn formatted_preview_limits_unicode_without_splitting_a_character() {
        let text = "😀".repeat(PREVIEW_CHARS + 1);
        let shown = formatted_preview(&text);
        assert_eq!(shown.chars().count(), PREVIEW_CHARS);
        assert!(shown.chars().all(|character| character == '😀'));
    }

    #[test]
    fn newline_normalization_still_obeys_the_preview_limit() {
        let shown = formatted_preview(&"я\r\n".repeat(PREVIEW_CHARS));
        assert_eq!(shown.chars().count(), PREVIEW_CHARS);
        assert!(!shown.contains('\r'));
    }

    #[test]
    fn line_count_includes_blank_and_trailing_lines_once() {
        assert_eq!(line_count("одна строка"), 1);
        assert_eq!(line_count("а\r\n\r\nб\n"), 4);
        assert_eq!(line_count("а\rб\nв\r\nг"), 4);
    }

    #[test]
    fn a_view_keeps_both_bounded_previews_without_full_text() {
        let text = format!("Первое\r\n  Второе\n{}", "я".repeat(300));
        let view = filled(&[&text]).view();
        let entry = &view.entries[0];
        assert!(!entry.preview.contains('\n'));
        assert!(entry.formatted_preview.starts_with("Первое\n  Второе\n"));
        assert!(entry.preview.chars().count() <= PREVIEW_CHARS);
        assert_eq!(entry.formatted_preview.chars().count(), PREVIEW_CHARS);
        assert_eq!(entry.chars, text.chars().count());
        assert_eq!(entry.lines, 3);
        let json = serde_json::to_value(&view).expect("сериализуется");
        let shown = &json["entries"][0];
        assert!(shown.get("formattedPreview").is_some());
        assert!(shown.get("text").is_none());
    }

    #[test]
    fn a_failed_copy_keeps_current_and_sequence() {
        let clipboard = Clipboard { history: Mutex::new(filled(&["первая", "вторая"])), ..Clipboard::default() };
        let before = clipboard.locked().view();
        let id = before.entries[1].id;
        let mut seen = 12;
        let result = copy_with(&clipboard, id, &mut seen, |text| {
            assert_eq!(text, "первая");
            (12, Err("вымышленный отказ записи".to_owned()))
        });
        assert_eq!(result.result, Err(ControlError::CopyFailed));
        assert!(!result.changed);
        assert_eq!(seen, 12);
        assert_eq!(clipboard.locked().view(), before);
    }

    #[test]
    fn an_acknowledged_copy_marks_without_reordering() {
        let clipboard = Clipboard { history: Mutex::new(filled(&["первая", "вторая"])), ..Clipboard::default() };
        let before = clipboard.locked().view();
        let id = before.entries[1].id;
        let mut seen = 12;
        let result = copy_with(&clipboard, id, &mut seen, |_| (13, Ok(())));
        assert_eq!(result.result, Ok(()));
        assert!(result.changed);
        assert_eq!(seen, 13);
        let after = clipboard.locked().view();
        assert_eq!(after.entries.iter().map(|entry| entry.id).collect::<Vec<_>>(), before.entries.iter().map(|entry| entry.id).collect::<Vec<_>>());
        assert_eq!(after.entries.iter().find(|entry| entry.current).map(|entry| entry.id), Some(id));
        let result = copy_with(&clipboard, id, &mut seen, |_| (14, Ok(())));
        assert_eq!(result.result, Ok(()));
        assert!(!result.changed);
        assert_eq!(seen, 14);
    }

    #[test]
    fn a_removed_entry_is_rejected_without_calling_the_writer() {
        let clipboard = Clipboard { history: Mutex::new(filled(&["демо"])), ..Clipboard::default() };
        let before = clipboard.locked().view();
        let mut seen = 9;
        let result = copy_with(&clipboard, 999, &mut seen, |_| panic!("отсутствующий текст не записываем"));
        assert_eq!(result.result, Err(ControlError::Missing));
        assert!(!result.changed);
        assert_eq!(seen, 9);
        assert_eq!(clipboard.locked().view(), before);
    }

    #[test]
    fn a_partial_write_failure_puts_out_the_stale_current_mark() {
        let clipboard = Clipboard { history: Mutex::new(filled(&["первая", "вторая"])), ..Clipboard::default() };
        let before = clipboard.locked().view();
        let mut seen = 12;
        let result = copy_with(&clipboard, before.entries[1].id, &mut seen, |_| {
            (13, Err("вымышленный отказ после очистки".to_owned()))
        });
        assert_eq!(result.result, Err(ControlError::CopyFailed));
        assert!(result.changed);
        assert_eq!(seen, 12, "следующий poll должен увидеть смену содержимого");
        let after = clipboard.locked().view();
        assert!(after.entries.iter().all(|entry| !entry.current));
        assert_eq!(after.entries.iter().map(|entry| entry.id).collect::<Vec<_>>(), before.entries.iter().map(|entry| entry.id).collect::<Vec<_>>());
    }

}
