//! Буфер обмена: последние двадцать скопированных строк, закреплённые строки
//! и последние десять картинок.
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
#[cfg(windows)]
mod image;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
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

/// Сколько записей помним. Закреплённые сюда не входят.
const CAPACITY: usize = 20;

/// Сколько записей можно закрепить. Закреплённые живут только в памяти, как и
/// вся история, и потолок у них свой: иначе закрепление стало бы способом
/// держать в памяти сколько угодно.
const PIN_CAPACITY: usize = 20;

/// Сколько картинок помним. Каждая лежит у нас PNG целиком — снимок экрана
/// это мегабайты, а не килобайты, как строка.
const IMAGE_CAPACITY: usize = 10;

/// Потолок картинки в точках: больше 50 мегапикселей — это не снимок экрана, а
/// чья-то панорама на сотни мегабайт в несжатом виде.
const MAX_PIXELS: u64 = 50_000_000;

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
    /// Закреплённая не вытесняется и переживает «Очистить». Только в памяти.
    pinned: bool,
}

/// Картинка, какой она лежит у нас.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Picture {
    id: u64,
    /// Картинка целиком, PNG: из него и возвращаем в буфер, и сохраняем файл.
    png: std::sync::Arc<Vec<u8>>,
    /// sha256 от PNG: та же картинка, скопированная снова, переезжает наверх.
    hash: [u8; 32],
    width: u32,
    height: u32,
    /// Миниатюра, `data:image/png;base64,…`. В событие не едет — её просит
    /// окно командой `clipboard_thumb`, по образцу обложки медиа.
    thumb: String,
}

/// Картинка, какой её видит окно: без пикселей.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShownImage {
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub current: bool,
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
    /// Ограниченный предпросмотр NEW с исходными переводами строк.
    pub formatted_preview: String,
    /// Строки исходной записи, в том числе пустые; CRLF — один перевод.
    pub lines: usize,
    /// Сколько знаков в записи целиком — для подсказки под курсором.
    pub chars: usize,
    /// Эта запись лежит в буфере прямо сейчас.
    pub current: bool,
    /// Закреплена: стоит над историей и не вытесняется.
    pub pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    /// Закреплённые первыми в порядке закрепления, за ними история, свежая
    /// первой.
    pub entries: Vec<Shown>,
    /// Сколько незакреплённых записей помещается всего — для счётчика.
    pub capacity: usize,
    /// Свежая первой.
    pub images: Vec<ShownImage>,
    pub image_capacity: usize,
}

impl Default for View {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            capacity: CAPACITY,
            images: Vec::new(),
            image_capacity: IMAGE_CAPACITY,
        }
    }
}

/// Что очищает кнопка в подвале: вкладку, которая открыта.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Clearing {
    #[default]
    Text,
    Images,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Action {
    /// Вернуть запись в буфер обмена.
    Copy { id: u64 },
    /// Убрать запись из списка. Самого буфера это не касается.
    Remove { id: u64 },
    /// Закрепить запись или открепить.
    Pin { id: u64, pinned: bool },
    /// Вернуть картинку в буфер обмена.
    CopyImage { id: u64 },
    /// Убрать картинку из списка.
    RemoveImage { id: u64 },
    /// Сохранить картинку в PNG через окно сохранения.
    SaveImage { id: u64 },
    /// Забыть незакреплённые строки или картинки и опустошить сам буфер.
    Clear {
        #[serde(default)]
        what: Clearing,
    },
}

/// Список записей.
///
/// Вся логика этого виджета живёт тут, и вся она проверяется тестом: Windows
/// нужна только чтобы узнать, что содержимое сменилось, и прочитать его.
#[derive(Debug, Default)]
struct History {
    /// Закреплённые первыми, за ними незакреплённые, свежая первой.
    entries: VecDeque<Entry>,
    /// Свежая первой.
    images: VecDeque<Picture>,
    /// Какая запись или картинка лежит в буфере прямо сейчас: номера у них
    /// общие, а в буфере бывает что-то одно.
    current: Option<u64>,
    next_id: u64,
}

