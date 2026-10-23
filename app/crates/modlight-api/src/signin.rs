//! Signing this installation in without ever seeing a password (RFC 8252 loopback).
//!
//! The browser authenticates and hands back an API token through a loopback POST, the
//! shape `gh auth login` uses. This module is `ModlightSignIn.cs` and
//! `ModlightLoopbackListener.cs` with the fixes the security audit asked for:
//!
//! * the listener binds `127.0.0.1` only, on a port the OS picks, and stays bound until
//!   the sign-in ends: the port is never released and re-acquired;
//! * `state` is 32 characters from a CSPRNG and is compared in constant time;
//! * a body with the WRONG state is answered and ignored, and the wait goes on. Upstream
//!   ended the whole sign-in on it, which let any local process abort a sign-in at will;
//! * a CORS preflight is answered only for the server's own origin (upstream answered `*`
//!   and `Access-Control-Allow-Private-Network: true` to anybody). The private-network
//!   permission is granted to that one origin too: when the server is public (behind
//!   Cloudflare, say) Chrome's Private Network Access sends a preflight before the page
//!   may reach 127.0.0.1, and without the grant the handoff never arrives;
//! * one shot: the first body carrying our state ends the wait and the listener closes.
//!
//! The loopback POST is made by the PAGE, and browsers may refuse to make it: Safari will
//! not send an https page's request to `http://127.0.0.1`, and Chrome's Local Network
//! Access drops it when its permission was dismissed once. So the sign-in also collects
//! the outcome from the SERVER (PKCE-style, RFC 7636 cut down): a random `verifier` stays
//! in this process, its SHA-256 `challenge` goes into the authorize link, and while the
//! listener waits the app polls `POST <host>/api/authorize/pickup` with `{state, verifier}`
//! every two seconds. Whichever path answers first wins; `/api/me` then checks the token
//! exactly as before. A server without the route just keeps answering 404 and the
//! loopback path works as it always did.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Semaphore, mpsc};
use tokio_util::sync::CancellationToken;

use crate::client::{Api, Credential};
use crate::error::ApiError;
use crate::host::{escape_data_string, is_plausible_token, try_normalize_host};
use crate::models::AccountInfo;

/// How long to hold the port open: long enough for a PERSON (find the browser, sign in,
/// pass 2FA, read the consent screen), which is what every loopback client allows.
pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// A connection that opens and then says nothing must not stall the wait.
pub const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);

/// Everything on this socket comes from a local process, so both are bounded.
pub const MAX_HEADER_BYTES: usize = 8 * 1024;
pub const MAX_BODY_BYTES: usize = 64 * 1024;

/// Connections handled at once; more are accepted only as earlier ones finish.
const MAX_CONNECTIONS: usize = 16;

/// The refusal code the authorize page posts when the person declines.
pub const DENIED_ERROR: &str = "access_denied";

/// Length of the nonce sent out with the authorize link.
pub const STATE_LENGTH: usize = 32;

/// How often the server-side pickup is asked while the person decides.
pub const PICKUP_INTERVAL: Duration = Duration::from_secs(2);

/// One pickup request may take this long before it counts as "no answer yet".
pub const PICKUP_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Where the app collects the outcome when the browser never delivered it.
pub const PICKUP_PATH: &str = "/api/authorize/pickup";

/// A fresh pickup verifier: 32 random bytes, base64url without padding (43 characters).
pub fn new_verifier() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// `base64url(SHA-256(verifier))` without padding: what the authorize link carries.
pub fn challenge_for(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// What the authorize page posted: a token or a refusal, carrying back our nonce.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Handoff {
    pub state: Option<String>,
    pub token: Option<String>,
    /// `access_denied` when the person said no in the browser.
    pub error: Option<String>,
}

/// A fresh nonce: alphanumeric, inside the `[A-Za-z0-9._~-]` set the page accepts.
pub fn new_state() -> String {
    let mut rng = rand::rng();
    (0..STATE_LENGTH)
        .map(|_| rng.sample(rand::distr::Alphanumeric) as char)
        .collect()
}

