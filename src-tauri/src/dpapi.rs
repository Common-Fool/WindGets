//! Шифрование секретов приложения через Windows Data Protection API (DPAPI).
//!
//! Этим живёт всё, что лежит на диске и не должно читаться глазами: пароль
//! почты и ключи виджета 2FA. Модуль стоит на уровне крейта именно поэтому —
//! внутри `mail` он прожил до появления второго хозяина, а две копии кода,
//! который держит секреты, разошлись бы при первой же правке.
//!
//! Секрет хранится зашифрованным ключом текущего пользователя Windows. Флаг
//! `CRYPTPROTECT_LOCAL_MACHINE` намеренно не ставится: расшифровать может
//! только этот же пользователь на этой же машине.
//!
//! ВАЖНО: **обратная сторона этого — потеря вместе с профилем.** Переустановка
//! Windows или смена профиля делает секреты нечитаемыми навсегда. Для пароля
//! почты это терпимо — его перевыпускают; для ключей 2FA значит, что виджет
//! обязан быть вторым экземпляром кодов, а не единственным.
//!
//! Наружу секрет не передаётся никогда: ни в событиях, ни в командах, ни в логах.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;

#[cfg(windows)]
use windows::Win32::Foundation::{HLOCAL, LocalFree};
#[cfg(windows)]
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};
#[cfg(windows)]
use windows::core::PCWSTR;

/// Защитный страж для гарантированного освобождения памяти, выделенной DPAPI через `LocalAlloc`.
#[cfg(windows)]
struct BlobGuard(CRYPT_INTEGER_BLOB);

#[cfg(windows)]
impl Drop for BlobGuard {
    fn drop(&mut self) {
        if !self.0.pbData.is_null() {
            // Освобождаем память, выделенную под выходной блоб
            unsafe {
                let _ = LocalFree(Some(HLOCAL(self.0.pbData as _)));
            }
            self.0.pbData = std::ptr::null_mut();
            self.0.cbData = 0;
        }
    }
}

/// Шифрует строку пароля через DPAPI и возвращает base64 от зашифрованного блоба.
pub fn protect(password: &str) -> Result<String, String> {
    #[cfg(windows)]
    {
        let bytes = password.as_bytes();
        let in_blob = CRYPT_INTEGER_BLOB {
            cbData: bytes.len() as u32,
            pbData: bytes.as_ptr() as *mut u8,
        };
        let mut out_blob = CRYPT_INTEGER_BLOB::default();

        // CRYPTPROTECT_UI_FORBIDDEN: запрет любых всплывающих окон Windows
        // Флаг CRYPTPROTECT_LOCAL_MACHINE НЕ используется: ключ привязан к пользователю
        let status = unsafe {
            CryptProtectData(
                &in_blob,
                PCWSTR::null(),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out_blob,
            )
        };

        status.map_err(|err| format!("ошибка шифрования DPAPI: {err}"))?;
        let guard = BlobGuard(out_blob);

        if guard.0.pbData.is_null() && guard.0.cbData > 0 {
            return Err("DPAPI вернул пустой указатель на данные".to_owned());
        }

        let slice = if guard.0.cbData == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(guard.0.pbData, guard.0.cbData as usize) }
        };

        Ok(BASE64.encode(slice))
    }

    #[cfg(not(windows))]
    {
        // Заглушка для не-Windows окружения
        Ok(BASE64.encode(password.as_bytes()))
    }
}

/// Расшифровывает строку base64 через DPAPI и возвращает исходный пароль.
pub fn unprotect(secret: &str) -> Result<String, String> {
    let encrypted = BASE64
        .decode(secret.trim())
        .map_err(|err| format!("некорректный base64 секрета: {err}"))?;

    #[cfg(windows)]
    {
        let in_blob = CRYPT_INTEGER_BLOB {
            cbData: encrypted.len() as u32,
            pbData: encrypted.as_ptr() as *mut u8,
        };
        let mut out_blob = CRYPT_INTEGER_BLOB::default();

        let status = unsafe {
            CryptUnprotectData(
                &in_blob,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out_blob,
            )
        };

        status.map_err(|err| format!("ошибка расшифровки DPAPI: {err}"))?;
        let guard = BlobGuard(out_blob);

        let slice = if guard.0.cbData == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(guard.0.pbData, guard.0.cbData as usize) }
        };

        String::from_utf8(slice.to_vec())
            .map_err(|err| format!("расшифрованный пароль не является корректным UTF-8: {err}"))
    }

    #[cfg(not(windows))]
    {
        String::from_utf8(encrypted)
            .map_err(|err| format!("расшифрованный пароль не является корректным UTF-8: {err}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn dpapi_round_trip() {
        let password = "not-a-real-password";
        let secret = protect(password).expect("пароль успешно зашифрован");
        assert_ne!(secret, password);
        let decrypted = unprotect(&secret).expect("пароль успешно расшифрован");
        assert_eq!(decrypted, password);
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_empty_password() {
        let password = "";
        let secret = protect(password).expect("пустой пароль зашифрован");
        let decrypted = unprotect(&secret).expect("пустой пароль расшифрован");
        assert_eq!(decrypted, password);
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_long_password() {
        // Длинный пароль в несколько килобайт
        let password = "A".repeat(8192);
        let secret = protect(&password).expect("длинный пароль зашифрован");
        let decrypted = unprotect(&secret).expect("длинный пароль расшифрован");
        assert_eq!(decrypted, password);
    }

    #[test]
    fn invalid_base64_returns_error() {
        let bad = "not a valid base64 !!!";
        let result = unprotect(bad);
        assert!(result.is_err(), "испорченный base64 должен возвращать Err");
    }
}
