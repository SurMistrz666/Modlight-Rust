//! Port of `lib/mail-templates.ts`: the transactional emails, as data.
//!
//! Two constraints shape everything below, and neither is negotiable because
//! mail clients enforce them:
//!
//!  - **Tables and inline styles only.** Outlook renders HTML with the Word
//!    engine: no flexbox, no grid, no `<style>` rules it feels like ignoring.
//!  - **Colours as literal hex.** An email has no access to the theme tokens;
//!    the palette below is a deliberate, documented COPY of the "modern" skin.
//!
//! Translations come from `messages/<locale>/mail.json` (the static import
//! upstream): mail is sent on somebody else's behalf, where the request's
//! language cookies do not apply. The output is byte-for-byte upstream's.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::Value;

use crate::i18n::Messages;

// ── Palette (copy of the "modern" skin, dark) ───────────────────────────────
const BG: &str = "#0b0b0c";
const CARD: &str = "#151517";
const BORDER: &str = "#26262a";
const TEXT: &str = "#fafafa";
const MUTED: &str = "#a1a1aa";
const FAINT: &str = "#52525b";
const ACCENT: &str = "#6E6BF5";
const WARN: &str = "#f59e0b";

const FONT: &str = "ui-sans-serif,system-ui,-apple-system,'Segoe UI',Roboto,Helvetica,Arial,sans-serif";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailLocale {
    En,
    Pl,
}

impl MailLocale {
    pub fn as_str(self) -> &'static str {
        match self {
            MailLocale::En => "en",
            MailLocale::Pl => "pl",
        }
    }

    fn ui(self) -> crate::i18n::Locale {
        match self {
            MailLocale::En => crate::i18n::Locale::En,
            MailLocale::Pl => crate::i18n::Locale::Pl,
        }
    }
}

/// `pickMailLocale(preferred, acceptLanguage)`: an explicit account preference
/// wins, otherwise a Polish browser (`/(?:^|[,\s])pl\b/i`) gets Polish.
pub fn pick_mail_locale(preferred: Option<&str>, accept_language: Option<&str>) -> MailLocale {
    match preferred {
        Some("pl") => return MailLocale::Pl,
        Some("en") => return MailLocale::En,
        _ => {}
    }
    let header = accept_language.unwrap_or("").to_ascii_lowercase();
    let bytes = header.as_bytes();
    for (i, _) in header.match_indices("pl") {
        let before_ok = i == 0 || matches!(bytes[i - 1], b',' | b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c);
        let after_ok = bytes.get(i + 2).is_none_or(|c| !(c.is_ascii_alphanumeric() || *c == b'_'));
        if before_ok && after_ok {
            return MailLocale::Pl;
        }
    }
    MailLocale::En
}

/// The `mail` dictionaries plus the app name: everything a template needs.
#[derive(Debug, Clone)]
pub struct MailTexts {
    pub en: Value,
    pub pl: Value,
    pub app_name: String,
}

impl MailTexts {
    pub fn from_messages(messages: &Messages, app_name: &str) -> MailTexts {
        MailTexts {
            en: messages.namespace(crate::i18n::Locale::En, "mail").cloned().unwrap_or(Value::Null),
            pl: messages.namespace(crate::i18n::Locale::Pl, "mail").cloned().unwrap_or(Value::Null),
            app_name: app_name.to_string(),
        }
    }

    fn dict(&self, locale: MailLocale) -> &Value {
        match locale {
            MailLocale::En => &self.en,
            MailLocale::Pl => &self.pl,
        }
    }

    /// `t(locale, key, vars)`: look up a dotted key and substitute
    /// `{placeholders}` (`/\{(\w+)\}/g`; unknown names stay as written).
    fn t(&self, locale: MailLocale, key: &str, vars: &[(&str, String)]) -> String {
        let mut node = self.dict(locale);
        for part in key.split('.') {
            node = node.get(part).unwrap_or(&Value::Null);
        }
        let Some(raw) = node.as_str() else {
            panic!("mail: missing message {}.{key}", locale.as_str());
        };
        substitute(raw, vars)
    }
}

