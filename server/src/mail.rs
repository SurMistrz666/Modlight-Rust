//! Port of `lib/mail.ts`: outgoing mail, the transport and nothing else. What
//! the messages SAY lives in [`crate::mail_templates`]; where the credentials
//! come from lives in [`crate::mail_settings`].
//!
//! nodemailer is reproduced rather than replaced ([`smtp`], [`address`]),
//! because the SMTP test button shows the operator nodemailer's own verdicts:
//!
//! | `security` | nodemailer option | meaning |
//! |---|---|---|
//! | `tls` | `secure: true` | implicit TLS (the historical port 465) |
//! | `starttls` | `requireTLS: true` | STARTTLS, and REFUSE to continue without it |
//! | `none` | neither | plain connect; STARTTLS when the server offers it |
//!
//! Timeouts: connection 10 s, greeting 10 s, socket 20 s. nodemailer sets none
//! of these by default, which is fine for a background job and wrong for the
//! "send a test message" button: a wrong host or a firewalled port would
//! leave the operator staring at a spinner until the kernel gave up on the
//! TCP handshake, which can take minutes. Its debug output (which prints the
//! AUTH exchange) is never on; nothing here logs the dialogue.
//!
//! The message is `multipart/alternative` (text, then `multipart/related`
//! html + the inline logo by Content-ID), the structure nodemailer builds for
//! html + text + a cid attachment; lettre formats the MIME parts.
//!
//! The upstream transporter is cached against a fingerprint of the settings
//! (`mailFingerprint`) so a changed configuration rebuilds it in every
//! process. Without connection pooling (lib/mail.ts sets none) a transporter
//! holds no state between messages, so here the options are simply rebuilt
//! from the settings snapshot on every send, which has the same effect.

pub mod address;
pub mod smtp;

use lettre::message::header::{ContentType, HeaderName, HeaderValue, Headers};
use lettre::message::{Attachment, MultiPart, SinglePart};

use crate::mail_settings::{ResolvedMail, SmtpSecurity, load_mail_settings};
use crate::mail_templates::{LOGO_CID, RenderedMail, logo_attachment};
use crate::secret_box::{redact_secrets, smtp_secret_forms};
use crate::state::AppState;
pub use smtp::{SendInfo, SmtpError as SmtpFailure};

/// `MailNotConfiguredError`.
pub const MAIL_NOT_CONFIGURED: &str = "Outgoing mail is not configured on this installation.";

/// A transporter for one configuration (`nodemailer.createTransport(
/// transportOptions(mail))`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transport {
    pub options: smtp::SmtpOptions,
}

/// `transportOptions(mail)`: "tls" is implicit TLS, "starttls" connects in
/// clear and REFUSES to continue if the server will not upgrade, "none"
/// leaves nodemailer's own upgrade-when-offered in charge, which is what an
/// env-configured install has always done. Authentication only with a user.
pub fn transport_options(mail: &ResolvedMail) -> Transport {
    Transport {
        options: smtp::SmtpOptions {
            host: mail.host.clone(),
            port: mail.port,
            secure: mail.security == SmtpSecurity::Tls,
            require_tls: mail.security == SmtpSecurity::Starttls,
            auth: (!mail.user.is_empty())
                .then(|| smtp::Credentials { user: mail.user.clone(), pass: mail.password.clone() }),
            connection_timeout: smtp::CONNECTION_TIMEOUT,
            greeting_timeout: smtp::GREETING_TIMEOUT,
            socket_timeout: smtp::SOCKET_TIMEOUT,
        },
    }
}

/// `getTransport()`: the transporter for the CURRENT configuration, or `None`
/// when mail is not ready.
pub async fn get_transport(state: &AppState) -> anyhow::Result<Option<(Transport, ResolvedMail)>> {
    let mail = load_mail_settings(state, false).await?;
    if !mail.ready {
        return Ok(None);
    }
    Ok(Some((transport_options(&mail), mail)))
}

/// `envelopeFrom(mail)`: the display name is stripped of quotes and line
/// breaks rather than escaped: a name is not worth a header-injection
/// surface, and nobody's brand needs one.
pub fn envelope_from(mail: &ResolvedMail) -> String {
    let name: String = mail.from_name.chars().filter(|c| !matches!(c, '"' | '\r' | '\n')).collect();
    let name = crate::config::js_trim(&name);
    if name.is_empty() { mail.from.clone() } else { format!("\"{name}\" <{}>", mail.from) }
}

/// One outgoing message (`MailMessage`).
#[derive(Debug, Clone)]
pub struct MailMessage {
    pub to: String,
    pub subject: String,
    pub html: String,
    pub text: String,
}

impl MailMessage {
    pub fn from_rendered(to: &str, rendered: &RenderedMail) -> MailMessage {
        MailMessage {
            to: to.to_string(),
            subject: rendered.subject.clone(),
            html: rendered.html.clone(),
            text: rendered.text.clone(),
        }
    }
}

fn pre_encoded(name: &'static str, value: String) -> HeaderValue {
    HeaderValue::dangerous_new_pre_encoded(HeaderName::new_from_ascii_str(name), value.clone(), value)
}

