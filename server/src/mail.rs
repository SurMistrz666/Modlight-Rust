//! Port of `lib/mail.ts`: outgoing mail, the transport and nothing else. What
//! the messages SAY lives in [`crate::mail_templates`]; where the credentials
//! come from lives in [`crate::mail_settings`].
//!
//! nodemailer is replaced by lettre with the same connection semantics:
//!
//! | `security` | nodemailer | lettre |
//! |---|---|---|
//! | `tls` | `secure: true` (implicit TLS, the historical port 465) | `Tls::Wrapper` |
//! | `starttls` | `requireTLS: true` (refuses to continue without the upgrade) | `Tls::Required` |
//! | `none` | neither: opportunistic STARTTLS when the server offers it | `Tls::Opportunistic` |
//!
//! Certificates are verified against the Mozilla root set (nodemailer used
//! Node's bundled Mozilla roots). Authentication is attempted only when a user
//! is configured. Timeouts: nodemailer had separate connect (10 s), greeting
//! (10 s) and socket (20 s) limits; lettre has one per-operation timeout,
//! set to 20 s, and the connection is bounded by 10 s on top.
//!
//! The message is `multipart/alternative` (text, then `multipart/related`
//! html + the inline logo by Content-ID), the structure nodemailer builds for
//! html + text + a cid attachment.

use std::sync::Mutex;
use std::time::Duration;

use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::mail_settings::{ResolvedMail, SmtpSecurity, load_mail_settings, mail_fingerprint};
use crate::mail_templates::{LOGO_CID, RenderedMail, logo_attachment};
use crate::secret_box::{redact_secrets, smtp_secret_forms};
use crate::state::AppState;

/// `MailNotConfiguredError`.
pub const MAIL_NOT_CONFIGURED: &str = "Outgoing mail is not configured on this installation.";

const SOCKET_TIMEOUT: Duration = Duration::from_secs(20);

type Transport = AsyncSmtpTransport<Tokio1Executor>;

struct Cached {
    fingerprint: String,
    transport: Transport,
}

static TRANSPORT: Mutex<Option<Cached>> = Mutex::new(None);

fn build_transport(mail: &ResolvedMail) -> anyhow::Result<Transport> {
    let params = TlsParameters::new(mail.host.clone())?;
    let tls = match mail.security {
        SmtpSecurity::Tls => Tls::Wrapper(params),
        SmtpSecurity::Starttls => Tls::Required(params),
        SmtpSecurity::None => Tls::Opportunistic(params),
    };
    let port = u16::try_from(mail.port).unwrap_or(587);
    let mut builder = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(mail.host.clone())
        .port(port)
        .tls(tls)
        .timeout(Some(SOCKET_TIMEOUT));
    if !mail.user.is_empty() {
        builder = builder.credentials(Credentials::new(mail.user.clone(), mail.password.clone()));
    }
    Ok(builder.build())
}

/// `getTransport()`: the transport for the CURRENT configuration, cached
/// against a fingerprint of the settings it was built from, or `None` when
/// mail is not ready.
pub async fn get_transport(state: &AppState) -> anyhow::Result<Option<(Transport, ResolvedMail)>> {
    let mail = load_mail_settings(state, false).await?;
    if !mail.ready {
        return Ok(None);
    }
    let fingerprint = mail_fingerprint(&mail);
    let mut guard = TRANSPORT.lock().unwrap_or_else(|e| e.into_inner());
    if guard.as_ref().map(|c| c.fingerprint.as_str()) != Some(fingerprint.as_str()) {
        *guard = Some(Cached { fingerprint, transport: build_transport(&mail)? });
    }
    let transport = guard.as_ref().map(|c| c.transport.clone()).expect("set above");
    Ok(Some((transport, mail)))
}

/// `envelopeFrom(mail)`: the display name is stripped of quotes and line
/// breaks rather than escaped.
pub fn envelope_from(mail: &ResolvedMail) -> String {
    let name: String = mail.from_name.chars().filter(|c| !matches!(c, '"' | '\r' | '\n')).collect();
    let name = crate::config::js_trim(&name);
    if name.is_empty() { mail.from.clone() } else { format!("\"{name}\" <{}>", mail.from) }
}

/// One outgoing message.
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

fn mailbox(value: &str) -> anyhow::Result<Mailbox> {
    value.parse::<Mailbox>().map_err(|e| anyhow::anyhow!("Invalid address {value:?}: {e}"))
}