fn substitute(raw: &str, vars: &[(&str, String)]) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let name_len = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(after.len());
        if name_len > 0 && after[name_len..].starts_with('}') {
            let name = &after[..name_len];
            match vars.iter().find(|(k, _)| *k == name) {
                Some((_, v)) => out.push_str(v),
                None => {
                    out.push('{');
                    out.push_str(name);
                    out.push('}');
                }
            }
            rest = &after[name_len + 1..];
        } else {
            out.push('{');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// HTML-escape. Every interpolated value goes through this, without exception.
fn esc(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

// ── Brand mark ──────────────────────────────────────────────────────────────

/// Content-ID of the inline logo. Referenced as `cid:` in the markup.
pub const LOGO_CID: &str = "modlight-logo";

/// `logoPath()`: the committed PNG under `public/brand/`.
pub fn logo_path(public_dir: &Path) -> PathBuf {
    public_dir.join("brand").join("email-logo.png")
}

/// `logoAttachments()`: the logo, or nothing at all when the file is missing
/// (a missing PNG must not take out every transactional mail). Cached: the
/// file is baked into the image.
pub fn logo_attachment(public_dir: &Path) -> Option<(String, Vec<u8>)> {
    static LOGO: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    let path = logo_path(public_dir);
    LOGO.get_or_init(|| match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(_) => {
            tracing::warn!("[mail] brand logo missing at {} — sending without it", path.display());
            None
        }
    })
    .clone()
    .map(|bytes| ("logo.png".to_string(), bytes))
}

// ── Layout ──────────────────────────────────────────────────────────────────

struct LayoutParts<'a> {
    locale: MailLocale,
    preheader: String,
    title: String,
    /// Already-escaped HTML for the body column.
    body_html: &'a str,
}

fn layout(texts: &MailTexts, parts: LayoutParts<'_>) -> String {
    let app_name = esc(&texts.app_name);
    let app = [("appName", texts.app_name.clone())];
    let logo_alt = esc(&texts.t(parts.locale, "common.logoAlt", &app));
    let footer = esc(&texts.t(parts.locale, "common.footer", &app));
    let automated = esc(&texts.t(parts.locale, "common.automated", &[]));
    let locale = parts.locale.as_str();
    let title = esc(&parts.title);
    let preheader = esc(&parts.preheader);
    let body = parts.body_html;
    format!(
        r#"<!doctype html>
<html lang="{locale}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<!-- Declared dark-only, and honestly so: every colour below is a hardcoded dark
     hex. Claiming to support both schemes invites a dark-mode client to apply
     its own inversion on top of ours, which is how a message ends up with dark
     text on a dark card. -->
<meta name="color-scheme" content="dark">
<meta name="supported-color-schemes" content="dark">
<title>{title}</title>
</head>
<body style="margin:0;padding:0;background:{BG};font-family:{FONT}">
<!-- Preheader: the line a mailbox shows next to the subject. Without one, the
     preview is the first sentence of the body, which is usually the disclaimer. -->
<div style="display:none;max-height:0;overflow:hidden;opacity:0;color:transparent">{preheader}</div>
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="background:{BG};padding:32px 12px">
  <tr>
    <td align="center">
      <table role="presentation" width="600" cellpadding="0" cellspacing="0" border="0" style="width:100%;max-width:600px;background:{CARD};border:1px solid {BORDER};border-radius:16px">
        <tr>
          <td style="padding:28px 32px 4px">
            <table role="presentation" cellpadding="0" cellspacing="0" border="0">
              <tr>
                <td style="padding-right:10px" valign="middle">
                  <img src="cid:{LOGO_CID}" width="40" height="40" alt="{logo_alt}" style="display:block;border:0;border-radius:9px">
                </td>
                <td valign="middle" style="font-family:{FONT};font-size:17px;font-weight:700;color:{TEXT}">{app_name}</td>
              </tr>
            </table>
          </td>
        </tr>
        <tr>
          <td style="padding:20px 32px 0;font-family:{FONT};font-size:21px;line-height:1.3;font-weight:600;color:{TEXT}">{title}</td>
        </tr>
        <tr>
          <td style="padding:12px 32px 32px;font-family:{FONT};font-size:14px;line-height:1.65;color:{MUTED}">{body}</td>
        </tr>
      </table>
      <table role="presentation" width="600" cellpadding="0" cellspacing="0" border="0" style="width:100%;max-width:600px">
        <tr>
          <td style="padding:16px 32px;font-family:{FONT};font-size:12px;line-height:1.6;color:{FAINT}">
            {footer}<br>
            {automated}
          </td>
        </tr>
      </table>
    </td>
  </tr>
</table>
</body>
</html>"#
    )
}

