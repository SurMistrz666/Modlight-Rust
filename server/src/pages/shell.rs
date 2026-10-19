//! The SPA shell: `web/dist/index.html` with the first page's data inlined.
//!
//! For every page URL the server answers with the built `index.html` and
//! - the `<head>` upstream's metadata would produce (title, description, Open
//!   Graph, Twitter, oEmbed discovery, icons), replacing the template's
//!   `<title>`;
//! - `lang`, `data-mode` and `data-theme` on `<html>`, the attributes the root
//!   layout rendered from cookies so the first frame is already themed;
//! - the page payload as `<script id="__MODLIGHT_PAGE__" type="application/json">`
//!   (escaped so it cannot end the script), right before `</body>`.
//!
//! The file is re-read whenever its modification time changes, so a rebuilt
//! SPA is picked up without a restart. Without a build, a minimal placeholder
//! page is served (the payload is still there, which is what tests read).

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

use crate::pages::meta::escape_attr;

/// Served when `web/dist/index.html` does not exist yet.
pub const PLACEHOLDER_INDEX: &str = "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\"/>\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"/>\n<title>Modlight</title>\n</head>\n<body>\n<div id=\"root\"></div>\n<noscript>Modlight needs JavaScript.</noscript>\n</body>\n</html>\n";

pub struct SpaShell {
    path: PathBuf,
    cache: Mutex<Option<(SystemTime, String)>>,
}

/// Everything injected into one response.
pub struct ShellParts<'a> {
    pub lang: &'a str,
    pub data_mode: &'a str,
    pub data_theme: &'a str,
    /// What `<ThemeApplier>`'s blocking script would have set on `<html>` for
    /// a wallpaper: the `style` value (`--ml-bg-image:url("..");--ml-bg-dim:0.7`)
    /// and `data-bg="on"`. `None` for pages without one.
    pub background_style: Option<&'a str>,
    /// Rendered head tags ([`crate::pages::meta::render_head`]).
    pub head: &'a str,
    /// The payload, already escaped for HTML ([`crate::rpc::tagged::json_for_html`]).
    pub payload_json: &'a str,
}

impl SpaShell {
    pub fn new(path: PathBuf) -> SpaShell {
        SpaShell { path, cache: Mutex::new(None) }
    }

    /// Whether a real SPA build is present.
    pub fn is_built(&self) -> bool {
        self.path.is_file()
    }

    fn template(&self) -> String {
        let Ok(meta) = std::fs::metadata(&self.path) else {
            return PLACEHOLDER_INDEX.to_string();
        };
        let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, text)) = cache.as_ref()
            && *at == modified
        {
            return text.clone();
        }
        match std::fs::read_to_string(&self.path) {
            Ok(text) => {
                *cache = Some((modified, text.clone()));
                text
            }
            Err(err) => {
                tracing::warn!("could not read {}: {err}", self.path.display());
                PLACEHOLDER_INDEX.to_string()
            }
        }
    }

    /// The shell with `parts` injected.
    pub fn render(&self, parts: &ShellParts<'_>) -> String {
        inject(&self.template(), parts)
    }
}

/// Byte offset of the first case-insensitive match of `needle`.
fn find_ci(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() || h.len() < n.len() {
        return None;
    }
    (from..=h.len() - n.len()).find(|&i| h[i..i + n.len()].eq_ignore_ascii_case(n))
}

/// Set (or replace) attributes on the `<html ...>` start tag.
fn set_html_attrs(html: &str, attrs: &[(&str, &str)]) -> String {
    let Some(start) = find_ci(html, "<html", 0) else {
        return html.to_string();
    };
    let Some(end_rel) = html[start..].find('>') else {
        return html.to_string();
    };
    let end = start + end_rel;
    let tag = &html[start..end];
    let mut rebuilt = tag.to_string();
    for (name, value) in attrs {
        // Remove an existing attribute of the same name (quoted value).
        for quote in ['"', '\''] {
            let pattern = format!(" {name}={quote}");
            if let Some(pos) = find_ci(&rebuilt, &pattern, 0) {
                let value_start = pos + pattern.len();
                if let Some(close) = rebuilt[value_start..].find(quote) {
                    rebuilt.replace_range(pos..value_start + close + 1, "");
                }
            }
        }
        rebuilt.push_str(&format!(" {name}=\"{}\"", escape_attr(value)));
    }
    format!("{}{}{}", &html[..start], rebuilt, &html[end..])
}