/// What nodemailer puts on the wire for `sendMail({ from, to, subject, html,
/// text, attachments: logoAttachments() })`: the envelope its MIME tree
/// derives from the From and To values, and the message.
pub fn compose(state: &AppState, mail: &ResolvedMail, msg: &MailMessage) -> (smtp::Envelope, Vec<u8>) {
    let mut senders = Vec::new();
    let from_header = address::convert_addresses(&address::addressparser(&envelope_from(mail)), &mut senders);
    let mut recipients = Vec::new();
    let to_header = address::convert_addresses(&address::addressparser(&msg.to), &mut recipients);
    let envelope = smtp::Envelope { from: senders.first().cloned().unwrap_or_default(), to: recipients };

    let html = SinglePart::builder().header(ContentType::TEXT_HTML).body(msg.html.clone());
    let text = SinglePart::builder().header(ContentType::TEXT_PLAIN).body(msg.text.clone());
    let body = match logo_attachment(&state.paths.public) {
        Some((filename, bytes)) => {
            let logo = Attachment::new_inline_with_name(LOGO_CID.to_string(), filename)
                .body(bytes, ContentType::parse("image/png").expect("static content type"));
            MultiPart::alternative().singlepart(text).multipart(MultiPart::related().singlepart(html).singlepart(logo))
        }
        None => MultiPart::alternative().singlepart(text).singlepart(html),
    };

    // `_generateMessageId`: random hex groups at the sender's domain.
    let id = hex::encode(rand::random::<[u8; 16]>());
    let domain = envelope.from.rsplit('@').next().filter(|d| !d.is_empty()).unwrap_or("localhost");
    let message_id = format!("<{}-{}-{}-{}-{}@{domain}>", &id[..8], &id[8..12], &id[12..16], &id[16..20], &id[20..]);

    let mut headers = Headers::new();
    if !from_header.is_empty() {
        headers.insert_raw(pre_encoded("From", from_header));
    }
    if !to_header.is_empty() {
        headers.insert_raw(pre_encoded("To", to_header));
    }
    headers.insert_raw(HeaderValue::new(HeaderName::new_from_ascii_str("Subject"), msg.subject.clone()));
    headers.insert_raw(pre_encoded("Message-ID", message_id));
    headers.insert_raw(pre_encoded("Date", chrono::Utc::now().format("%a, %d %b %Y %H:%M:%S +0000").to_string()));
    headers.insert_raw(pre_encoded("MIME-Version", "1.0".to_string()));

    let mut bytes = headers.to_string().into_bytes();
    bytes.extend_from_slice(&body.formatted());
    (envelope, bytes)
}

/// `transport.sendMail(...)` on an already-built transporter, with the
/// transporter's own error (the SMTP test needs it to classify the failure).
pub async fn send_with_transport(
    state: &AppState,
    transport: &Transport,
    mail: &ResolvedMail,
    msg: &MailMessage,
) -> Result<SendInfo, SmtpFailure> {
    let (envelope, bytes) = compose(state, mail, msg);
    smtp::send(&transport.options, &envelope, &bytes).await
}

/// `sendMail(msg)`: refuses loudly when mail is not configured, and that is a
/// deliberate change: the previous behaviour logged the message and reported
/// success, so "reset my password" told the user to check an inbox that would
/// never receive anything. A caller that must not fail (verification on
/// sign-up) checks `mailReady()` first.
///
/// Development keeps the console fallback, because a developer with no SMTP
/// server still has to be able to click the link. Errors are redacted before
/// anyone reads them: the server's own error text routinely quotes part of
/// the AUTH line back at us.
pub async fn send_mail(state: &AppState, msg: &MailMessage) -> anyhow::Result<()> {
    let Some((transport, mail)) = get_transport(state).await? else {
        if !state.config.is_prod {
            tracing::info!("[mail:dev] To: {} | {}\n{}", msg.to, msg.subject, msg.text);
            return Ok(());
        }
        anyhow::bail!(MAIL_NOT_CONFIGURED);
    };
    match send_with_transport(state, &transport, &mail, msg).await {
        Ok(_) => Ok(()),
        Err(failure) => {
            anyhow::bail!(redact_secrets(&failure.message, &smtp_secret_forms(&mail.user, &mail.password)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail_settings::{MailPasswordState, MailSource};

    fn resolved(name: &str) -> ResolvedMail {
        ResolvedMail {
            source: MailSource::Panel,
            host: "smtp.example.invalid".into(),
            port: 587,
            security: SmtpSecurity::Starttls,
            user: String::new(),
            password: String::new(),
            from: "no-reply@example.invalid".into(),
            from_name: name.into(),
            ready: true,
            password_state: MailPasswordState::Unset,
            verification_required: false,
        }
    }

    #[test]
    fn the_from_header_strips_quotes_and_line_breaks() {
        assert_eq!(envelope_from(&resolved("Modlight")), "\"Modlight\" <no-reply@example.invalid>");
        assert_eq!(envelope_from(&resolved("  Evil\"\r\nBcc: x ")), "\"EvilBcc: x\" <no-reply@example.invalid>");
        assert_eq!(envelope_from(&resolved("\"\"")), "no-reply@example.invalid");
    }

    #[test]
    fn transport_options_follow_the_security_mode() {
        let mut mail = resolved("Modlight");
        let t = transport_options(&mail).options;
        assert!(!t.secure && t.require_tls && t.auth.is_none());
        mail.security = SmtpSecurity::Tls;
        mail.user = "u".into();
        mail.password = "p".into();
        let t = transport_options(&mail).options;
        assert!(t.secure && !t.require_tls);
        assert_eq!(t.auth, Some(smtp::Credentials { user: "u".into(), pass: "p".into() }));
        mail.security = SmtpSecurity::None;
        let t = transport_options(&mail).options;
        assert!(!t.secure && !t.require_tls);
    }
}
