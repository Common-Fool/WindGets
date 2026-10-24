//! Сам буфер обмена Windows: прочитать, записать, очистить.
//!
//! Здесь и только здесь живут вызовы Win32 самого буфера, наружу уходят
//! [`Content`] и `Result<(), String>`. Разбор картинок — в `image.rs`. COM тут ни при чём — буфер это user32, — но правило
//! то же, что у звука: главный поток занят WebView2, и ходить сюда из потока
//! команды нельзя. Чужое приложение держит буфер открытым сколько ему угодно.

use std::thread::sleep;
use std::time::Duration;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber,
    IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};

/// Текст в UTF-16.
///
/// Число закреплено за форматом с самого начала Win32 и объявлено в winuser.h.
/// В крейте `windows` оно лежит в `Win32_System_Ole` — мегабайте
/// сгенерированного кода, который весь целиком собирался бы ради этой строки.
const CF_UNICODETEXT: u32 = 13;

/// Картинка без заголовка файла: BITMAPINFO и точки. Числа оттуда же, из
/// winuser.h, и по той же причине объявлены руками.
const CF_DIB: u32 = 8;
const CF_DIBV5: u32 = 17;

/// Потолок блока картинки: 50 мегапикселей по четыре байта и заголовок.
/// Чужой блок больше этого не копируем вовсе — точнее размер проверит WIC.
const MAX_IMAGE_BYTES: usize = 50_000_000 * 4 + 4096;

/// Что лежит в буфере.
pub enum Content {
    Text(String),
    Image(Raw),
    /// Ничего, что мы пишем: файлы, пустой буфер, пометка «не для истории»,
    /// слишком длинный текст или слишком большая картинка.
    Nothing,
}

/// Картинка как она лежала в буфере.
pub enum Raw {
    /// Зарегистрированный формат `PNG`: его кладут браузеры, «Ножницы» и
    /// почти все снимки экрана — с прозрачностью, без потерь.
    Png(Vec<u8>),
    /// CF_DIBV5 или CF_DIB: BITMAPINFO и точки, без заголовка файла.
    Dib(Vec<u8>),
}

/// Сколько раз пробуем открыть буфер.
///
/// Буфер держит одно приложение за раз, и отказ от чужого открытия — не
/// ошибка, а «попробуй ещё»: любой редактор берёт его на доли миллисекунды.
const ATTEMPTS: u32 = 5;

const RETRY: Duration = Duration::from_millis(10);

/// Открытый буфер обмена.
///
/// Закрывается сам: пропущенный `CloseClipboard` запирает буфер для всей
/// системы, и вставка перестаёт работать везде, а не только у нас.
struct Open;

impl Open {
    fn new(owner: Option<HWND>) -> Result<Self, String> {
        for attempt in 0..ATTEMPTS {
            if unsafe { OpenClipboard(owner) }.is_ok() {
                return Ok(Self);
            }
            if attempt + 1 < ATTEMPTS {
                sleep(RETRY);
            }
        }

        Err("буфер обмена занят другим приложением".to_owned())
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        let _ = unsafe { CloseClipboard() };
    }
}

/// Номер последовательности: меняется на каждую смену содержимого.
///
/// Единственное, что спрашивается на каждом тике. Буфер при этом не
/// открывается вовсе — потому опрос и обходится системе даром.
pub fn sequence() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

/// Содержимое буфера.
///
/// `Err` — буфер держит чужое приложение: это не «ничего нет», и прочесть его
/// надо ещё раз, а не пропустить.
///
/// ВАЖНО: **есть текст — пишется только текст.** Excel, Word и браузер кладут
/// рядом с текстом картинку того же куска, и запоминать её второй записью —
/// шум.
pub fn read() -> Result<Content, String> {
    let _open = Open::new(None)?;

    if excluded() {
        return Ok(Content::Nothing);
    }
    if unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT) }.is_ok() {
        return Ok(read_text().map_or(Content::Nothing, Content::Text));
    }
    if let Some(png) = format_id(w!("PNG")).and_then(read_block) {
        return Ok(Content::Image(Raw::Png(png)));
    }
    // V5 первым: у него есть прозрачность, а CF_DIB система выводит из него
    // сама и альфу при этом теряет
    for format in [CF_DIBV5, CF_DIB] {
        if let Some(dib) = read_block(format) {
            return Ok(Content::Image(Raw::Dib(dib)));
        }
    }
    Ok(Content::Nothing)
}