fn paragraph(text: &str, color: &str) -> String {
    format!(r#"<p style="margin:0 0 14px;color:{color};font-size:14px;line-height:1.65">{}</p>"#, esc(text))
}

/// A call to action, followed by the same URL in plain text.
fn action(url: &str, label: &str, fallback_label: &str) -> String {
    let safe = esc(url);
    let label = esc(label);
    let fallback = esc(fallback_label);
    format!(
        r#"<table role="presentation" cellpadding="0" cellspacing="0" border="0" style="margin:18px 0 14px">
  <tr><td align="center" bgcolor="{ACCENT}" style="border-radius:10px">
    <a href="{safe}" style="display:inline-block;padding:12px 22px;font-family:{FONT};font-size:14px;font-weight:600;color:#ffffff;text-decoration:none">{label}</a>
  </td></tr>
</table>
<p style="margin:0 0 6px;color:{FAINT};font-size:12px">{fallback}</p>
<p style="margin:0 0 14px;font-size:12px;word-break:break-all"><a href="{safe}" style="color:{ACCENT};text-decoration:underline">{safe}</a></p>"#
    )
}

/// `RenderedMail`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedMail {
    pub subject: String,
    pub html: String,
    pub text: String,
}

fn build(
    texts: &MailTexts,
    locale: MailLocale,
    ns: &str,
    vars: &[(&str, String)],
    html: &str,
    text: String,
) -> RenderedMail {
    // `{ appName, ...vars }`
    let mut with_app: Vec<(&str, String)> = vec![("appName", texts.app_name.clone())];
    for (k, v) in vars {
        if let Some(slot) = with_app.iter_mut().find(|(name, _)| name == k) {
            slot.1 = v.clone();
        } else {
            with_app.push((k, v.clone()));
        }
    }
    RenderedMail {
        subject: texts.t(locale, &format!("{ns}.subject"), &with_app),
        html: layout(
            texts,
            LayoutParts {
                locale,
                preheader: texts.t(locale, &format!("{ns}.preheader"), &with_app),
                title: texts.t(locale, &format!("{ns}.title"), &with_app),
                body_html: html,
            },
        ),
        text,
    }
}

fn link_template(texts: &MailTexts, ns: &str, locale: MailLocale, email: &str, url: &str) -> RenderedMail {
    let vars = [("appName", texts.app_name.clone()), ("email", email.to_string())];
    let intro = texts.t(locale, &format!("{ns}.intro"), &vars);
    let expiry = texts.t(locale, &format!("{ns}.expiry"), &vars);
    let ignore = texts.t(locale, &format!("{ns}.ignore"), &vars);
    let label = texts.t(locale, &format!("{ns}.button"), &vars);
    let fallback = texts.t(locale, "common.linkFallback", &[]);
    let html = paragraph(&intro, MUTED)
        + &action(url, &label, &fallback)
        + &paragraph(&expiry, FAINT)
        + &paragraph(&ignore, FAINT);
    let text = [intro.as_str(), "", &format!("{label}: {url}"), "", expiry.as_str(), ignore.as_str()].join("\n");
    build(texts, locale, ns, &vars, &html, text)
}

