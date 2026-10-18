//! Translations on the server (PORT.md section 11).
//!
//! `messages/` is shared with the SPA. The server reads it once at startup and
//! uses it where upstream translates on the server: the root layout payload,
//! page metadata, mail and error pages.
//!
//! - [`request`] is the port of `i18n/request.ts` (locale resolution and the
//!   namespaces the root layout hands to the client).
//! - [`format`] formats ICU messages the way next-intl (intl-messageformat)
//!   does for the syntax the dictionaries use.

pub mod format;
pub mod request;

use std::path::Path;

use anyhow::Context;
use indexmap::IndexMap;
use serde_json::Value;

pub use format::{Arg, format_message};
pub use request::{LOCALES, Locale, NAMESPACES, resolve_locale};

/// Every dictionary of every locale: `locale -> namespace -> JSON`.
#[derive(Debug, Default)]
pub struct Messages {
    by_locale: IndexMap<String, IndexMap<String, Value>>,
}

impl Messages {
    /// Load `<dir>/<locale>/<namespace>.json` for every supported locale.
    pub fn load(dir: &Path) -> anyhow::Result<Messages> {
        let mut by_locale = IndexMap::new();
        for locale in LOCALES {
            let locale_dir = dir.join(locale.as_str());
            let mut files: Vec<_> = std::fs::read_dir(&locale_dir)
                .with_context(|| format!("reading {}", locale_dir.display()))?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "json"))
                .collect();
            files.sort();
            let mut namespaces = IndexMap::new();
            for file in files {
                let ns = file.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
                let text = std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
                let value: Value =
                    serde_json::from_str(&text).with_context(|| format!("parsing {}", file.display()))?;
                namespaces.insert(ns, value);
            }
            by_locale.insert(locale.as_str().to_string(), namespaces);
        }
        Ok(Messages { by_locale })
    }

    /// One namespace of one locale.
    pub fn namespace(&self, locale: Locale, namespace: &str) -> Option<&Value> {
        self.by_locale.get(locale.as_str()).and_then(|m| m.get(namespace))
    }

    /// The object `getMessages()` returns in the root layout: the namespaces of
    /// [`NAMESPACES`] in that order (mail.json is server-only, as upstream).
    pub fn client_messages(&self, locale: Locale) -> Value {
        let mut out = serde_json::Map::new();
        for ns in NAMESPACES {
            if let Some(v) = self.namespace(locale, ns) {
                out.insert((*ns).to_string(), v.clone());
            }
        }
        Value::Object(out)
    }

    /// Raw message text for `namespace` + dotted `key`, if it exists.
    pub fn raw(&self, locale: Locale, namespace: &str, key: &str) -> Option<&str> {
        let mut node = self.namespace(locale, namespace)?;
        for part in key.split('.') {
            node = node.get(part)?;
        }
        node.as_str()
    }

    /// next-intl `getTranslations(namespace)(key, values)`: the formatted
    /// message, or `namespace.key` when it is missing (next-intl's fallback).
    pub fn t(&self, locale: Locale, namespace: &str, key: &str, args: &[(&str, Arg)]) -> String {
        match self.raw(locale, namespace, key) {
            Some(text) => match format::try_format_message(text, locale, args) {
                Ok(s) => s,
                Err(err) => {
                    tracing::error!("[i18n] {err} in {namespace}.{key} ({})", locale.as_str());
                    format!("{namespace}.{key}")
                }
            },
            None => {
                tracing::error!("[i18n] missing message {namespace}.{key} ({})", locale.as_str());
                format!("{namespace}.{key}")
            }
        }
    }

    /// A translator bound to one locale and namespace.
    pub fn translator(&self, locale: Locale, namespace: &'static str) -> Translator<'_> {
        Translator { messages: self, locale, namespace }
    }
}

/// `const t = await getTranslations("auth")`.
#[derive(Clone, Copy)]
pub struct Translator<'a> {
    messages: &'a Messages,
    pub locale: Locale,
    pub namespace: &'static str,
}

impl Translator<'_> {
    pub fn t(&self, key: &str) -> String {
        self.messages.t(self.locale, self.namespace, key, &[])
    }

    pub fn t_with(&self, key: &str, args: &[(&str, Arg)]) -> String {
        self.messages.t(self.locale, self.namespace, key, args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Paths;

    #[test]
    fn loads_the_shipped_dictionaries() {
        let m = Messages::load(&Paths::crate_dir().messages).unwrap();
        assert_eq!(m.raw(Locale::En, "auth", "login.title"), Some("Sign in"));
        let client = m.client_messages(Locale::Pl);
        let keys: Vec<&String> = client.as_object().unwrap().keys().collect();
        assert_eq!(keys, NAMESPACES.iter().collect::<Vec<_>>());
        assert!(client.get("mail").is_none());
        assert_eq!(m.t(Locale::En, "auth", "no.such.key", &[]), "auth.no.such.key");
    }

    /// tests/unit/i18n-parity.test.ts in spirit: both locales carry the same keys.
    #[test]
    fn locales_have_the_same_keys() {
        fn keys(prefix: &str, v: &Value, out: &mut Vec<String>) {
            if let Some(obj) = v.as_object() {
                for (k, v) in obj {
                    keys(&format!("{prefix}.{k}"), v, out);
                }
            } else {
                out.push(prefix.to_string());
            }
        }
        let m = Messages::load(&Paths::crate_dir().messages).unwrap();
        let (mut en, mut pl) = (Vec::new(), Vec::new());
        for (ns, v) in &m.by_locale["en"] {
            keys(ns, v, &mut en);
        }
        for (ns, v) in &m.by_locale["pl"] {
            keys(ns, v, &mut pl);
        }
        en.sort();
        pl.sort();
        assert_eq!(en, pl);
    }
}