/// Блок формата целиком, если он не больше потолка картинки.
fn read_block(format: u32) -> Option<Vec<u8>> {
    if unsafe { IsClipboardFormatAvailable(format) }.is_err() {
        return None;
    }
    let handle = unsafe { GetClipboardData(format) }.ok()?;
    let memory = HGLOBAL(handle.0);
    let size = unsafe { GlobalSize(memory) };
    if size == 0 || size > MAX_IMAGE_BYTES {
        return None;
    }
    let pointer = unsafe { GlobalLock(memory) }.cast::<u8>();
    if pointer.is_null() {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(pointer, size) }.to_vec();
    let _ = unsafe { GlobalUnlock(memory) };
    Some(bytes)
}

/// Текст из уже открытого буфера.
fn read_text() -> Option<String> {

    let handle = unsafe { GetClipboardData(CF_UNICODETEXT) }.ok()?;
    let memory = HGLOBAL(handle.0);

    let pointer = unsafe { GlobalLock(memory) }.cast::<u16>();
    if pointer.is_null() {
        return None;
    }

    // Даже чужой блок на сотни мегабайт просматриваем только до потолка:
    // один знак может занимать пару UTF-16, плюс место под завершающий ноль.
    let limit = (unsafe { GlobalSize(memory) } / size_of::<u16>()).min(super::MAX_CHARS * 2 + 1);
    let wide = unsafe { std::slice::from_raw_parts(pointer, limit) };
    let text = super::bounded_text_length(wide).map(|length| String::from_utf16_lossy(&wide[..length]));
    let _ = unsafe { GlobalUnlock(memory) };

    text
}

/// Кладёт текст в буфер.
///
/// ВАЖНО: владельцем открываем окно виджета, а не `None`. С пустым владельцем
/// `EmptyClipboard` сбрасывает владельца буфера в ноль, и `SetClipboardData`
/// вправе после этого отказать — так написано в документации `EmptyClipboard`.
pub fn write(text: &str, owner: Option<HWND>) -> Result<(), String> {
    let _open = Open::new(owner)?;

    unsafe { EmptyClipboard() }.map_err(|error| format!("буфер не очистился: {error}"))?;

    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();

    set_data(CF_UNICODETEXT, &wide)
}

/// Кладёт картинку: CF_DIB для всех и PNG для тех, кто умеет прозрачность.
/// Остальные виды (CF_BITMAP, CF_DIBV5) система выводит из CF_DIB сама.
pub fn write_image(png: &[u8], dib: &[u8], owner: Option<HWND>) -> Result<(), String> {
    let format = format_id(w!("PNG")).ok_or_else(|| "не зарегистрировать формат PNG".to_owned())?;
    let _open = Open::new(owner)?;

    unsafe { EmptyClipboard() }.map_err(|error| format!("буфер не очистился: {error}"))?;

    set_data(CF_DIB, dib)?;
    set_data(format, png)
}

/// Кладёт секрет с запретом истории, облачного буфера и обработки мониторами.
/// Пометки публикуются до текста под одним открытием: отказ не оставит
/// в буфере секрет без защиты. Обычного запасного копирования здесь нет.
pub fn write_secret(text: &str, owner: HWND) -> Result<(), String> {
    let formats = [
        format_id(w!("ExcludeClipboardContentFromMonitorProcessing")),
        format_id(w!("CanIncludeInClipboardHistory")),
        format_id(w!("CanUploadToCloudClipboard")),
    ];
    let formats = formats.map(|id| id.ok_or_else(|| "не зарегистрировать защитный формат буфера".to_owned()));
    let [exclude, history, cloud] = formats;
    let formats = [exclude?, history?, cloud?];
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let _open = Open::new(Some(owner))?;
    unsafe { EmptyClipboard() }.map_err(|error| format!("буфер не очистился: {error}"))?;
    for format in formats {
        set_data(format, &[0u32])?;
    }
    set_data(CF_UNICODETEXT, &wide)
}

/// Передаёт буферу отдельный блок памяти; при отказе владение остаётся у нас.
fn set_data<T: Copy>(format: u32, data: &[T]) -> Result<(), String> {
    // GMEM_MOVEABLE обязателен: буфер принимает только перемещаемую память
    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, std::mem::size_of_val(data)) }
        .map_err(|error| format!("не выделить память под данные буфера: {error}"))?;
    let pointer = unsafe { GlobalLock(memory) }.cast::<T>();
    if pointer.is_null() {
        let _ = unsafe { GlobalFree(Some(memory)) };
        return Err("память под данные буфера не заперлась".to_owned());
    }
    unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), pointer, data.len()) };
    let _ = unsafe { GlobalUnlock(memory) };

    match unsafe { SetClipboardData(format, Some(HANDLE(memory.0))) } {
        Ok(_) => Ok(()),
        Err(error) => {
            let _ = unsafe { GlobalFree(Some(memory)) };
            Err(format!("буфер не принял данные: {error}"))
        }
    }
}