pub fn render_verify_email(texts: &MailTexts, locale: MailLocale, email: &str, url: &str) -> RenderedMail {
    link_template(texts, "verifyEmail", locale, email, url)
}

pub fn render_reset_password(texts: &MailTexts, locale: MailLocale, email: &str, url: &str) -> RenderedMail {
    link_template(texts, "resetPassword", locale, email, url)
}

pub fn render_account_invited(texts: &MailTexts, locale: MailLocale, email: &str, url: &str) -> RenderedMail {
    link_template(texts, "accountInvited", locale, email, url)
}

pub fn render_smtp_test(texts: &MailTexts, locale: MailLocale, host: &str, port: i64, from: &str) -> RenderedMail {
    let vars = [
        ("appName", texts.app_name.clone()),
        ("host", host.to_string()),
        ("port", port.to_string()),
        ("from", from.to_string()),
    ];
    let intro = texts.t(locale, "smtpTest.intro", &vars);
    let detail = texts.t(locale, "smtpTest.detail", &vars);
    let closing = texts.t(locale, "smtpTest.closing", &vars);
    let html = paragraph(&intro, MUTED) + &paragraph(&detail, TEXT) + &paragraph(&closing, FAINT);
    let text = [intro.as_str(), "", detail.as_str(), "", closing.as_str()].join("\n");
    build(texts, locale, "smtpTest", &vars, &html, text)
}

pub fn render_password_changed(texts: &MailTexts, locale: MailLocale, email: &str) -> RenderedMail {
    let vars = [("appName", texts.app_name.clone()), ("email", email.to_string())];
    let intro = texts.t(locale, "passwordChanged.intro", &vars);
    let closing = texts.t(locale, "passwordChanged.closing", &vars);
    let warning = texts.t(locale, "passwordChanged.warning", &vars);
    let html = paragraph(&intro, MUTED) + &paragraph(&closing, MUTED) + &paragraph(&warning, WARN);
    let text = [intro.as_str(), "", closing.as_str(), warning.as_str()].join("\n");
    build(texts, locale, "passwordChanged", &vars, &html, text)
}

pub fn render_email_changed(texts: &MailTexts, locale: MailLocale, old_email: &str, new_email: &str) -> RenderedMail {
    let vars =
        [("appName", texts.app_name.clone()), ("oldEmail", old_email.to_string()), ("newEmail", new_email.to_string())];
    let intro = texts.t(locale, "emailChanged.intro", &vars);
    let closing = texts.t(locale, "emailChanged.closing", &vars);
    let warning = texts.t(locale, "emailChanged.warning", &vars);
    let html = paragraph(&intro, MUTED) + &paragraph(&closing, MUTED) + &paragraph(&warning, WARN);
    let text = [intro.as_str(), "", closing.as_str(), warning.as_str()].join("\n");
    build(texts, locale, "emailChanged", &vars, &html, text)
}