impl History {
    fn take_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Сколько закреплённых: они стоят в начале списка подряд.
    fn pinned_count(&self) -> usize {
        self.entries.iter().take_while(|entry| entry.pinned).count()
    }

    /// Двадцать первая незакреплённая вытесняет самую старую. Она же
    /// последняя в списке: закреплённые стоят впереди.
    fn trim(&mut self) {
        while self.entries.len() - self.pinned_count() > CAPACITY {
            self.entries.pop_back();
        }
    }

    /// Записывает свежее содержимое буфера. Возвращает «список изменился».
    fn record(&mut self, text: &str) -> bool {
        // Пустое и пробельное — это не запись, а промах мимо нужного текста
        if text.trim().is_empty() {
            return false;
        }
        if text.chars().count() > MAX_CHARS {
            return false;
        }

        // Тот же текст, скопированный ещё раз, переезжает наверх истории, а не
        // ложится второй строкой: он и правда самый свежий. Закреплённый
        // остаётся на своём месте — его место выбрал человек
        if let Some(at) = self.entries.iter().position(|entry| entry.text == text) {
            if self.entries[at].pinned {
                // Принято, даже если пометка уже стояла: `false` значил бы для
                // опроса «в буфере чужое», и он погасил бы её
                let id = self.entries[at].id;
                self.mark(id);
                return true;
            }
            let entry = self.entries.remove(at).expect("позиция только что найдена");
            self.current = Some(entry.id);
            let top = self.pinned_count();
            self.entries.insert(top, entry);
            return true;
        }

        let id = self.take_id();
        let top = self.pinned_count();
        self.entries.insert(top, Entry {
            id,
            text: text.to_owned(),
            pinned: false,
        });
        self.trim();
        self.current = Some(id);

        true
    }

    /// Закрепляет запись в конец закреплённых или открепляет наверх истории.
    fn pin(&mut self, id: u64, pinned: bool) -> Result<bool, ControlError> {
        let Some(at) = self.entries.iter().position(|entry| entry.id == id) else {
            return Err(ControlError::Missing);
        };
        if self.entries[at].pinned == pinned {
            return Ok(false);
        }
        if pinned && self.pinned_count() >= PIN_CAPACITY {
            return Err(ControlError::PinsFull);
        }

        let mut entry = self.entries.remove(at).expect("позиция только что найдена");
        entry.pinned = pinned;
        // Конец закреплённых и начало истории — одно и то же место
        let top = self.pinned_count();
        self.entries.insert(top, entry);
        self.trim();
        Ok(true)
    }

    /// Записывает картинку. Повтор переезжает наверх.
    fn record_image(&mut self, picture: Picture) -> bool {
        if let Some(at) = self.images.iter().position(|kept| kept.hash == picture.hash) {
            let kept = self.images.remove(at).expect("позиция только что найдена");
            self.current = Some(kept.id);
            self.images.push_front(kept);
            return true;
        }

        let id = self.take_id();
        self.images.push_front(Picture { id, ..picture });
        self.images.truncate(IMAGE_CAPACITY);
        self.current = Some(id);
        true
    }

    fn image_of(&self, id: u64) -> Option<&Picture> {
        self.images.iter().find(|picture| picture.id == id)
    }

