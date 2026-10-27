//! Port of `lib/theme-api.ts`: the wire contract for GET/PUT /api/theme.
//!
//! The desktop client owns the same setting the panel's picker owns, so both
//! ends have to agree on what a valid skin id is and on what a refusal looks
//! like. Pure: no database, no framework, so the route handler and the tests
//! read the same rules instead of one re-deriving them from the other.

use axum::http::StatusCode;
use serde::Serialize;

use crate::js_json::{self, JsValue};
pub use crate::themes::DEFAULT_THEME;
use crate::themes::{THEME_ALIASES, THEMES};

/// Ceiling on the request body.
///
/// The only field is a skin id of at most a dozen characters, so 4 KiB is
/// already absurdly generous - and without a ceiling the body is buffered
/// into memory before there is anything to validate. Same reasoning as the
/// short-link body cap in app/api/shortlinks/route.ts.
pub const THEME_BODY_MAX_BYTES: usize = 4 * 1024;

/// One entry of the picker a client renders.
///
/// `name`/`description` are the ENGLISH FALLBACKS from lib/themes.ts. The
/// panel translates them through `profile.themes.<id>.{name,desc}` and a
/// client is free to do the same keyed on `id`; shipping the English text
/// anyway means a client that has no dictionary for a skin added after its
/// release still shows something a human can read, rather than a raw id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ThemeChoice {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
}

/// Every skin, in picker order.
///
/// Served on every GET so a client never hardcodes the list: a hardcoded copy
/// in a shipped desktop binary goes stale the moment the server is updated,
/// which shows up as a skin the website uses and the app cannot name.
pub fn theme_catalog() -> Vec<ThemeChoice> {
    THEMES.iter().map(|t| ThemeChoice { id: t.id, name: t.name, description: t.description }).collect()
}

/// Refusal for a skin id we do not have, listing the ones we do.
pub fn unknown_theme_message() -> String {
    let ids: Vec<&str> = THEMES.iter().map(|t| t.id).collect();
    format!("Unknown theme. Known themes: {}.", ids.join(", "))
}

/// `ThemeBody`: the validated id, or a refusal with its status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeBody {
    Ok { theme: &'static str },
    Refused { status: StatusCode, message: String },
}

fn refused(message: impl Into<String>) -> ThemeBody {
    ThemeBody::Refused { status: StatusCode::BAD_REQUEST, message: message.into() }
}

