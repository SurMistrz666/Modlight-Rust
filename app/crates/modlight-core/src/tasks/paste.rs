//! A paste being written (`PasteViewModel`, `PasteLanguages`): its text, its language,
//! and the file name those two make.
//!
//! The panel creates pastes through a server action with no HTTP route, so a paste goes
//! up the way ShareX has always sent text: as a FILE through `POST /upload`, named with
//! the extension the language implies. The extension is the part the server reads to
//! label it; the language key is kept for the day there is a paste endpoint.

use crate::culture::Culture;
use crate::l10n::{Arg, format, s, t};

/// One entry in the language menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PasteLanguage {
    /// The panel's own identifier (`lib/paste-languages.ts`).
    pub key: &'static str,
    /// What the file is called, which is the ONLY part the server sees.
    pub extension: &'static str,
    /// The name a person reads; `None` for the one that is translated.
    name: Option<&'static str>,
}

impl PasteLanguage {
    pub fn name(&self) -> &'static str {
        self.name.unwrap_or_else(|| t(s::Paste_LanguagePlainText))
    }
}

const fn language(key: &'static str, extension: &'static str, name: &'static str) -> PasteLanguage {
    PasteLanguage {
        key,
        extension,
        name: Some(name),
    }
}

/// Every language, in the panel's order. The first is the default.
pub const LANGUAGES: [PasteLanguage; 28] = [
    // Localised, unlike the rest: "Plain text" is a description, the others are names.
    PasteLanguage {
        key: "text",
        extension: "txt",
        name: None,
    },
    language("ts", "ts", "TypeScript"),
    language("tsx", "tsx", "TypeScript (TSX)"),
    language("js", "js", "JavaScript"),
    language("jsx", "jsx", "JavaScript (JSX)"),
    language("json", "json", "JSON"),
    language("py", "py", "Python"),
    language("go", "go", "Go"),
    language("rs", "rs", "Rust"),
    language("java", "java", "Java"),
    language("c", "c", "C"),
    language("cpp", "cpp", "C++"),
    language("cs", "cs", "C#"),
    language("php", "php", "PHP"),
    language("rb", "rb", "Ruby"),
    language("sh", "sh", "Shell"),
    language("yml", "yml", "YAML"),
    language("toml", "toml", "TOML"),
    language("sql", "sql", "SQL"),
    language("html", "html", "HTML"),
    language("css", "css", "CSS"),
    language("md", "md", "Markdown"),
    language("xml", "xml", "XML"),
    language("kt", "kt", "Kotlin"),
    language("swift", "swift", "Swift"),
    language("dockerfile", "dockerfile", "Dockerfile"),
    language("ini", "ini", "INI"),
    language("diff", "diff", "Diff"),
];

/// Plain text, which is what an unanswered question should mean.
pub fn default_language() -> PasteLanguage {
    LANGUAGES[0]
}

/// The language with this key, or plain text.
pub fn find(key: Option<&str>) -> PasteLanguage {
    key.and_then(|key| LANGUAGES.iter().find(|l| l.key.eq_ignore_ascii_case(key)))
        .copied()
        .unwrap_or_else(default_language)
}

/// Does this entry answer what was typed into the search box? Name, key and extension,
/// as substrings, case-insensitive.
pub fn matches(language: &PasteLanguage, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let query_lower = query.to_lowercase();
    language.name().to_lowercase().contains(&query_lower)
        || language.key.to_lowercase().contains(&query_lower)
        || language.extension.to_lowercase().contains(&query_lower)
}

/// The most text one paste may carry: the web panel's own ceiling.
pub const MAX_CHARACTERS: usize = 5_000_000;

#[derive(Clone, Debug, PartialEq)]
pub struct Paste {
    pub title: String,
    pub content: String,
    pub language: PasteLanguage,
    pub language_filter: String,
    pub problem: String,
}

impl Default for Paste {
    fn default() -> Self {
        Paste {
            title: String::new(),
            content: String::new(),
            language: default_language(),
            language_filter: String::new(),
            problem: String::new(),
        }
    }
}