    fn remove_image(&mut self, id: u64) -> bool {
        let Some(at) = self.images.iter().position(|picture| picture.id == id) else {
            return false;
        };
        self.images.remove(at);
        if self.current == Some(id) {
            self.current = None;
        }
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

    /// «Очистить» на вкладке строк: закреплённые остаются. Пометка гаснет —
    /// сам буфер опустошается рядом.
    fn clear(&mut self) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.pinned);
        let changed = self.entries.len() != before || self.current.is_some();
        self.current = None;
        changed
    }

    /// «Очистить» на вкладке картинок.
    fn clear_images(&mut self) -> bool {
        let changed = !self.images.is_empty() || self.current.is_some();
        self.images.clear();
        self.current = None;
        changed
    }

    /// Выключение модуля: забывается всё, закреплённые и картинки тоже.
    fn forget(&mut self) -> bool {
        let changed = !self.entries.is_empty() || !self.images.is_empty() || self.current.is_some();
        self.entries.clear();
        self.images.clear();
        self.current = None;
        changed
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
                    pinned: entry.pinned,
                })
                .collect(),
            capacity: CAPACITY,
            images: self
                .images
                .iter()
                .map(|picture| ShownImage {
                    id: picture.id,
                    width: picture.width,
                    height: picture.height,
                    current: self.current == Some(picture.id),
                })
                .collect(),
            image_capacity: IMAGE_CAPACITY,
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

/// NEW сохраняет переносы и отступы только в ограниченной проекции.
/// Полный текст по-прежнему остаётся в History, а Classic читает прежний preview.
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
    /// Закреплено уже [`PIN_CAPACITY`] записей.
    PinsFull,
    /// Картинка не сохранилась в файл.
    SaveFailed,
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