/// Is `locale` the UI locale `MailLocale` maps to? (Used by i18n parity checks.)
pub fn ui_locale(locale: MailLocale) -> crate::i18n::Locale {
    locale.ui()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Paths;

    const URL: &str = "https://files.example.invalid/verify/abc123";

    fn texts() -> MailTexts {
        let paths = Paths::crate_dir();
        let messages = Messages::load(&paths.messages).unwrap();
        MailTexts::from_messages(&messages, "Modlight")
    }

    fn every(t: &MailTexts, locale: MailLocale) -> Vec<(&'static str, RenderedMail)> {
        vec![
            ("verifyEmail", render_verify_email(t, locale, "ada@example.invalid", URL)),
            ("resetPassword", render_reset_password(t, locale, "ada@example.invalid", URL)),
            ("accountInvited", render_account_invited(t, locale, "ada@example.invalid", URL)),
            ("smtpTest", render_smtp_test(t, locale, "smtp.example.invalid", 587, "no-reply@example.invalid")),
            ("passwordChanged", render_password_changed(t, locale, "ada@example.invalid")),
            ("emailChanged", render_email_changed(t, locale, "old@example.invalid", "new@example.invalid")),
        ]
    }

    fn has_placeholder(s: &str) -> bool {
        let re = regex::Regex::new(r"\{\w+\}").unwrap();
        re.is_match(s)
    }

    // tests/unit/mail-templates.test.ts: "transactional emails"
    #[test]
    fn every_template_in_every_locale_is_well_formed() {
        let t = texts();
        for locale in [MailLocale::En, MailLocale::Pl] {
            for (name, mail) in every(&t, locale) {
                assert!(!mail.subject.trim().is_empty(), "{name}");
                assert!(mail.html.contains("<html"), "{name}");
                assert!(!mail.text.trim().is_empty(), "{name}");
                assert!(!has_placeholder(&mail.subject), "{name}");
                assert!(!has_placeholder(&mail.text), "{name}");
                let seen = format!("{}{}", mail.subject, mail.text);
                assert!(!seen.contains('—') && !seen.contains('–'), "{name}");
                assert!(mail.html.contains("role=\"presentation\""), "{name}");
                assert!(!mail.html.contains("display:flex") && !mail.html.contains("display:grid"), "{name}");
                assert!(mail.html.contains("cid:modlight-logo"), "{name}");
                assert!(!mail.html.contains("src=\"data:"), "{name}");
            }
        }
    }

    #[test]
    fn shows_the_full_link_as_text_not_only_as_a_button() {
        let t = texts();
        for locale in [MailLocale::En, MailLocale::Pl] {
            for (name, mail) in every(&t, locale) {
                if ["verifyEmail", "resetPassword", "accountInvited"].contains(&name) {
                    assert!(mail.html.contains(URL), "{name}");
                    assert!(mail.text.contains(URL), "{name}");
                }
            }
        }
    }

    #[test]
    fn escapes_interpolated_values_instead_of_pasting_them_into_markup() {
        let t = texts();
        let mail =
            render_verify_email(&t, MailLocale::En, "\"><script>alert(1)</script>", "https://example.invalid/?a=1&b=2");
        assert!(!mail.html.contains("<script>"));
        assert!(mail.html.contains("&amp;b=2"));
    }

    #[test]
    fn says_the_same_things_in_both_languages() {
        let t = texts();
        let en = every(&t, MailLocale::En);
        let pl = every(&t, MailLocale::Pl);
        for ((a_name, a), (b_name, b)) in en.iter().zip(pl.iter()) {
            assert_eq!(a_name, b_name);
            assert_ne!(a.subject, b.subject, "{a_name}");
        }
    }

    #[test]
    fn ships_the_raster_logo_the_messages_reference() {
        assert!(logo_path(&Paths::crate_dir().public).is_file());
    }

    // "language choice"
    #[test]
    fn honours_an_explicit_account_preference() {
        assert_eq!(pick_mail_locale(Some("pl"), Some("en-GB,en;q=0.9")), MailLocale::Pl);
        assert_eq!(pick_mail_locale(Some("en"), Some("pl-PL")), MailLocale::En);
    }

    #[test]
    fn falls_back_to_polish_only_for_a_polish_browser() {
        assert_eq!(pick_mail_locale(None, Some("pl-PL,pl;q=0.9")), MailLocale::Pl);
        assert_eq!(pick_mail_locale(None, Some("de-DE,de;q=0.9")), MailLocale::En);
        assert_eq!(pick_mail_locale(None, None), MailLocale::En);
        assert_eq!(pick_mail_locale(Some("de"), Some("en, PL")), MailLocale::Pl);
        assert_eq!(pick_mail_locale(None, Some("plx")), MailLocale::En);
    }

    #[test]
    fn placeholders_substitute_like_the_upstream_regex() {
        assert_eq!(substitute("Hi {name}, {missing} {", &[("name", "Ada".into())]), "Hi Ada, {missing} {");
        assert_eq!(substitute("{a}{b}", &[("a", "1".into()), ("b", "2".into())]), "12");
    }
}