/// Compare the nonce without letting the time taken say how much of it matched.
/// The length is compared normally: it is a constant, not a secret.
pub fn nonce_equals(received: Option<&str>, expected: &str) -> bool {
    let Some(received) = received else {
        return false;
    };
    let (a, b) = (received.as_bytes(), expected.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut difference = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        difference |= x ^ y;
    }
    difference == 0
}

/// What the account's token list will call this machine: "Modlight on `<machine>`",
/// clipped to the 64 characters the server is willing to show.
pub fn client_name(app_name: &str, machine: &str) -> String {
    let name = format!("{app_name} on {machine}");
    name.chars().take(64).collect()
}

/// The page the browser is sent to. `challenge` is the pickup's (see [`challenge_for`]).
pub fn authorize_url(
    origin: &str,
    port: u16,
    client_name: &str,
    state: &str,
    challenge: &str,
) -> String {
    format!(
        "{origin}/authorize?port={port}&name={}&state={state}&challenge={challenge}",
        escape_data_string(client_name)
    )
}

/// What one pickup request learned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickupPoll {
    /// The person decided: a token or a refusal, as the loopback would have carried it.
    Ready(Handoff),
    /// Not yet (pending, unknown, rate-limited, unreachable): ask again later.
    Wait,
    /// The server refused the request itself (a malformed body, an unsupported route
    /// shape): asking again will not help, so the pickup stops and the loopback carries on.
    Stop,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PickupAnswer {
    token: Option<String>,
    error: Option<String>,
}

/// Ask the server once. No `x-api-key`, no redirects (the client follows none), and
/// only ever the origin the sign-in started with.
pub async fn poll_pickup(api: &Api, origin: &str, state: &str, verifier: &str) -> PickupPoll {
    let url = format!("{}{PICKUP_PATH}", origin.trim_end_matches('/'));
    let body = serde_json::json!({ "state": state, "verifier": verifier });
    let sent = api
        .http()
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body.to_string())
        .timeout(PICKUP_REQUEST_TIMEOUT)
        .send()
        .await;
    let Ok(response) = sent else {
        return PickupPoll::Wait;
    };
    let status = response.status().as_u16();
    match status {
        200 => {
            let Ok(Some(bytes)) = Api::read_limited(response, 16 * 1024).await else {
                return PickupPoll::Wait;
            };
            let answer: PickupAnswer = serde_json::from_slice(&bytes).unwrap_or_default();
            if answer.token.is_none() && answer.error.is_none() {
                // A 200 that is not ours (a proxy's page): not an answer.
                return PickupPoll::Wait;
            }
            PickupPoll::Ready(Handoff {
                state: Some(state.to_string()),
                token: answer.token,
                error: answer.error,
            })
        }
        400 | 405 | 413 | 415 => PickupPoll::Stop,
        _ => PickupPoll::Wait,
    }
}

/// Poll until the server has the outcome. Never returns when the server refuses the
/// request outright: the loopback listener (and the deadline) still decide then.
async fn pick_up(api: &Api, origin: &str, state: &str, verifier: &str) -> Handoff {
    loop {
        tokio::time::sleep(PICKUP_INTERVAL).await;
        match poll_pickup(api, origin, state, verifier).await {
            PickupPoll::Ready(handoff) => {
                log::info!("Modlight sign-in: the outcome arrived through the server pickup.");
                return handoff;
            }
            PickupPoll::Wait => {}
            PickupPoll::Stop => {
                log::info!(
                    "Modlight sign-in: the server refused the pickup; waiting on the loopback only."
                );
                return std::future::pending().await;
            }
        }
    }
}

/// The desktop half of the loopback redirect.
pub struct LoopbackListener {
    listener: TcpListener,
    port: u16,
    /// The only origin a CORS preflight is answered for: the server's.
    allowed_origin: String,
}