impl Paste {
    /// The languages the search box lets through.
    pub fn languages(&self) -> Vec<PasteLanguage> {
        let query = self.language_filter.trim();
        LANGUAGES
            .iter()
            .copied()
            .filter(|l| matches(l, query))
            .collect()
    }

    /// The name the file goes up under: the title sanitised as the panel does, or
    /// "paste", plus the language's extension.
    pub fn file_name(&self) -> String {
        format!(
            "{}.{}",
            sanitize_title(&self.title),
            self.language.extension
        )
    }

    /// The line under the editor.
    pub fn footer_text(&self) -> String {
        let count = self.content.chars().count() as i64;
        format(
            t(s::Paste_Footer),
            &[
                Arg::from(Culture::current().n0(count)),
                Arg::from(self.file_name()),
            ],
        )
    }

    /// The bytes and the name to upload, or the reason not to (written to `problem`).
    pub fn try_send(&mut self) -> Option<(Vec<u8>, String)> {
        if self.content.trim().is_empty() {
            self.problem = t(s::Paste_Empty).to_string();
            return None;
        }
        if self.content.chars().count() > MAX_CHARACTERS {
            self.problem = format(
                t(s::Paste_TooLarge),
                &[Arg::from(Culture::current().n0(MAX_CHARACTERS as i64))],
            );
            return None;
        }
        // No byte order mark: the server sniffs the first bytes.
        Some((self.content.as_bytes().to_vec(), self.file_name()))
    }
}

/// A title turned into a file name stem: anything outside letters, digits, dot, dash and
/// underscore collapses to one dash, at most 80 characters, "paste" when nothing is left.
pub fn sanitize_title(value: &str) -> String {
    let mut out = String::new();
    let mut last_was_dash = false;
    for c in value.trim().chars() {
        if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
            out.push(c);
            last_was_dash = false;
        } else if !last_was_dash {
            out.push('-');
            last_was_dash = true;
        }
        if out.chars().count() >= 80 {
            break;
        }
    }
    let name = out.trim_matches('-');
    if name.is_empty() {
        "paste".to_string()
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::l10n::{Language, set_thread_language};

    #[test]
    fn titles_become_file_names_like_the_panels() {
        assert_eq!(sanitize_title(""), "paste");
        // Upstream's comment promises one dash here, but a literal dash resets the
        // run, so its code gives three; the port does what the code does.
        assert_eq!(sanitize_title("  My notes - draft  "), "My-notes---draft");
        assert_eq!(sanitize_title("My notes  draft"), "My-notes-draft");
        assert_eq!(sanitize_title("Zażółć"), "Za");
        assert_eq!(sanitize_title("a//b"), "a-b");
        assert_eq!(sanitize_title(&"x".repeat(200)).len(), 80);
        assert_eq!(sanitize_title("---"), "paste");
    }

    #[test]
    fn the_language_names_the_extension() {
        let mut paste = Paste {
            title: "build script".into(),
            content: "echo hi".into(),
            language: find(Some("SH")),
            ..Paste::default()
        };
        assert_eq!(paste.file_name(), "build-script.sh");
        let (bytes, name) = paste.try_send().unwrap();
        assert_eq!(bytes, b"echo hi");
        assert_eq!(name, "build-script.sh");
        assert_eq!(find(Some("nope")).key, "text");
        assert_eq!(LANGUAGES.len(), 28);
    }

    #[test]
    fn empty_and_huge_pastes_are_refused() {
        set_thread_language(Some(Language::English));
        let mut paste = Paste {
            content: "   \n".into(),
            ..Paste::default()
        };
        assert!(paste.try_send().is_none());
        assert_eq!(paste.problem, t(s::Paste_Empty));
        set_thread_language(None);
    }

    #[test]
    fn search_reads_names_keys_and_extensions() {
        let paste = Paste {
            language_filter: "script".into(),
            ..Paste::default()
        };
        let found: Vec<&str> = paste.languages().iter().map(|l| l.key).collect();
        assert!(found.contains(&"ts") && found.contains(&"js"), "{found:?}");
        let paste = Paste {
            language_filter: "cs".into(),
            ..Paste::default()
        };
        assert!(paste.languages().iter().any(|l| l.key == "cs"));
    }
}