fn build_message(state: &AppState, mail: &ResolvedMail, msg: &MailMessage) -> anyhow::Result<Message> {
    let html_part = SinglePart::builder().header(ContentType::TEXT_HTML).body(msg.html.clone());
    let body = match logo_attachment(&state.paths.public) {
        Some((filename, bytes)) => {
            let logo = Attachment::new_inline_with_name(LOGO_CID.to_string(), filename)
                .body(bytes, ContentType::parse("image/png").expect("static content type"));
            MultiPart::alternative()
                .singlepart(SinglePart::plain(msg.text.clone()))
                .multipart(MultiPart::related().singlepart(html_part).singlepart(logo))
        }
        None => MultiPart::alternative().singlepart(SinglePart::plain(msg.text.clone())).singlepart(html_part),
    };
    Ok(Message::builder()
        .from(mailbox(&envelope_from(mail))?)
        .to(mailbox(&msg.to)?)
        .subject(msg.subject.clone())
        .multipart(body)?)
}

/// What an SMTP attempt failed with, in nodemailer's vocabulary.
#[derive(Debug, Clone)]
pub struct SmtpFailure {
    /// nodemailer's `err.code`: EAUTH, EENVELOPE, ETLS, ESOCKET, ECONNECTION,
    /// ETIMEDOUT, EDNS, or "" when unknown.
    pub code: &'static str,
    /// `err.responseCode`.
    pub response_code: Option<u16>,
    /// `err.response`: the server's own words, when it said anything.
    pub response: Option<String>,
    /// `err.message`.
    pub message: String,
}

fn classify_lettre(err: &lettre::transport::smtp::Error) -> SmtpFailure {
    let message = err.to_string();
    let response_code = err.status().map(|c| {
        let text = c.to_string();
        text.parse::<u16>().unwrap_or(0)
    });
    let response = response_code.map(|code| {
        let detail = std::error::Error::source(err).map(|s| s.to_string()).unwrap_or_default();
        if detail.is_empty() { code.to_string() } else { format!("{code} {detail}") }
    });
    let lower = message.to_ascii_lowercase();
    let code = if let Some(c) = response_code {
        match c {
            535 | 534 | 530 => "EAUTH",
            550 | 553 | 554 => "EENVELOPE",
            _ => "",
        }
    } else if err.is_timeout() {
        "ETIMEDOUT"
    } else if err.is_tls() {
        "ETLS"
    } else if lower.contains("failed to lookup") || lower.contains("dns") || lower.contains("nodename nor servname") {
        "EDNS"
    } else if lower.contains("connection") || lower.contains("network") || lower.contains("refused") {
        "ECONNECTION"
    } else {
        ""
    };
    SmtpFailure { code, response_code, response, message }
}

/// Send through an already-built transport and return the accepted
/// recipients (the SMTP test needs the raw failure to classify it).
pub async fn send_with_transport(
    state: &AppState,
    transport: &Transport,
    mail: &ResolvedMail,
    msg: &MailMessage,
) -> Result<Vec<String>, SmtpFailure> {
    let message = build_message(state, mail, msg).map_err(|e| SmtpFailure {
        code: "EENVELOPE",
        response_code: None,
        response: None,
        message: e.to_string(),
    })?;
    let recipients: Vec<String> = message.envelope().to().iter().map(|a| a.to_string()).collect();
    let attempt = tokio::time::timeout(Duration::from_secs(10) + SOCKET_TIMEOUT * 3, transport.send(message)).await;
    match attempt {
        Ok(Ok(_)) => Ok(recipients),
        Ok(Err(err)) => Err(classify_lettre(&err)),
        Err(_) => Err(SmtpFailure {
            code: "ETIMEDOUT",
            response_code: None,
            response: None,
            message: "Connection timeout".into(),
        }),
    }
}

/// `sendMail(msg)`: refuses loudly when mail is not configured (production);
/// development logs the message and returns, so a developer with no SMTP
/// server can still click the link. Errors are redacted before anyone reads
/// them: the server's own error text routinely quotes part of the AUTH line.
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
            let raw = failure.response.unwrap_or(failure.message);
            anyhow::bail!(redact_secrets(&raw, &smtp_secret_forms(&mail.user, &mail.password)))
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
}
