//! The interface texts, English and Polish, generated at build time from upstream's
//! `.resx` files, and the `string.Format` that fills them in.
//!
//! The language follows the app setting (`SupportedLanguage`: Automatic, English,
//! Polish). The process-wide choice can be overridden per thread, which is what lets the
//! snapshot tests draw the English and the Polish screen at the same time.

use std::cell::Cell;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::culture::Culture;

mod generated {
    use super::Key;
    include!(concat!(env!("OUT_DIR"), "/strings.rs"));
}

pub use generated::{
    avalonia, capture, editor, effects, helpers, history, mac, port, s, tools, uploaders, windows,
};

/// One entry of the string tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key(pub(crate) usize);

/// The languages the upstream tables carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Language {
    English,
    Polish,
}

impl Language {
    pub fn code(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::Polish => "pl",
        }
    }
}

/// The `Language` setting as the Windows client stores it (`SupportedLanguage`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LanguageSetting {
    Automatic,
    English,
    Polish,
}

impl LanguageSetting {
    pub const ALL: [LanguageSetting; 3] = [
        LanguageSetting::Automatic,
        LanguageSetting::English,
        LanguageSetting::Polish,
    ];

    /// The enum name the settings document carries (StringEnumConverter).
    pub fn as_str(self) -> &'static str {
        match self {
            LanguageSetting::Automatic => "Automatic",
            LanguageSetting::English => "English",
            LanguageSetting::Polish => "Polish",
        }
    }

    pub fn parse(value: &str) -> LanguageSetting {
        match value {
            "English" => LanguageSetting::English,
            "Polish" => LanguageSetting::Polish,
            _ => LanguageSetting::Automatic,
        }
    }

    /// The language actually used: Automatic follows the system's preferred language.
    pub fn resolve(self, system_language: &str) -> Language {
        match self {
            LanguageSetting::English => Language::English,
            LanguageSetting::Polish => Language::Polish,
            LanguageSetting::Automatic => {
                if system_language.to_ascii_lowercase().starts_with("pl") {
                    Language::Polish
                } else {
                    Language::English
                }
            }
        }
    }
}

static LANGUAGE: AtomicU8 = AtomicU8::new(0);

thread_local! {
    static THREAD_LANGUAGE: Cell<Option<Language>> = const { Cell::new(None) };
}

/// Set the language of the whole process.
pub fn set_language(language: Language) {
    LANGUAGE.store(language as u8, Ordering::Relaxed);
}

/// Use `language` on this thread only, whatever the process uses (tests, previews).
pub fn set_thread_language(language: Option<Language>) {
    THREAD_LANGUAGE.with(|cell| cell.set(language));
}

pub fn language() -> Language {
    THREAD_LANGUAGE.with(|cell| cell.get()).unwrap_or_else(|| {
        if LANGUAGE.load(Ordering::Relaxed) == Language::Polish as u8 {
            Language::Polish
        } else {
            Language::English
        }
    })
}

/// The text of `key` in the current language.
pub fn t(key: Key) -> &'static str {
    text_in(key, language())
}

pub fn text_in(key: Key, language: Language) -> &'static str {
    match language {
        Language::English => generated::ENGLISH[key.0],
        Language::Polish => generated::POLISH[key.0],
    }
}

/// `Project:Name` of a key, for diagnostics.
pub fn name_of(key: Key) -> &'static str {
    generated::NAMES[key.0]
}

/// Whether the Polish table has its own text for `key` (otherwise Polish falls back to
/// the English one).
pub fn has_polish(key: Key) -> bool {
    generated::TRANSLATED[key.0]
}

/// Find a key by `Project:Name` (used by tests and by the coverage check).
pub fn lookup(name: &str) -> Option<Key> {
    generated::NAMES.iter().position(|n| *n == name).map(Key)
}

/// `EnumExtensions.GetLocalizedDescription`: the `{Type}_{Value}` text of
/// ShareX.HelpersLib's table when there is one, else the `[Description]` upstream gives
/// the value (`description`), else the value's name.
pub fn enum_text(type_name: &str, value: &str, description: Option<&'static str>) -> String {
    for project in ["ShareX.HelpersLib", "ShareX.ScreenCaptureLib", "ShareX"] {
        if let Some(key) = lookup(&format!("{project}:{type_name}_{value}")) {
            let text = t(key);
            if !text.is_empty() {
                return text.to_string();
            }
        }
    }
    description.unwrap_or(value).to_string()
}

/// How many strings the tables hold.
pub fn count() -> usize {
    generated::COUNT
}

/// One argument of a formatted string.
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Text(String),
    Int(i64),
    Float(f64),
}

impl From<&str> for Arg {
    fn from(value: &str) -> Self {
        Arg::Text(value.to_string())
    }
}

impl From<String> for Arg {
    fn from(value: String) -> Self {
        Arg::Text(value)
    }
}

impl From<&String> for Arg {
    fn from(value: &String) -> Self {
        Arg::Text(value.clone())
    }
}

impl From<i64> for Arg {
    fn from(value: i64) -> Self {
        Arg::Int(value)
    }
}

impl From<i32> for Arg {
    fn from(value: i32) -> Self {
        Arg::Int(value as i64)
    }
}