/// Опустошает буфер.
pub fn empty(owner: Option<HWND>) -> Result<(), String> {
    let _open = Open::new(owner)?;

    unsafe { EmptyClipboard() }.map_err(|error| format!("буфер не очистился: {error}"))
}

/// Помечено ли содержимое как «не для истории».
///
/// Два формата, которые ставят менеджеры паролей — KeePass, Bitwarden,
/// 1Password. Системная история буфера (Win+V) живёт по этим же пометкам, и
/// виджету, висящему на видном мониторе, они нужны ровно так же.
///
/// Зовётся при открытом буфере: за значением второго формата приходится
/// сходить в `GetClipboardData`.
fn excluded() -> bool {
    // Первый несёт смысл одним своим присутствием
    if let Some(format) = format_id(w!("ExcludeClipboardContentFromMonitorProcessing")) {
        if unsafe { IsClipboardFormatAvailable(format) }.is_ok() {
            return true;
        }
    }

    // А второй — значением: ноль запрещает, единица разрешает
    match format_id(w!("CanIncludeInClipboardHistory")).and_then(read_dword) {
        Some(value) => value == 0,
        None => false,
    }
}

/// Номер зарегистрированного формата. Ноль означает отказ регистрации.
fn format_id(name: PCWSTR) -> Option<u32> {
    let id = unsafe { RegisterClipboardFormatW(name) };
    (id != 0).then_some(id)
}

/// Читает формат, содержимое которого — одно четырёхбайтовое число.
fn read_dword(format: u32) -> Option<u32> {
    if unsafe { IsClipboardFormatAvailable(format) }.is_err() {
        return None;
    }

    let handle = unsafe { GetClipboardData(format) }.ok()?;
    let memory = HGLOBAL(handle.0);
    if unsafe { GlobalSize(memory) } < size_of::<u32>() {
        return None;
    }

    let pointer = unsafe { GlobalLock(memory) }.cast::<u32>();
    if pointer.is_null() {
        return None;
    }
    let value = unsafe { *pointer };
    let _ = unsafe { GlobalUnlock(memory) };

    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Живой круг: записали — прочитали — очистили.
    ///
    /// Проверяет ровно то, что тестом на чистой логике не проверить:
    /// `GlobalAlloc` с `GMEM_MOVEABLE`, передачу владения памятью буферу и
    /// поиск завершающего нуля при чтении. Заодно ловит смену номера
    /// последовательности — на ней держится всё «своя запись не в списке».
    ///
    /// ВАЖНО: **всё это одним тестом, а не тремя.** Буфер обмена один на всю
    /// систему и открывается ровно одним потоком за раз, а `cargo test` гоняет
    /// тесты параллельно: тремя тестами они отбирали буфер друг у друга, и
    /// падало то чтение (открыть не вышло), то очистка (`EmptyClipboard` на
    /// уже закрытом буфере). Повторы в [`Open`] эту драку только растягивают.
    ///
    /// Запуск: `cargo test --manifest-path src-tauri/Cargo.toml -- --ignored`
    #[test]
    #[ignore = "трогает живой буфер обмена"]
    fn text_survives_the_round_trip() {
        let before = sequence();
        let sample = "Проверка буфера — ёлка, ß, 漢字\nи вторая строка";

        write(sample, None).expect("буфер принял текст");

        assert_eq!(read_text_now().as_deref(), Some(sample));
        assert_ne!(sequence(), before, "номер последовательности обязан меняться");

        // Пустая строка — это тоже строка: буфер обязан принять её и вернуть.
        // В историю она не попадёт, но это решает `mod.rs`, а не здесь.
        write("", None).expect("буфер принял пустую строку");
        assert_eq!(read_text_now().as_deref(), Some(""));

        // А после очистки текста нет вовсе
        empty(None).expect("буфер очистился");
        assert!(matches!(read(), Ok(Content::Nothing)));
    }

    fn read_text_now() -> Option<String> {
        match read().expect("буфер открылся") {
            Content::Text(text) => Some(text),
            _ => None,
        }
    }
}
