//! UTF-16 strings for the wide (`W`) Win32 calls.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

/// `text` as a NUL-terminated UTF-16 string.
pub fn wide(text: impl AsRef<OsStr>) -> Vec<u16> {
    text.as_ref()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// A UTF-16 buffer up to its first NUL (or its end) as a `String`.
pub fn from_wide(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

/// A NUL-terminated UTF-16 string at `pointer` as a `String` (empty for null).
///
/// # Safety
/// `pointer` is null or points at a NUL-terminated UTF-16 string.
pub unsafe fn from_wide_ptr(pointer: *const u16) -> String {
    if pointer.is_null() {
        return String::new();
    }
    let mut length = 0;
    // SAFETY: the caller promises a NUL-terminated string.
    unsafe {
        while *pointer.add(length) != 0 {
            length += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(pointer, length))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trip() {
        let w = super::wide("Zdjęcie ż");
        assert_eq!(*w.last().unwrap(), 0);
        assert_eq!(super::from_wide(&w), "Zdjęcie ż");
        // SAFETY: `w` is NUL-terminated.
        assert_eq!(unsafe { super::from_wide_ptr(w.as_ptr()) }, "Zdjęcie ż");
    }
}