/// Validate a PUT body: `{"theme": "<id>"}`.
///
/// Error-as-data rather than thrown strings: the caller needs the status code
/// as much as the sentence.
///
/// The important line is the last one, and it is a REFUSAL rather than a
/// normalisation. `resolve_theme`, the function every read path uses, maps
/// anything it does not recognise onto the default, which is exactly right
/// when painting a page from a column that may hold a value written years
/// ago, and exactly wrong here: it would accept `{"theme": "hunter2"}` with a
/// 200 and silently store the default, so a client with a typo in its picker
/// would report success while resetting the user's skin on every save.
///
/// A RETIRED id is accepted and canonicalised, because THEME_ALIASES is a
/// promise that a renamed skin keeps working forever. `modern` means
/// `default` and is stored as such. Membership of the alias map is checked
/// as an own property, so `constructor` and friends stay garbage.
///
/// The body goes through `JSON.parse` semantics ([`js_json::parse`]): no
/// nesting limit and lone surrogate escapes accepted, so what the reference
/// calls "not a string" or "unknown" is never called "not JSON" here.
pub fn parse_theme_body(text: &str) -> ThemeBody {
    if crate::config::js_trim(text).is_empty() {
        return refused(r#"Body must be JSON: {"theme": "<id>"}."#);
    }

    let Ok(body) = js_json::parse(text) else {
        return refused("Body is not valid JSON.");
    };

    let JsValue::Object(body) = &body else {
        return refused(r#"Body must be a JSON object: {"theme": "<id>"}."#);
    };

    let Some(JsValue::String(raw)) = body.get("theme") else {
        return refused("`theme` is required and must be a string.");
    };

    let value = raw.trim().to_string_lossy();
    if let Some(t) = THEMES.iter().find(|t| t.id == value) {
        return ThemeBody::Ok { theme: t.id };
    }

    if let Some((_, current)) = THEME_ALIASES.iter().find(|(retired, _)| *retired == value) {
        return ThemeBody::Ok { theme: current };
    }

    refused(unknown_theme_message())
}

#[cfg(test)]
mod tests {
    //! tests/unit/theme-api.test.ts
    use super::*;
    use crate::themes::is_theme_id;

    fn body(v: serde_json::Value) -> String {
        v.to_string()
    }

    // ── theme catalogue ──

    #[test]
    fn offers_every_skin_the_panel_offers_in_the_same_order() {
        let ids: Vec<&str> = theme_catalog().iter().map(|t| t.id).collect();
        let panel: Vec<&str> = THEMES.iter().map(|t| t.id).collect();
        assert_eq!(ids, panel);
    }

    #[test]
    fn carries_readable_text_for_a_skin_the_client_has_never_heard_of() {
        for choice in theme_catalog() {
            assert!(!choice.name.is_empty());
            assert!(!choice.description.is_empty());
        }
    }

    #[test]
    fn names_a_default_that_is_itself_a_valid_choice() {
        assert!(is_theme_id(DEFAULT_THEME));
        assert!(theme_catalog().iter().any(|t| t.id == DEFAULT_THEME));
    }

    // ── parseThemeBody ──

    #[test]
    fn accepts_every_id_the_catalogue_advertises() {
        for t in THEMES {
            assert_eq!(parse_theme_body(&body(serde_json::json!({ "theme": t.id }))), ThemeBody::Ok { theme: t.id });
        }
    }

    #[test]
    fn refuses_an_unknown_id_instead_of_storing_the_default() {
        let res = parse_theme_body(&body(serde_json::json!({ "theme": "hunter2" })));
        assert_eq!(res, ThemeBody::Refused { status: StatusCode::BAD_REQUEST, message: unknown_theme_message() });
    }

    #[test]
    fn names_the_ids_it_would_have_accepted() {
        for t in THEMES {
            assert!(unknown_theme_message().contains(t.id));
        }
        assert_eq!(
            unknown_theme_message(),
            "Unknown theme. Known themes: graphite, default, glass, synthwave, terminal, paper, win98, nebula, grimoire, cartoon, mono, deco, riso, discord, spotify."
        );
    }

    #[test]
    fn accepts_a_retired_id_and_stores_the_name_it_was_renamed_to() {
        for (retired, current) in THEME_ALIASES {
            assert_eq!(
                parse_theme_body(&body(serde_json::json!({ "theme": retired }))),
                ThemeBody::Ok { theme: current }
            );
        }
    }

    #[test]
    fn does_not_treat_inherited_object_properties_as_aliases() {
        for key in ["constructor", "toString", "__proto__", "hasOwnProperty"] {
            assert!(matches!(parse_theme_body(&body(serde_json::json!({ "theme": key }))), ThemeBody::Refused { .. }));
        }
    }

    #[test]
    fn tolerates_surrounding_whitespace() {
        assert_eq!(
            parse_theme_body(&body(serde_json::json!({ "theme": "  terminal " }))),
            ThemeBody::Ok { theme: "terminal" }
        );
        // JavaScript's trim(), not ASCII's.
        assert_eq!(parse_theme_body("{\"theme\":\"\\u00a0paper\\u2028\"}"), ThemeBody::Ok { theme: "paper" });
    }

    #[test]
    fn refuses_a_theme_that_is_not_a_string() {
        for value in ["0", "1", "true", "null", "{}", r#"["terminal"]"#] {
            assert_eq!(
                parse_theme_body(&format!(r#"{{"theme": {value}}}"#)),
                refused("`theme` is required and must be a string."),
                "{value}"
            );
        }
    }

    #[test]
    fn refuses_a_body_that_is_not_a_json_object() {
        for raw in [r#""terminal""#, "[]", "null", "42", r#"["terminal"]"#] {
            assert_eq!(parse_theme_body(raw), refused(r#"Body must be a JSON object: {"theme": "<id>"}."#), "{raw}");
        }
    }

    #[test]
    fn refuses_an_empty_or_unparseable_body_with_a_400_not_a_crash() {
        for raw in ["", "   "] {
            assert_eq!(parse_theme_body(raw), refused(r#"Body must be JSON: {"theme": "<id>"}."#));
        }
        for raw in ["{", "{theme: terminal}"] {
            assert_eq!(parse_theme_body(raw), refused("Body is not valid JSON."));
        }
    }

    #[test]
    fn ignores_anything_else_in_the_envelope() {
        assert_eq!(
            parse_theme_body(&body(serde_json::json!({ "theme": "paper", "locale": "pl" }))),
            ThemeBody::Ok { theme: "paper" }
        );
    }

    #[test]
    fn reads_bodies_serde_json_would_refuse_the_way_json_parse_does() {
        // Deep nesting parses (JSON.parse is iterative): the refusal is about
        // the field, not the syntax.
        let deep = format!(r#"{{"theme":{}{}}}"#, "[".repeat(2000), "]".repeat(2000));
        assert_eq!(parse_theme_body(&deep), refused("`theme` is required and must be a string."));
        // A lone surrogate is a string, just not a skin.
        assert_eq!(parse_theme_body(r#"{"theme":"\ud800"}"#), refused(unknown_theme_message()));
        // The last of two keys wins.
        assert_eq!(parse_theme_body(r#"{"theme":"x","theme":"win98"}"#), ThemeBody::Ok { theme: "win98" });
    }
}