impl LoopbackListener {
    /// Bind `127.0.0.1` on a port the OS picks.
    pub async fn bind(allowed_origin: &str) -> std::io::Result<LoopbackListener> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        Ok(LoopbackListener {
            listener,
            port,
            allowed_origin: allowed_origin.trim_end_matches('/').to_string(),
        })
    }

    /// The port the authorize URL has to point at.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Wait until a body carrying `expected_state` arrives, or `cancel` fires.
    ///
    /// Anything else that connects (a port scanner, a probe, a request that stops
    /// halfway, a body with somebody else's state) is answered and forgotten.
    pub async fn wait_for(
        &self,
        expected_state: &str,
        cancel: &CancellationToken,
    ) -> Option<Handoff> {
        let (sender, mut receiver) = mpsc::channel::<Handoff>(MAX_CONNECTIONS);
        let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
        let origin = Arc::new(self.allowed_origin.clone());

        loop {
            tokio::select! {
                _ = cancel.cancelled() => return None,
                Some(handoff) = receiver.recv() => {
                    if nonce_equals(handoff.state.as_deref(), expected_state) {
                        return Some(handoff);
                    }
                    log::info!("Modlight sign-in: ignored a loopback body with the wrong state.");
                }
                accepted = self.listener.accept() => {
                    let Ok((stream, _)) = accepted else { continue };
                    let Ok(permit) = slots.clone().try_acquire_owned() else {
                        // Too many at once: drop it rather than queue unbounded work.
                        drop(stream);
                        continue;
                    };
                    let sender = sender.clone();
                    let origin = origin.clone();
                    tokio::spawn(async move {
                        let _permit = permit;
                        let read = tokio::time::timeout(CONNECTION_TIMEOUT, serve(stream, &origin)).await;
                        if let Ok(Some(handoff)) = read {
                            let _ = sender.send(handoff).await;
                        }
                    });
                }
            }
        }
    }
}

/// Read one request from one connection and answer it.
async fn serve(mut stream: TcpStream, allowed_origin: &str) -> Option<Handoff> {
    let mut received = Vec::with_capacity(4096);
    let mut buffer = [0u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut buffer).await.ok()?;
        if read == 0 {
            return None;
        }
        received.extend_from_slice(&buffer[..read]);
        if let Some(end) = find(&received, b"\r\n\r\n") {
            break end;
        }
        if received.len() > MAX_HEADER_BYTES {
            return None;
        }
    };
    if header_end > MAX_HEADER_BYTES {
        return None;
    }

    // Request line and headers are ASCII by definition; only the body may not be.
    let head = String::from_utf8_lossy(&received[..header_end]).to_string();
    let method = head.split(' ').next().unwrap_or_default().to_string();
    let request_origin = header(&head, "origin");

    if method == "OPTIONS" {
        // Answered so a browser that insists on a preflight gets one; every permission,
        // the private-network one included, goes to the server's own origin only. A
        // public server's page needs that one to reach 127.0.0.1 in Chrome (Private
        // Network Access). Nothing is read from a preflight.
        let mut response = String::from("HTTP/1.1 204 No Content\r\n");
        if request_origin.as_deref() == Some(allowed_origin) {
            response.push_str(&format!(
                "Access-Control-Allow-Origin: {allowed_origin}\r\n\
                 Access-Control-Allow-Methods: POST\r\n\
                 Access-Control-Allow-Headers: content-type\r\n\
                 Access-Control-Max-Age: 600\r\n\
                 Vary: Origin\r\n"
            ));
            if header(&head, "access-control-request-private-network")
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("true"))
            {
                response.push_str("Access-Control-Allow-Private-Network: true\r\n");
            }
        }
        response.push_str("Connection: close\r\n\r\n");
        let _ = write(&mut stream, &response).await;
        return None;
    }

    if method != "POST" {
        let _ = write(
            &mut stream,
            "HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;
        return None;
    }

    let length: usize = header(&head, "content-length")
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0);
    if length == 0 || length > MAX_BODY_BYTES {
        let _ = write(
            &mut stream,
            "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;
        return None;
    }

    let body_start = header_end + 4;
    let mut body: Vec<u8> = received[body_start.min(received.len())..].to_vec();
    body.truncate(length);
    while body.len() < length {
        let read = stream.read(&mut buffer).await.ok()?;
        if read == 0 {
            return None;
        }
        let wanted = (length - body.len()).min(read);
        body.extend_from_slice(&buffer[..wanted]);
    }

    // 204 before parsing, without Content-Length (RFC 9110 forbids one on a 204). The
    // page fetches with mode "no-cors" and cannot read this reply anyway.
    let mut response = String::from("HTTP/1.1 204 No Content\r\n");
    if request_origin.as_deref() == Some(allowed_origin) {
        response.push_str(&format!(
            "Access-Control-Allow-Origin: {allowed_origin}\r\nVary: Origin\r\n"
        ));
    }
    response.push_str("Connection: close\r\n\r\n");
    let _ = write(&mut stream, &response).await;

    serde_json::from_slice::<Handoff>(&body).ok()
}