/// Миниатюра картинки, `data:`-адрес или `null`. Ходит в память, не в буфер.
#[tauri::command]
pub fn clipboard_thumb(state: State<'_, Clipboard>, id: u64) -> Option<String> {
    state.locked().image_of(id).map(|picture| picture.thumb.clone())
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

/// Модуль выключили: история забывается на ближайшем тике, даже если его
/// тут же включат снова. Сам тик видит выключенный модуль, только если попал
/// в паузу, — «выключить — включить» быстрее полсекунды оставляло в истории
/// всё скопированное, пароли извне тоже.
static FORGET: AtomicBool = AtomicBool::new(false);

pub fn forget() {
    FORGET.store(true, Ordering::SeqCst);
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

    // WIC — это COM, и квартира у потока своя на всю его жизнь
    image::enter();

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

        if FORGET.swap(false, Ordering::SeqCst) {
            // И номер свежий: скопированное при выключенном модуле — не новое
            seen = board::sequence();
            changed |= app.state::<Clipboard>().locked().forget();
        }

        if recording(&app) {
            changed |= poll(&app, &mut seen);
        } else {
            // Виджет выключили: забываем всё, закреплённые и картинки тоже, и
            // держим номер свежим, чтобы при включении обратно не приняли за
            // новое то, что скопировали при выключенном виджете
            seen = board::sequence();
            changed |= app.state::<Clipboard>().locked().forget();
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

    let content = match board::read() {
        Ok(content) => content,
        // Буфер держит чужое приложение (Excel с большим диапазоном, RDP,
        // другой менеджер буфера). Номер не сдвигаем: следующий тик прочтёт
        // снова, иначе эта запись пропала бы навсегда — номер больше не сменится
        Err(_) => return false,
    };
    *seen = now;

    // Картинку разбираем до замка: WIC над снимком 4K — десятки миллисекунд,
    // и окно, спросившее миниатюру, ждать их не должно
    let (text, picture) = match content {
        board::Content::Text(text) => (Some(text), None),
        board::Content::Image(raw) => match image::picture(&raw, MAX_PIXELS) {
            Ok(picture) => (None, Some(picture)),
            Err(error) => {
                eprintln!("[буфер] картинка не записана: {error}");
                (None, None)
            }
        },
        board::Content::Nothing => (None, None),
    };

    let clipboard = app.state::<Clipboard>();
    let mut history = clipboard.locked();
    let recorded = match (text, picture) {
        (Some(text), _) => history.record(&text),
        (None, Some(picture)) => history.record_image(Picture {
            id: 0,
            png: std::sync::Arc::new(picture.png),
            hash: picture.hash,
            width: picture.width,
            height: picture.height,
            thumb: picture.thumb,
        }),
        _ => false,
    };
    if recorded {
        return true;
    }
    // Файлы, пометка «не для истории», пустое, слишком длинное или слишком
    // большое: в буфере теперь не то, что было, и подсвечивать прежнюю строку
    // как лежащую в нём — врать (Ctrl+V вставит другое)
    history.current.take().is_some()
}

/// Отвечает только после попытки исполнения, даже если список не изменился.
#[cfg(windows)]
fn obey_order(app: &AppHandle, order: Order, seen: &mut u32) -> bool {
    // Сохранение ждёт человека в окне выбора, а поток буфера ждать не может:
    // ответ уходит из потока окна выбора, когда файл записан или отменён
    if let Action::SaveImage { id } = order.action {
        save_image(app, id, order.reply);
        return false;
    }
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
        Action::Pin { id, pinned } => match clipboard.locked().pin(id, pinned) {
            Ok(changed) => Outcome { changed, result: Ok(()) },
            Err(error) => Outcome { changed: false, result: Err(error) },
        },
        Action::CopyImage { id } => copy_image_with(&clipboard, id, seen, |png| {
            let result = image::dib(png).and_then(|dib| board::write_image(png, &dib, owner(app)));
            (board::sequence(), result)
        }),
        Action::RemoveImage { id } => Outcome { changed: clipboard.locked().remove_image(id), result: Ok(()) },
        // Разбирается в `obey_order`: ответ ждёт окно выбора
        Action::SaveImage { .. } => Outcome { changed: false, result: Err(ControlError::Unavailable) },
        Action::Clear { what } => {
            // Прежнее поведение: забываем историю даже при отказе очистки ОС.
            let result = board::empty(owner(app)).map_err(|error| {
                eprintln!("[буфер] {error}");
                ControlError::ClearFailed
            });
            *seen = board::sequence();
            let mut history = clipboard.locked();
            let changed = match what {
                Clearing::Text => history.clear(),
                Clearing::Images => history.clear_images(),
            };
            Outcome { changed, result }
        }
    }
}

/// То же, что [`copy_with`], для картинки: PNG берётся под замком, пишется без.
fn copy_image_with(
    clipboard: &Clipboard,
    id: u64,
    seen: &mut u32,
    write: impl FnOnce(&[u8]) -> (u32, Result<(), String>),
) -> Outcome {
    let Some(png) = clipboard.locked().image_of(id).map(|picture| picture.png.clone()) else {
        return Outcome { changed: false, result: Err(ControlError::Missing) };
    };
    let (sequence, result) = write(&png);
    match result {
        Ok(()) => {
            *seen = sequence;
            Outcome { changed: clipboard.locked().mark(id), result: Ok(()) }
        }
        Err(error) => {
            eprintln!("[буфер] {error}");
            let changed = sequence != *seen && clipboard.locked().current.take().is_some();
            Outcome { changed, result: Err(ControlError::CopyFailed) }
        }
    }
}

/// Окно сохранения с именем по часам и запись PNG как он лежит у нас.
#[cfg(windows)]
fn save_image(app: &AppHandle, id: u64, reply: oneshot::Sender<Result<(), ControlError>>) {
    let Some(png) = app.state::<Clipboard>().locked().image_of(id).map(|picture| picture.png.clone()) else {
        let _ = reply.send(Err(ControlError::Missing));
        return;
    };
    let Some(owner) = owner(app) else {
        let _ = reply.send(Err(ControlError::Unavailable));
        return;
    };

    let what = crate::picker::Pick::Save {
        name: format!("Clipboard {}.png", file_stamp()),
        filter: crate::picker::Filter::PNG,
    };
    crate::picker::pick(owner.0 as isize, what, None, move |paths| {
        let Some(path) = paths.into_iter().next() else {
            // Передумал — это не отказ
            let _ = reply.send(Ok(()));
            return;
        };
        let result = crate::disk::write_atomically(&path, png.as_slice()).map_err(|error| {
            eprintln!("[буфер] картинка не сохранилась в {}: {error}", path.display());
            ControlError::SaveFailed
        });
        let _ = reply.send(result);
    });
}

/// Время для имени файла по часам человека: двоеточий в именах Windows нет.
#[cfg(windows)]
fn file_stamp() -> String {
    // SAFETY: GetLocalTime только заполняет структуру и не может отказать
    let time = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}-{:02}-{:02}",
        time.wYear, time.wMonth, time.wDay, time.wHour, time.wMinute, time.wSecond
    )
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
        assert_eq!(json, r#"{"entries":[],"capacity":20,"images":[],"imageCapacity":10}"#);
    }

    /// Закреплённая стоит над историей, не вытесняется и переживает «Очистить».
    #[test]
    fn a_pinned_entry_outlives_the_history() {
        let mut history = filled(&["закреплю"]);
        let id = history.entries[0].id;
        assert_eq!(history.pin(id, true), Ok(true));

        for number in 0..=CAPACITY {
            history.record(&format!("запись {number}"));
        }
        assert_eq!(history.entries.len(), CAPACITY + 1);
        assert_eq!(history.entries[0].text, "закреплю");
        assert_eq!(history.entries[1].text, format!("запись {CAPACITY}"));

        assert!(history.clear());
        assert_eq!(history.entries.iter().map(|entry| entry.text.as_str()).collect::<Vec<_>>(), ["закреплю"]);
        assert!(history.forget());
        assert!(history.entries.is_empty());
    }

    /// Повтор закреплённого текста снаружи ставит пометку и не двигает его, а
    /// уже стоящая пометка не гаснет: опрос принимает содержимое.
    #[test]
    fn a_repeat_of_a_pinned_text_keeps_its_place_and_mark() {
        let mut history = filled(&["закреплю", "другое"]);
        let id = history.entries[1].id;
        history.pin(id, true).expect("закрепилось");
        history.record("третье");

        assert!(history.record("закреплю"));
        assert_eq!(history.entries[0].id, id);
        assert_eq!(history.current, Some(id));
        assert!(history.record("закреплю"), "повтор при стоящей пометке — тоже принят");
        assert_eq!(history.current, Some(id));
    }

    #[test]
    fn pins_have_their_own_ceiling_and_unpin_goes_to_the_top() {
        let mut history = History::default();
        for number in 0..PIN_CAPACITY {
            history.record(&format!("строка {number}"));
        }
        let ids: Vec<u64> = history.entries.iter().map(|entry| entry.id).collect();
        for id in &ids {
            assert_eq!(history.pin(*id, true), Ok(true));
        }
        history.record("лишняя");
        let extra = history.entries[PIN_CAPACITY].id;
        assert_eq!(history.pin(extra, true), Err(ControlError::PinsFull));
        assert_eq!(history.pin(999, true), Err(ControlError::Missing));

        assert_eq!(history.pin(ids[0], false), Ok(true));
        assert_eq!(history.entries[PIN_CAPACITY - 1].id, ids[0], "открепленная — первой в истории");
        assert!(!history.entries[PIN_CAPACITY - 1].pinned);
    }

    fn picture(hash: u8) -> Picture {
        Picture { id: 0, png: std::sync::Arc::new(vec![hash]), hash: [hash; 32], width: 2, height: 1, thumb: String::new() }
    }

    #[test]
    fn images_repeat_upwards_and_fall_out_after_ten() {
        let mut history = History::default();
        for hash in 0..=IMAGE_CAPACITY as u8 {
            assert!(history.record_image(picture(hash)));
        }
        assert_eq!(history.images.len(), IMAGE_CAPACITY);
        let oldest = history.images.back().expect("есть").id;
        assert!(history.record_image(picture(1)));
        assert_eq!(history.images[0].hash, [1; 32]);
        assert_eq!(history.current, Some(history.images[0].id));
        assert_ne!(history.images.back().expect("есть").id, oldest, "повтор переехал, а не лёг второй");

        assert!(history.clear_images());
        assert!(history.images.is_empty());
        assert_eq!(history.current, None);
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
        assert!(matches!(clear, Action::Clear { what: Clearing::Text }));
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
