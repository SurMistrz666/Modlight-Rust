//! The sentences the server's answers and refusals are put into, in the reader's
//! language (`ModlightSignIn` results, the shortener's and the uploader's refusals,
//! `MainWindow.ReportFailure`).

use modlight_api::{ApiError, SignInFailure};

use crate::l10n::{Arg, format, port, s, t, uploaders};

/// What a failed sign-in tells the person.
pub fn sign_in_failure(failure: &SignInFailure) -> String {
    match failure {
        SignInFailure::InvalidHost => t(s::ModlightSignIn_InvalidHost).to_string(),
        SignInFailure::ListenerFailed(message) => format(
            t(s::ModlightSignIn_ListenerFailed),
            &[Arg::from(message.as_str())],
        ),
        SignInFailure::TimedOut => t(s::ModlightSignIn_TimedOut).to_string(),
        SignInFailure::Canceled => t(s::ModlightSignIn_Canceled).to_string(),
        SignInFailure::Denied(None) => t(s::ModlightSignIn_Denied).to_string(),
        SignInFailure::Denied(Some(reason)) => format(
            t(s::ModlightSignIn_DeniedWithReason),
            &[Arg::from(reason.as_str())],
        ),
        SignInFailure::NoToken => t(s::ModlightSignIn_NoToken).to_string(),
        SignInFailure::Unreachable { origin, message } => unreachable(origin, message),
        SignInFailure::TokenRejected => t(s::ModlightSignIn_TokenRejected).to_string(),
    }
}

/// "Signed in as Alex Rivera."
pub fn signed_in_as(name: &str) -> String {
    format(t(s::ModlightSignIn_SignedInAs), &[Arg::from(name)])
}

/// "Could not reach http://...: `<reason>`".
pub fn unreachable(host: &str, message: &str) -> String {
    format(
        t(s::ModlightSignIn_Unreachable),
        &[Arg::from(host), Arg::from(message)],
    )
}

/// A short-link refusal: the server's `reason` token first, the status as a fallback.
pub fn short_link_refusal(error: &ApiError) -> String {
    let (status, detail, reason) = match error {
        ApiError::Refused {
            status,
            detail,
            reason,
        } => (*status, detail.as_str(), reason.as_deref()),
        ApiError::Unauthorized => (401, "", None),
        other => return other.to_string(),
    };
    let by_reason = match reason {
        Some("invalid-url") => Some(uploaders::Modlight_Link_invalid_url),
        Some("invalid-code") => Some(uploaders::Modlight_Link_invalid_code),
        Some("code-taken") => Some(uploaders::Modlight_Address_taken),
        Some("code-too-long") => Some(uploaders::Modlight_Link_code_too_long),
        Some("root-not-allowed") => Some(uploaders::Modlight_Link_root_not_allowed),
        Some("no-tag") => Some(uploaders::Modlight_Link_no_tag),
        Some("rate-limited") => Some(uploaders::Modlight_Too_many_requests),
        _ => None,
    };
    let message = match by_reason {
        Some(key) => t(key).to_string(),
        None => match status {
            401 => t(uploaders::Modlight_Token_not_recognized).to_string(),
            403 => t(uploaders::Modlight_Token_not_allowed).to_string(),
            409 => t(uploaders::Modlight_Address_taken).to_string(),
            429 => t(uploaders::Modlight_Too_many_requests).to_string(),
            other => format(
                t(uploaders::Modlight_Server_refused),
                &[Arg::Int(other as i64)],
            ),
        },
    };
    with_detail(message, detail)
}

/// An upload refusal by status (`Modlight.ReportRefusal`).
pub fn upload_refusal(status: u16, server_text: Option<&str>) -> String {
    let message = match status {
        400 => t(uploaders::Modlight_Request_rejected).to_string(),
        401 => t(uploaders::Modlight_Account_disabled).to_string(),
        404 => t(uploaders::Modlight_Token_not_recognized).to_string(),
        409 => t(uploaders::Modlight_Address_taken).to_string(),
        413 => t(uploaders::Modlight_File_too_large).to_string(),
        507 => t(uploaders::Modlight_Out_of_space).to_string(),
        other => format(
            t(uploaders::Modlight_Server_refused),
            &[Arg::Int(other as i64)],
        ),
    };
    with_detail(message, server_text.unwrap_or(""))
}

fn with_detail(message: String, detail: &str) -> String {
    if detail.is_empty() {
        message
    } else {
        format!("{message}\n{detail}")
    }
}

/// How a failed library action is reported in a notice (`MainWindow.ReportFailure`):
/// (title, detail, is the remedy signing in again).
pub fn library_failure(error: &ApiError, host: &str) -> (String, String, bool) {
    match error {
        ApiError::Unauthorized => (t(s::MySpace_TokenRejected).to_string(), String::new(), true),
        // Not upstream: the account's files are encrypted and locked. The token is fine:
        // the remedy is the file password above the files, never signing in again.
        ApiError::Locked(_) => (
            t(port::VaultFilesLocked).to_string(),
            t(port::VaultFilesLockedDetail).to_string(),
            false,
        ),
        ApiError::Refused { detail, reason, .. } => (
            t(s::MySpace_ServerRefused).to_string(),
            if detail.is_empty() {
                crate::library::convert::refusal_text(reason.as_deref().unwrap_or("")).to_string()
            } else {
                detail.clone()
            },
            false,
        ),
        other => (unreachable(host, &other.to_string()), String::new(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::l10n::{Language, set_thread_language};

    #[test]
    fn refusals_name_the_fix() {
        set_thread_language(Some(Language::English));
        let taken = ApiError::Refused {
            status: 409,
            detail: "That code is taken.".into(),
            reason: Some("code-taken".into()),
        };
        assert_eq!(
            short_link_refusal(&taken),
            format!(
                "{}\nThat code is taken.",
                t(uploaders::Modlight_Address_taken)
            )
        );
        let odd = ApiError::Refused {
            status: 418,
            detail: String::new(),
            reason: Some("teapot".into()),
        };
        assert!(short_link_refusal(&odd).contains("418"));
        assert_eq!(
            upload_refusal(507, None),
            t(uploaders::Modlight_Out_of_space)
        );
        assert_eq!(
            sign_in_failure(&SignInFailure::Denied(Some("x".into()))),
            format(t(s::ModlightSignIn_DeniedWithReason), &[Arg::from("x")])
        );
        let (title, _, sign_in) = library_failure(&ApiError::Unauthorized, "h");
        assert_eq!(title, t(s::MySpace_TokenRejected));
        assert!(sign_in);
        // Locked files are not a refused token: no sign-in is offered.
        let (title, detail, sign_in) =
            library_failure(&ApiError::Locked(modlight_api::FilesLocked::default()), "h");
        assert_eq!(title, t(port::VaultFilesLocked));
        assert_eq!(detail, t(port::VaultFilesLockedDetail));
        assert!(!sign_in);
        set_thread_language(None);
    }
}