async fn write(stream: &mut TcpStream, response: &str) -> std::io::Result<()> {
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await
}

fn find(data: &[u8], pattern: &[u8]) -> Option<usize> {
    data.windows(pattern.len())
        .position(|window| window == pattern)
}

/// The value of one header, by case-insensitive name.
fn header(head: &str, name: &str) -> Option<String> {
    head.split('\n').skip(1).find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().to_string())
    })
}

/// A successful sign-in: the origin we signed in to, the token and who it belongs to.
#[derive(Clone, PartialEq)]
pub struct SignedIn {
    /// The host we signed in to, never the one `/api/me` reported: that one is where the
    /// account's LINKS are built, while every call goes to the deployment that issued
    /// the token.
    pub origin: String,
    pub token: String,
    pub account: AccountInfo,
}

impl std::fmt::Debug for SignedIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignedIn")
            .field("origin", &self.origin)
            .field("token", &"<redacted>")
            .field("account", &self.account)
            .finish()
    }
}

/// Why a sign-in did not end in a token (`ModlightSignInStatus` with its details).
#[derive(Debug, Clone, PartialEq)]
pub enum SignInFailure {
    /// What was typed is not a server address.
    InvalidHost,
    /// The loopback port could not be opened.
    ListenerFailed(String),
    /// Nobody answered before the deadline.
    TimedOut,
    /// The app stopped waiting.
    Canceled,
    /// The person said no in the browser; `Some(reason)` when it was not plain
    /// `access_denied`.
    Denied(Option<String>),
    /// The answer carried no usable token.
    NoToken,
    /// The server could not be reached to check the token.
    Unreachable { origin: String, message: String },
    /// The server did not recognise the token.
    TokenRejected,
}

/// Run the whole browser flow: bind, open the authorize page, wait, check the token.
///
/// `open_url` is called once with the authorize URL. Nothing is stored here: the caller
/// keeps the credential, after this returns success.
pub async fn sign_in_with_browser(
    api: &Api,
    host_input: &str,
    client_name: &str,
    open_url: impl FnOnce(&str),
    cancel: &CancellationToken,
) -> Result<SignedIn, SignInFailure> {
    let origin = try_normalize_host(host_input).ok_or(SignInFailure::InvalidHost)?;

    let listener = LoopbackListener::bind(&origin)
        .await
        .map_err(|e| SignInFailure::ListenerFailed(e.to_string()))?;
    let state = new_state();
    let verifier = new_verifier();

    open_url(&authorize_url(
        &origin,
        listener.port(),
        client_name,
        &state,
        &challenge_for(&verifier),
    ));
    log::info!(
        "Modlight sign-in: waiting on 127.0.0.1:{} and on the server pickup",
        listener.port()
    );

    // Both paths at once; the first answer wins and the other is dropped with the select.
    let deadline = CancellationToken::new();
    let waiting = listener.wait_for(&state, &deadline);
    let picked = pick_up(api, &origin, &state, &verifier);
    let handoff = tokio::select! {
        handoff = waiting => handoff,
        handoff = picked => Some(handoff),
        _ = cancel.cancelled() => { deadline.cancel(); return Err(SignInFailure::Canceled); }
        _ = tokio::time::sleep(APPROVAL_TIMEOUT) => { deadline.cancel(); return Err(SignInFailure::TimedOut); }
    };
    deadline.cancel();
    drop(listener);

    let Some(handoff) = handoff else {
        return Err(SignInFailure::Canceled);
    };

    if let Some(error) = handoff.error.filter(|e| !e.is_empty()) {
        return Err(SignInFailure::Denied(if error == DENIED_ERROR {
            None
        } else {
            Some(error)
        }));
    }

    let token = handoff.token.unwrap_or_default();
    if !is_plausible_token(&token) {
        return Err(SignInFailure::NoToken);
    }

    use_token(api, &origin, &token, cancel).await
}