impl From<usize> for Arg {
    fn from(value: usize) -> Self {
        Arg::Int(value as i64)
    }
}

impl From<u64> for Arg {
    fn from(value: u64) -> Self {
        Arg::Int(value as i64)
    }
}

impl From<f64> for Arg {
    fn from(value: f64) -> Self {
        Arg::Float(value)
    }
}

/// `string.Format(CurrentCulture, text, args)` for the placeholders the tables use:
/// `{0}`, and `{0:N0}` / `{0:0.0}` style numeric formats. Anything else in braces (the
/// tables carry a few literal `{n}`) is left as it is.
pub fn format(template: &str, args: &[Arg]) -> String {
    let culture = Culture::current();
    let mut out = String::with_capacity(template.len() + 16);
    let mut chars = template.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        match c {
            '{' if chars.peek().map(|(_, n)| *n) == Some('{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek().map(|(_, n)| *n) == Some('}') => {
                chars.next();
                out.push('}');
            }
            '{' => {
                let rest = &template[at + 1..];
                let Some(close) = rest.find('}') else {
                    out.push_str(&template[at..]);
                    return out;
                };
                let inside = &rest[..close];
                let (index, spec) = match inside.split_once(':') {
                    Some((index, spec)) => (index, Some(spec)),
                    None => (inside, None),
                };
                match index.trim().parse::<usize>() {
                    Ok(i) if i < args.len() => out.push_str(&format_arg(&args[i], spec, &culture)),
                    _ => {
                        out.push('{');
                        out.push_str(inside);
                        out.push('}');
                    }
                }
                // Skip what was consumed: the inside and the closing brace.
                while let Some((next, _)) = chars.peek() {
                    if *next > at + close + 1 {
                        break;
                    }
                    chars.next();
                }
            }
            other => out.push(other),
        }
    }
    out
}

fn format_arg(arg: &Arg, spec: Option<&str>, culture: &Culture) -> String {
    match (arg, spec) {
        (Arg::Text(text), _) => text.clone(),
        (Arg::Int(value), Some(spec)) if spec.eq_ignore_ascii_case("N0") => culture.n0(*value),
        (Arg::Int(value), Some(spec)) if spec.starts_with("0.") => {
            culture.fixed(*value as f64, spec.len() - 2)
        }
        (Arg::Int(value), _) => value.to_string(),
        (Arg::Float(value), Some(spec)) if spec.eq_ignore_ascii_case("N0") => {
            culture.n0(value.round() as i64)
        }
        (Arg::Float(value), Some(spec)) if spec.starts_with("0.") => {
            culture.fixed(*value, spec.len() - 2)
        }
        (Arg::Float(value), _) => culture.general(*value),
    }
}

/// `t(key)` with its placeholders filled.
pub fn tf(key: Key, args: &[Arg]) -> String {
    format(t(key), args)
}

/// Build an argument list: `args![name, 3, size]`.
#[macro_export]
macro_rules! args {
    ($($value:expr),* $(,)?) => {
        &[$($crate::l10n::Arg::from($value)),*]
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::culture::{Culture, set_thread_culture};

    #[test]
    fn both_tables_are_complete() {
        assert!(count() > 2000);
        for index in 0..count() {
            let key = Key(index);
            // An entry may legitimately be empty in upstream; it must exist in both.
            let _ = text_in(key, Language::English);
            let _ = text_in(key, Language::Polish);
        }
        assert_eq!(text_in(s::MySpace_Trash, Language::English), "Trash");
        assert_ne!(text_in(s::MySpace_Trash, Language::Polish), "Trash");
        assert_eq!(lookup("ShareX:MySpace_Trash"), Some(s::MySpace_Trash));
    }

    #[test]
    fn formats_like_string_format() {
        set_thread_culture(Some(Culture::english()));
        assert_eq!(
            format("{0} of {1}", &[Arg::from("a"), Arg::from(2)]),
            "a of 2"
        );
        assert_eq!(format("{0:N0} files", &[Arg::from(6926)]), "6,926 files");
        assert_eq!(format("{0:0.0} MB", &[Arg::from(5.25)]), "5.3 MB");
        assert_eq!(format("{n} {0} {9}", &[Arg::from("x")]), "{n} x {9}");
        assert_eq!(format("{{0}} {0}", &[Arg::from("x")]), "{0} x");
        set_thread_culture(Some(Culture::polish()));
        assert_eq!(format("{0:N0}", &[Arg::from(6926)]), "6\u{a0}926");
        set_thread_culture(None);
    }

    #[test]
    fn languages_resolve_like_the_windows_setting() {
        assert_eq!(
            LanguageSetting::Automatic.resolve("pl-PL"),
            Language::Polish
        );
        assert_eq!(
            LanguageSetting::Automatic.resolve("en-GB"),
            Language::English
        );
        assert_eq!(LanguageSetting::Polish.resolve("en"), Language::Polish);
        assert_eq!(LanguageSetting::parse("Polish"), LanguageSetting::Polish);
        assert_eq!(
            LanguageSetting::parse("Klingon"),
            LanguageSetting::Automatic
        );
        set_thread_language(Some(Language::Polish));
        assert_eq!(language(), Language::Polish);
        set_thread_language(None);
    }
}