/// Inject everything into a template.
pub fn inject(template: &str, parts: &ShellParts<'_>) -> String {
    let mut attrs = vec![("lang", parts.lang), ("data-mode", parts.data_mode), ("data-theme", parts.data_theme)];
    if let Some(style) = parts.background_style {
        attrs.push(("style", style));
        attrs.push(("data-bg", "on"));
    }
    let mut html = set_html_attrs(template, &attrs);
    // Drop the template's own <title>: the metadata brings the real one.
    if let Some(t_start) = find_ci(&html, "<title", 0)
        && let Some(t_end) = find_ci(&html, "</title>", t_start)
    {
        html.replace_range(t_start..t_end + "</title>".len(), "");
    }
    match find_ci(&html, "</head>", 0) {
        Some(pos) => html.insert_str(pos, parts.head),
        None => html.insert_str(0, parts.head),
    }
    let script = format!("<script id=\"__MODLIGHT_PAGE__\" type=\"application/json\">{}</script>", parts.payload_json);
    match find_ci(&html, "</body>", 0) {
        Some(pos) => html.insert_str(pos, &script),
        None => html.push_str(&script),
    }
    html
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injects_attributes_head_and_payload() {
        let template = "<!doctype html><html lang=\"en\" class=\"h-full\"><head><meta charset=\"utf-8\"><title>Vite</title></head><body><div id=\"root\"></div></body></html>";
        let out = inject(
            template,
            &ShellParts {
                lang: "pl",
                data_mode: "dark",
                data_theme: "glass",
                background_style: None,
                head: "<title>Sign in - Modlight</title>",
                payload_json: "{\"route\":\"/login\"}",
            },
        );
        assert_eq!(
            out,
            "<!doctype html><html class=\"h-full\" lang=\"pl\" data-mode=\"dark\" data-theme=\"glass\"><head><meta charset=\"utf-8\"><title>Sign in - Modlight</title></head><body><div id=\"root\"></div><script id=\"__MODLIGHT_PAGE__\" type=\"application/json\">{\"route\":\"/login\"}</script></body></html>"
        );
    }

    #[test]
    fn wallpaper_lands_on_html_like_theme_applier_sets_it() {
        let out = inject(
            "<html class=\"h-full\"><head></head><body></body></html>",
            &ShellParts {
                lang: "en",
                data_mode: "system",
                data_theme: "paper",
                background_style: Some("--ml-bg-image:url(\"/bg/bob?v=t\");--ml-bg-dim:0.7"),
                head: "",
                payload_json: "{}",
            },
        );
        assert!(out.starts_with("<html class=\"h-full\" lang=\"en\" data-mode=\"system\" data-theme=\"paper\" style=\"--ml-bg-image:url(&quot;/bg/bob?v=t&quot;);--ml-bg-dim:0.7\" data-bg=\"on\">"));
    }

    #[test]
    fn placeholder_is_used_without_a_build() {
        let shell = SpaShell::new(PathBuf::from("/nonexistent/index.html"));
        assert!(!shell.is_built());
        let out = shell.render(&ShellParts {
            lang: "en",
            data_mode: "system",
            data_theme: "discord",
            background_style: None,
            head: "",
            payload_json: "{}",
        });
        assert!(out.contains("<html lang=\"en\" data-mode=\"system\" data-theme=\"discord\">"));
        assert!(out.contains("id=\"__MODLIGHT_PAGE__\""));
    }
}