/// Check a token against the server and return it if it works (the manual route too).
/// Nothing may be stored before `/api/me` answers.
pub async fn use_token(
    api: &Api,
    host_input: &str,
    token: &str,
    cancel: &CancellationToken,
) -> Result<SignedIn, SignInFailure> {
    let origin = try_normalize_host(host_input).ok_or(SignInFailure::InvalidHost)?;
    let trimmed = token.trim();
    if !is_plausible_token(trimmed) {
        return Err(SignInFailure::NoToken);
    }

    let credential = Credential::new(origin.clone(), trimmed);
    let answer = tokio::select! {
        answer = api.get_account(&credential) => answer,
        _ = cancel.cancelled() => return Err(SignInFailure::Canceled),
    };

    match answer {
        Ok(Some(account)) => Ok(SignedIn {
            origin,
            token: trimmed.to_string(),
            account,
        }),
        Ok(None) => Err(SignInFailure::TokenRejected),
        Err(ApiError::Cancelled) => Err(SignInFailure::Canceled),
        Err(error) => Err(SignInFailure::Unreachable {
            origin,
            message: error.to_string(),
        }),
    }
}

/// Whatever this account can be called on screen: name, else e-mail, else tag.
pub fn display_name(account: &AccountInfo) -> String {
    [&account.name, &account.email, &account.tag]
        .into_iter()
        .flatten()
        .find(|value| !value.trim().is_empty())
        .cloned()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_are_long_and_unreserved() {
        let a = new_state();
        let b = new_state();
        assert_eq!(a.len(), STATE_LENGTH);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(a, b);
    }

    #[test]
    fn nonces_compare_exactly() {
        assert!(nonce_equals(Some("abc"), "abc"));
        assert!(!nonce_equals(Some("abd"), "abc"));
        assert!(!nonce_equals(Some("ab"), "abc"));
        assert!(!nonce_equals(None, "abc"));
    }

    #[test]
    fn the_authorize_link_names_the_machine() {
        let name = client_name("Modlight", "Marcin's Mac mini");
        assert_eq!(name, "Modlight on Marcin's Mac mini");
        assert_eq!(
            client_name("Modlight", &"x".repeat(100)).chars().count(),
            64
        );
        assert_eq!(
            authorize_url("http://127.0.0.1:8420", 50123, &name, "S7ate", "Ch4llenge"),
            "http://127.0.0.1:8420/authorize?port=50123&name=Modlight%20on%20Marcin%27s%20Mac%20mini&state=S7ate&challenge=Ch4llenge"
        );
    }

    #[test]
    fn verifiers_and_challenges_are_rfc_7636() {
        let v = new_verifier();
        assert_eq!(v.len(), 43);
        assert!(
            v.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
        assert_ne!(v, new_verifier());
        // RFC 7636 appendix B.
        assert_eq!(
            challenge_for("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert_eq!(challenge_for(&v).len(), 43);
    }

    #[test]
    fn display_names_fall_back_in_order() {
        let mut account = AccountInfo {
            tag: Some("ToqOse".into()),
            ..Default::default()
        };
        assert_eq!(display_name(&account), "ToqOse");
        account.email = Some("a@b.c".into());
        assert_eq!(display_name(&account), "a@b.c");
        account.name = Some("  ".into());
        assert_eq!(display_name(&account), "a@b.c");
        account.name = Some("Alex".into());
        assert_eq!(display_name(&account), "Alex");
    }
}
