//! Port of `lib/ids.ts`: short-code generation and random tokens, from the
//! operating system's CSPRNG (upstream: `crypto.randomInt` / `randomBytes`),
//! so public URLs are not enumerable.
//!
//! Codes alternate consonant/vowel for readability and end with a few
//! digits, e.g. "Bocugavi42".

use base64::Engine;
use rand::Rng;
use rand::rngs::OsRng;

const CONSONANTS: &[u8] = b"bcdfghjklmnpqrstvwxyzBCDFGHJKLMNPQRSTVWXYZ"; // 42 chars
const VOWELS: &[u8] = b"aeiouAEIOU"; // 10 chars
const DIGITS: &[u8] = b"0123456789";

fn pick(alphabet: &[u8]) -> char {
    alphabet[OsRng.gen_range(0..alphabet.len())] as char
}

/// Pronounceable, hard-to-guess code. Default length 10 (8 letters + 2
/// digits), the media-code shape ShareX links use.
pub fn human_code(length: usize) -> String {
    // Math.round(length * 0.2): .5 rounds up.
    let digit_count = ((length as f64) * 0.2 + 0.5).floor() as usize;
    let letter_count = length - digit_count.min(length);
    let mut out = String::with_capacity(length);
    for i in 0..letter_count {
        out.push(if i % 2 == 1 { pick(VOWELS) } else { pick(CONSONANTS) });
    }
    for _ in 0..digit_count.min(length) {
        out.push(pick(DIGITS));
    }
    out
}

/// Short per-user code used as the first URL segment (default length 6).
pub fn user_code(length: usize) -> String {
    (0..length).map(|i| if i % 2 == 1 { pick(VOWELS) } else { pick(CONSONANTS) }).collect()
}

/// Vanity slug sanitizer: lowercases, collapses runs of non-alphanumerics to a
/// single dash, trims dashes.
pub fn slugify(input: &str) -> String {
    let lower = input.to_lowercase();
    let mut out = String::new();
    let mut pending_dash = false;
    for c in lower.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(c);
        } else {
            pending_dash = true;
        }
    }
    out
}

/// URL-safe, high-entropy secret (API tokens, deletion tokens, etc.):
/// `randomBytes(bytes).toString("base64url")`.
pub fn secure_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    OsRng.fill(buf.as_mut_slice());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    // tests/unit/ids.test.ts covers the shapes; these pin the same rules.
    #[test]
    fn human_code_shape() {
        let c = human_code(10);
        assert_eq!(c.len(), 10);
        assert!(c[8..].chars().all(|ch| ch.is_ascii_digit()));
        assert!(c[..8].chars().all(|ch| ch.is_ascii_alphabetic()));
        let vowels: Vec<char> = c[..8].chars().skip(1).step_by(2).collect();
        assert!(vowels.iter().all(|v| "aeiouAEIOU".contains(*v)));
        assert_eq!(human_code(5).len(), 5);
    }

    #[test]
    fn user_code_and_slug() {
        assert_eq!(user_code(6).len(), 6);
        assert_eq!(slugify("  Hello, World!  "), "hello-world");
        assert_eq!(slugify("Zażółć gęślą"), "za-g-l");
        assert_eq!(slugify("---"), "");
    }

    // tests/unit/ids.test.ts
    #[test]
    fn ids_like_upstream() {
        let c = human_code(10);
        assert_eq!(c.len(), 10);
        assert!(!c.contains('.'));
        assert!(c[8..].chars().all(|ch| ch.is_ascii_digit()));
        let set: std::collections::HashSet<String> = (0..500).map(|_| human_code(10)).collect();
        assert!(set.len() > 495);
        assert_eq!(slugify("Hello World!!"), "hello-world");
        assert_eq!(slugify("  a__b  "), "a-b");
        assert_eq!(slugify("Łódź 2024"), "d-2024");
    }

    #[test]
    fn secure_token_is_base64url() {
        let t = secure_token(24);
        assert_eq!(t.len(), 32);
        assert!(t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }
}
