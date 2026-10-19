//! SMTP delivery that behaves like nodemailer 8.0.10 (`lib/smtp-transport`
//! and `lib/smtp-connection`), for the options lib/mail.ts sets: one
//! connection per message, implicit TLS (`secure`), mandatory STARTTLS
//! (`requireTLS`) or nodemailer's default upgrade-when-advertised, password
//! authentication, and the 10 s / 10 s / 20 s connection, greeting and socket
//! timeouts.
//!
//! Why not lettre's transport: the SMTP test button reports nodemailer's own
//! error vocabulary to the operator (`err.code`, `err.responseCode`,
//! `err.response`, `err.message`), and the decisions that produce it differ
//! from lettre's in ways an operator sees: nodemailer authenticates only when
//! the server advertises AUTH, sends STARTTLS whenever it is advertised (and
//! fails when the upgrade is refused), falls back to HELO, picks the AUTH
//! mechanism by its own order (PLAIN, LOGIN, CRAM-MD5), reports a refused
//! connection as a socket error (`ESOCKET`, "connect ECONNREFUSED ip:port"),
//! and words every failure in its own way. Each step below names the
//! nodemailer function it follows. lettre still provides the TLS stream
//! (rustls with the Mozilla roots, as Node's bundled roots) and the MIME body.
//!
//! Known differences: the text of TLS errors is rustls' rather than OpenSSL's,
//! and the DNS step uses the system resolver (getaddrinfo) where nodemailer
//! asks the DNS servers first and falls back to getaddrinfo.

use std::collections::VecDeque;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use futures_util::io::{AsyncReadExt, AsyncWriteExt};
#[allow(deprecated)]
use lettre::transport::smtp::client::{AsyncNetworkStream, TlsParameters};
use tokio::time::Instant;

/// `connectionTimeout`, `greetingTimeout`, `socketTimeout` of lib/mail.ts.
pub const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);
pub const GREETING_TIMEOUT: Duration = Duration::from_secs(10);
pub const SOCKET_TIMEOUT: Duration = Duration::from_secs(20);

/// What smtp-transport waits after an unexplained close before it reports
/// "Unexpected socket close".
const UNEXPECTED_CLOSE_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credentials {
    pub user: String,
    pub pass: String,
}

/// The transport options lib/mail.ts builds (`transportOptions`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtpOptions {
    pub host: String,
    /// As configured: 0 means the default for the mode, and anything outside
    /// 0..65535 is refused the way Node's `net.connect` refuses it.
    pub port: i64,
    /// `secure`: implicit TLS from the first byte.
    pub secure: bool,
    /// `requireTLS`: STARTTLS even when not advertised, and no HELO fallback.
    pub require_tls: bool,
    pub auth: Option<Credentials>,
    pub connection_timeout: Duration,
    pub greeting_timeout: Duration,
    pub socket_timeout: Duration,
}

/// A nodemailer error: `err.code`, `err.message`, `err.response`,
/// `err.responseCode`, `err.command`.
#[derive(Debug, Clone, PartialEq)]
pub struct SmtpError {
    pub code: Option<&'static str>,
    pub message: String,
    pub response: Option<String>,
    pub response_code: Option<f64>,
    pub command: Option<String>,
}

impl std::fmt::Display for SmtpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SmtpError {}

/// `_formatError(message, type, response, command)`: the response (when it
/// is a non-empty string) is appended to the message, and its leading number
/// becomes `responseCode` unless that number is 0.
pub fn format_error(
    message: impl Into<String>,
    code: &'static str,
    response: Option<&str>,
    command: &str,
) -> SmtpError {
    let mut message = message.into();
    let response = response.filter(|r| !r.is_empty());
    if let Some(r) = response {
        message.push_str(": ");
        message.push_str(r);
    }
    let response_code = response.and_then(leading_number).filter(|n| *n != 0.0);
    SmtpError {
        code: if code.is_empty() || code == "Error" { None } else { Some(code) },
        message,
        response: response.map(str::to_string),
        response_code,
        command: if command.is_empty() { None } else { Some(command.to_string()) },
    }
}

/// `Number((response.match(/^\d+/) || [])[0])`
fn leading_number(s: &str) -> Option<f64> {
    let digits: String = s.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() { None } else { digits.parse::<f64>().ok() }
}

/// The SMTP envelope (`message.getEnvelope()`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub from: String,
    pub to: Vec<String>,
}

/// What a successful send reports (`info`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendInfo {
    pub accepted: Vec<String>,
    pub rejected: Vec<String>,
    /// The server's answer to the message.
    pub response: String,
}

/// Node's error code names for the I/O errors a socket reports.
fn errno_name(e: &io::Error) -> String {
    use io::ErrorKind::*;
    let name = match e.kind() {
        ConnectionRefused => "ECONNREFUSED",
        ConnectionReset => "ECONNRESET",
        ConnectionAborted => "ECONNABORTED",
        BrokenPipe => "EPIPE",
        TimedOut => "ETIMEDOUT",
        HostUnreachable => "EHOSTUNREACH",
        NetworkUnreachable => "ENETUNREACH",
        NetworkDown => "ENETDOWN",
        AddrNotAvailable => "EADDRNOTAVAIL",
        AddrInUse => "EADDRINUSE",
        PermissionDenied => "EACCES",
        NotConnected => "ENOTCONN",
        _ => return e.to_string(),
    };
    name.to_string()
}

/// How Node words a failed `connect()`: `connect ECONNREFUSED 127.0.0.1:25`.
fn connect_error(e: &io::Error, addr: &SocketAddr) -> SmtpError {
    let message = format!("connect {} {}:{}", errno_name(e), addr.ip(), addr.port());
    format_error(message, "ESOCKET", None, "CONN")
}

/// A failed TLS handshake, as Node's TLS socket reports it (`ESOCKET`): a
/// peer that hangs up mid-handshake, a reset, or the TLS library's own words
/// (rustls' here, OpenSSL's upstream: the one wording that cannot match).
fn tls_failure(e: &lettre::transport::smtp::Error) -> SmtpError {
    let mut source: &dyn std::error::Error = e;
    let mut io_kind = None;
    loop {
        if let Some(io) = source.downcast_ref::<io::Error>() {
            io_kind = Some(io.kind());
        }
        match source.source() {
            Some(inner) => source = inner,
            None => break,
        }
    }
    let message = match io_kind {
        Some(io::ErrorKind::UnexpectedEof) => {
            "Client network socket disconnected before secure TLS connection was established".to_string()
        }
        Some(io::ErrorKind::ConnectionReset) => "read ECONNRESET".to_string(),
        _ => source.to_string(),
    };
    format_error(message, "ESOCKET", None, "CONN")
}

/// `shared.resolveHostname`: an IP literal as is, otherwise every address of
/// the name, IPv4 first. A failure is `EDNS` with getaddrinfo's wording.
async fn resolve(host: &str) -> Result<Vec<IpAddr>, SmtpError> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![ip]);
    }
    match tokio::net::lookup_host((host, 0)).await {
        Ok(addrs) => {
            let all: Vec<IpAddr> = addrs.map(|a| a.ip()).collect();
            let mut out: Vec<IpAddr> = Vec::new();
            for ip in all.iter().filter(|ip| ip.is_ipv4()).chain(all.iter().filter(|ip| ip.is_ipv6())) {
                if !out.contains(ip) {
                    out.push(*ip);
                }
            }
            if out.is_empty() {
                return Err(dns_error(host, None));
            }
            Ok(out)
        }
        Err(e) => Err(dns_error(host, Some(&e))),
    }
}

/// Node maps EAI_NONAME / EAI_NODATA to ENOTFOUND and keeps the other EAI_*
/// names: `getaddrinfo ENOTFOUND smtp.example.invalid`.
fn dns_error(host: &str, e: Option<&io::Error>) -> SmtpError {
    let text = e.map(|e| e.to_string().to_ascii_lowercase()).unwrap_or_default();
    let code = if text.contains("temporary failure") || text.contains("try again") { "EAI_AGAIN" } else { "ENOTFOUND" };
    format_error(format!("getaddrinfo {code} {host}"), "EDNS", None, "CONN")
}

/// `_getHostname()`: the machine's name when it is fully qualified,
/// `[127.0.0.1]` otherwise; an IPv4 literal in brackets.
pub fn client_hostname() -> String {
    let name = lettre::transport::smtp::extension::ClientId::default().to_string();
    let name = if name.contains('.') { name } else { "[127.0.0.1]".to_string() };
    let is_ipv4 = name.split('.').count() == 4
        && name.split('.').all(|p| (1..=3).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit()));
    if is_ipv4 { format!("[{name}]") } else { name }
}

/// `decodeServerResponse`: UTF-8 when the bytes are valid UTF-8, else the
/// bytes as latin1 characters.
fn decode_server_response(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| char::from(b)).collect(),
    }
}

/// `/^\d+-/` on a line: a multi-line response continues.
fn continues(line: &[u8]) -> bool {
    let digits = line.iter().take_while(|b| b.is_ascii_digit()).count();
    digits > 0 && line.get(digits) == Some(&b'-')
}

/// What a read ended with when no response came.
enum ReadEnd {
    /// The peer closed the connection; the unterminated rest of its last line.
    Closed(String),
    Timeout,
    Error(io::Error),
}

/// Which response is awaited (decides how an unexplained close is reported).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Awaiting {
    Greeting,
    Other,
}

#[allow(deprecated)]
struct Connection<'a> {
    opts: &'a SmtpOptions,
    stream: AsyncNetworkStream,
    /// Complete lines not yet grouped into a returned response.
    queue: VecDeque<Vec<Vec<u8>>>,
    /// The unterminated rest of the input (`_remainder`).
    remainder: Vec<u8>,
    secure: bool,
    name: String,
    allows_auth: bool,
    supported_auth: Vec<&'static str>,
    extensions: Vec<&'static str>,
    /// A TLS upgrade did not complete: lettre has already given the socket
    /// up, and the stream must not be touched again (not even to close it).
    broken: bool,
}

#[allow(deprecated)]
impl<'a> Connection<'a> {
    /// `connect()`: resolve, try a random address first and the others in
    /// order (`_fallbackAddresses`), each within the connection timeout, the
    /// TLS handshake included for `secure`.
    async fn open(opts: &'a SmtpOptions) -> Result<Connection<'a>, SmtpError> {
        let ips = resolve(&opts.host).await?;
        // `Number(options.port) || (secure ? 465 : 587)`
        let port = match opts.port {
            0 if opts.secure => 465,
            0 => 587,
            p => u16::try_from(p).map_err(|_| {
                let message = format!("Port should be >= 0 and < 65536. Received type number ({p}).");
                format_error(message, "ECONNECTION", None, "CONN")
            })?,
        };
        let first = if ips.len() > 1 { rand::random::<usize>() % ips.len() } else { 0 };
        let mut order = vec![ips[first]];
        order.extend(ips.iter().enumerate().filter(|(i, _)| *i != first).map(|(_, ip)| *ip));

        let mut last = None;
        for ip in order {
            let addr = SocketAddr::new(ip, port);
            match tokio::time::timeout(opts.connection_timeout, Self::connect_one(opts, addr)).await {
                Ok(Ok(stream)) => {
                    return Ok(Connection {
                        opts,
                        stream,
                        queue: VecDeque::new(),
                        remainder: Vec::new(),
                        secure: opts.secure,
                        name: client_hostname(),
                        allows_auth: false,
                        supported_auth: Vec::new(),
                        extensions: Vec::new(),
                        broken: false,
                    });
                }
                Ok(Err(e)) => last = Some(e),
                Err(_) => last = Some(format_error("Connection timeout", "ETIMEDOUT", None, "CONN")),
            }
        }
        Err(last.unwrap_or_else(|| format_error("Connection timeout", "ETIMEDOUT", None, "CONN")))
    }

    async fn connect_one(opts: &SmtpOptions, addr: SocketAddr) -> Result<AsyncNetworkStream, SmtpError> {
        let tcp = tokio::net::TcpStream::connect(addr).await.map_err(|e| connect_error(&e, &addr))?;
        let _ = tcp.set_nodelay(true);
        let mut stream = AsyncNetworkStream::use_existing_tokio1(Box::new(tcp));
        if opts.secure {
            let params = TlsParameters::new(opts.host.clone()).map_err(|e| tls_failure(&e))?;
            stream.upgrade_tls(params).await.map_err(|e| tls_failure(&e))?;
        }
        Ok(stream)
    }

    /// `_sendCommand(str)`
    async fn send_command(&mut self, line: &str) -> Result<(), SmtpError> {
        let mut bytes = line.as_bytes().to_vec();
        bytes.extend_from_slice(b"\r\n");
        self.write_all(&bytes).await
    }

    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), SmtpError> {
        let write = async {
            self.stream.write_all(bytes).await?;
            self.stream.flush().await
        };
        match tokio::time::timeout(self.opts.socket_timeout, write).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(format_error(format!("write {}", errno_name(&e)), "ESOCKET", None, "CONN")),
            Err(_) => Err(format_error("Timeout", "ETIMEDOUT", None, "CONN")),
        }
    }

    /// The next complete response (`_onData` + `_processResponse`): lines
    /// split on `\r?\n`, a line following one that starts `NNN-` belongs to
    /// the same response, lines joined with `\n`.
    async fn read_raw(&mut self, deadline: Option<Instant>) -> Result<String, ReadEnd> {
        loop {
            if let Some(front) = self.queue.front() {
                let complete = self.queue.len() > 1 || !front.last().is_some_and(|l| continues(l));
                if complete {
                    let lines = self.queue.pop_front().unwrap_or_default();
                    return Ok(decode_server_response(&lines.join(&b'\n')));
                }
            }
            let mut chunk = [0u8; 8192];
            let wait = match deadline {
                Some(d) => d.saturating_duration_since(Instant::now()).min(self.opts.socket_timeout),
                None => self.opts.socket_timeout,
            };
            let n = match tokio::time::timeout(wait, self.stream.read(&mut chunk)).await {
                Err(_) => return Err(ReadEnd::Timeout),
                // A TLS peer that closes without close_notify is a close to
                // Node, not an error.
                Ok(Err(e)) if e.kind() == io::ErrorKind::UnexpectedEof => 0,
                Ok(Err(e)) => return Err(ReadEnd::Error(e)),
                Ok(Ok(n)) => n,
            };
            if n == 0 {
                let rest = decode_server_response(&self.remainder);
                return Err(ReadEnd::Closed(crate::config::js_trim(&rest).to_string()));
            }
            self.remainder.extend_from_slice(&chunk[..n]);
            while let Some(pos) = self.remainder.iter().position(|&b| b == b'\n') {
                let mut line: Vec<u8> = self.remainder.drain(..=pos).collect();
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                match self.queue.back_mut() {
                    Some(last) if last.last().is_some_and(|l| continues(l)) => last.push(line),
                    _ => self.queue.push_back(vec![line]),
                }
            }
        }
    }

    /// A response, or the error nodemailer reports when none comes.
    async fn read(&mut self, awaiting: Awaiting) -> Result<String, SmtpError> {
        let deadline = (awaiting == Awaiting::Greeting).then(|| Instant::now() + self.opts.greeting_timeout);
        match self.read_raw(deadline).await {
            Ok(s) => Ok(s),
            Err(ReadEnd::Timeout) if awaiting == Awaiting::Greeting => {
                Err(format_error("Greeting never received", "ETIMEDOUT", None, "CONN"))
            }
            Err(ReadEnd::Timeout) => Err(format_error("Timeout", "ETIMEDOUT", None, "CONN")),
            Err(ReadEnd::Error(e)) => Err(format_error(format!("read {}", errno_name(&e)), "ESOCKET", None, "CONN")),
            Err(ReadEnd::Closed(rest)) => Err(self.closed(awaiting, &rest).await),
        }
    }

    /// `_onClose`: while waiting for the greeting, a close is only an error
    /// when the last words were a 4xx/5xx reply; otherwise the connection
    /// just "ends" and smtp-transport reports it a second later.
    async fn closed(&self, awaiting: Awaiting, rest: &str) -> SmtpError {
        let response = (!rest.is_empty()).then_some(rest);
        if awaiting != Awaiting::Greeting {
            return format_error("Connection closed unexpectedly", "ECONNECTION", response, "CONN");
        }
        let b = rest.as_bytes();
        let is_error_reply = b.len() >= 3
            && matches!(b[0], b'4' | b'5')
            && b[1].is_ascii_digit()
            && b[2].is_ascii_digit()
            && b.get(3).is_none_or(|c| !(c.is_ascii_alphanumeric() || *c == b'_'));
        if is_error_reply {
            return format_error("Connection closed unexpectedly", "ECONNECTION", response, "CONN");
        }
        tokio::time::sleep(UNEXPECTED_CLOSE_DELAY).await;
        format_error("Unexpected socket close", "", None, "")
    }

    /// `_actionGreeting`, then `_actionEHLO` / `_actionHELO` / `_actionSTARTTLS`.
    async fn handshake(&mut self) -> Result<(), SmtpError> {
        let greeting = self.read(Awaiting::Greeting).await?;
        if !greeting.starts_with("220") {
            return Err(format_error(
                format!("Invalid greeting. response={greeting}"),
                "EPROTOCOL",
                Some(&greeting),
                "CONN",
            ));
        }
        loop {
            self.send_command(&format!("EHLO {}", self.name)).await?;
            let str = self.read(Awaiting::Other).await?;
            if str.starts_with("421") {
                return Err(format_error(
                    format!("Server terminates connection. response={str}"),
                    "ECONNECTION",
                    Some(&str),
                    "EHLO",
                ));
            }
            if !str.starts_with('2') {
                if self.opts.require_tls {
                    return Err(format_error(
                        format!("EHLO failed but HELO does not support required STARTTLS. response={str}"),
                        "ECONNECTION",
                        Some(&str),
                        "EHLO",
                    ));
                }
                // Try HELO instead.
                self.send_command(&format!("HELO {}", self.name)).await?;
                let helo = self.read(Awaiting::Other).await?;
                if !helo.starts_with('2') {
                    return Err(format_error(
                        format!("Invalid HELO. response={helo}"),
                        "EPROTOCOL",
                        Some(&helo),
                        "HELO",
                    ));
                }
                // Assume that authentication is enabled (most probably it is not).
                self.allows_auth = true;
                return Ok(());
            }

            if !self.secure && (ehlo_has(&str, "STARTTLS") || self.opts.require_tls) {
                self.send_command("STARTTLS").await?;
                let answer = self.read(Awaiting::Other).await?;
                if !answer.starts_with('2') {
                    return Err(format_error(
                        "Error upgrading connection with STARTTLS",
                        "ETLS",
                        Some(&answer),
                        "STARTTLS",
                    ));
                }
                self.upgrade().await?;
                // Restart the session over TLS.
                continue;
            }

            for ext in ["SMTPUTF8", "DSN", "8BITMIME", "REQUIRETLS", "PIPELINING"] {
                if ehlo_has(&str, ext) {
                    self.extensions.push(ext);
                }
            }
            self.allows_auth = ehlo_has_auth(&str);
            for mechanism in ["PLAIN", "LOGIN", "CRAM-MD5", "XOAUTH2"] {
                if ehlo_offers(&str, mechanism) {
                    self.supported_auth.push(mechanism);
                }
            }
            return Ok(());
        }
    }

    /// `_upgradeConnection`: a failed handshake is a socket error of the new
    /// TLS socket, silence the socket timeout.
    async fn upgrade(&mut self) -> Result<(), SmtpError> {
        let params = TlsParameters::new(self.opts.host.clone()).map_err(|e| tls_failure(&e))?;
        // Until the handshake succeeds the plain socket is gone (a failed or
        // abandoned upgrade leaves lettre's stream empty).
        self.broken = true;
        match tokio::time::timeout(self.opts.socket_timeout, self.stream.upgrade_tls(params)).await {
            Err(_) => Err(format_error("Timeout", "ETIMEDOUT", None, "CONN")),
            Ok(Err(e)) => Err(tls_failure(&e)),
            Ok(Ok(())) => {
                self.broken = false;
                self.secure = true;
                self.remainder.clear();
                self.queue.clear();
                Ok(())
            }
        }
    }

    /// `login(auth)`: the first mechanism nodemailer recognised in the EHLO
    /// answer (its own order), PLAIN when none.
    async fn login(&mut self, auth: &Credentials) -> Result<(), SmtpError> {
        let method = self.supported_auth.first().copied().unwrap_or("PLAIN");
        if method != "XOAUTH2" && (auth.user.is_empty() || auth.pass.is_empty()) {
            return Err(format_error(format!("Missing credentials for \"{method}\""), "EAUTH", None, "API"));
        }
        match method {
            "LOGIN" => {
                self.send_command("AUTH LOGIN").await?;
                let str = self.read(Awaiting::Other).await?;
                if !is_334(&str) {
                    return Err(format_error(
                        "Invalid login sequence while waiting for \"334 VXNlcm5hbWU6\"",
                        "EAUTH",
                        Some(&str),
                        "AUTH LOGIN",
                    ));
                }
                self.send_command(&STANDARD.encode(auth.user.as_bytes())).await?;
                let str = self.read(Awaiting::Other).await?;
                if !is_334(&str) {
                    return Err(format_error(
                        "Invalid login sequence while waiting for \"334 UGFzc3dvcmQ6\"",
                        "EAUTH",
                        Some(&str),
                        "AUTH LOGIN",
                    ));
                }
                self.send_command(&STANDARD.encode(auth.pass.as_bytes())).await?;
                let str = self.read(Awaiting::Other).await?;
                self.auth_complete(str, method).await
            }
            "CRAM-MD5" => {
                self.send_command("AUTH CRAM-MD5").await?;
                let str = self.read(Awaiting::Other).await?;
                // `/^334\s+(.+)$/`
                let challenge = str
                    .strip_prefix("334")
                    .filter(|rest| rest.starts_with(char::is_whitespace))
                    .map(str::trim_start)
                    .filter(|c| !c.is_empty() && !c.contains('\n') && !c.contains('\r'));
                let Some(challenge) = challenge else {
                    return Err(format_error(
                        "Invalid login sequence while waiting for server challenge string",
                        "EAUTH",
                        Some(&str),
                        "AUTH CRAM-MD5",
                    ));
                };
                // `Buffer.from(c, "base64").toString("ascii")` drops the high bit.
                let decoded: Vec<u8> =
                    crate::secret_box::node_base64_decode(challenge).iter().map(|b| b & 0x7f).collect();
                let digest = hmac_md5(auth.pass.as_bytes(), &decoded);
                let answer = format!("{} {}", auth.user, hex::encode(digest));
                self.send_command(&STANDARD.encode(answer.as_bytes())).await?;
                let str = self.read(Awaiting::Other).await?;
                let ok = str.strip_prefix("235").is_some_and(|rest| rest.starts_with(char::is_whitespace));
                if !ok {
                    return Err(format_error(
                        "Invalid login sequence while waiting for \"235\"",
                        "EAUTH",
                        Some(&str),
                        "AUTH CRAM-MD5",
                    ));
                }
                Ok(())
            }
            // Only XOAUTH2 advertised: nodemailer then reaches for an OAuth2
            // token provider that a password login does not have.
            "XOAUTH2" => Err(format_error("Cannot read properties of undefined (reading 'getToken')", "", None, "")),
            _ => {
                // PLAIN, skipping the authorization identity ("it causes
                // problems with some servers").
                let payload = STANDARD.encode(format!("\0{}\0{}", auth.user, auth.pass).as_bytes());
                self.send_command(&format!("AUTH PLAIN {payload}")).await?;
                let str = self.read(Awaiting::Other).await?;
                self.auth_complete(str, "PLAIN").await
            }
        }
    }

    /// `_actionAUTHComplete`: a 334 challenge is answered with an empty line
    /// until the server decides.
    async fn auth_complete(&mut self, mut str: String, method: &str) -> Result<(), SmtpError> {
        while str.starts_with("334") {
            self.send_command("").await?;
            str = self.read(Awaiting::Other).await?;
        }
        if !str.starts_with('2') {
            return Err(format_error("Invalid login", "EAUTH", Some(&str), &format!("AUTH {method}")));
        }
        Ok(())
    }

    /// `send(envelope, message)`: `_setEnvelope`, MAIL FROM, RCPT TO (all at
    /// once with PIPELINING), DATA, the dot-stuffed message.
    async fn send(&mut self, envelope: &Envelope, message: &[u8]) -> Result<SendInfo, SmtpError> {
        let from = crate::config::js_trim(&envelope.from).to_string();
        let to: Vec<String> = envelope.to.iter().map(|t| crate::config::js_trim(t).to_string()).collect();
        let unsafe_chars = |s: &str| s.contains(['\r', '\n', '<', '>']);
        if to.is_empty() {
            return Err(format_error("No recipients defined", "EENVELOPE", None, "API"));
        }
        if !from.is_empty() && unsafe_chars(&from) {
            return Err(format_error(format!("Invalid sender {}", json_string(&from)), "EENVELOPE", None, "API"));
        }
        let mut utf8 = !from.is_ascii();
        for t in &to {
            if t.is_empty() || unsafe_chars(t) {
                return Err(format_error(format!("Invalid recipient {}", json_string(t)), "EENVELOPE", None, "API"));
            }
            utf8 |= !t.is_ascii();
        }
        let using_utf8 = utf8 && self.extensions.contains(&"SMTPUTF8");
        let args = if using_utf8 { " SMTPUTF8" } else { "" };
        self.send_command(&format!("MAIL FROM:<{from}>{args}")).await?;
        let str = self.read(Awaiting::Other).await?;
        if !str.starts_with('2') {
            let message = if using_utf8 && str.starts_with("550 ") && !from.is_ascii() {
                "Internationalized mailbox name not allowed"
            } else {
                "Mail command failed"
            };
            return Err(format_error(message, "EENVELOPE", Some(&str), "MAIL FROM"));
        }

        let pipelining = self.extensions.contains(&"PIPELINING");
        let mut accepted = Vec::new();
        let mut rejected = Vec::new();
        let mut last = String::new();
        if pipelining {
            for t in &to {
                self.send_command(&format!("RCPT TO:<{t}>")).await?;
            }
        }
        for t in &to {
            if !pipelining {
                self.send_command(&format!("RCPT TO:<{t}>")).await?;
            }
            last = self.read(Awaiting::Other).await?;
            if last.starts_with('2') { accepted.push(t.clone()) } else { rejected.push(t.clone()) }
        }
        if rejected.len() >= to.len() {
            return Err(format_error(
                "Can't send mail - all recipients were rejected",
                "EENVELOPE",
                Some(&last),
                "RCPT TO",
            ));
        }

        self.send_command("DATA").await?;
        let str = self.read(Awaiting::Other).await?;
        if !(str.starts_with('2') || str.starts_with('3')) {
            return Err(format_error("Data command failed", "EENVELOPE", Some(&str), "DATA"));
        }
        self.write_all(&dot_stuff(message)).await?;
        let str = self.read(Awaiting::Other).await?;
        if !str.starts_with('2') {
            return Err(format_error("Message failed", "EMESSAGE", Some(&str), "DATA"));
        }
        Ok(SendInfo { accepted, rejected, response: str })
    }

    /// `close()`: end the socket (nodemailer sends no QUIT after a message).
    async fn close(mut self) {
        if self.broken {
            return;
        }
        let _ = tokio::time::timeout(Duration::from_secs(2), self.stream.close()).await;
    }
}

fn is_334(s: &str) -> bool {
    s.starts_with("334 ") || s.starts_with("334-")
}

fn json_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

/// `/[ -]NAME\b/i` over the whole EHLO answer.
fn ehlo_has(ehlo: &str, name: &str) -> bool {
    let upper = ehlo.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    upper.match_indices(name).any(|(i, _)| {
        i > 0
            && matches!(bytes[i - 1], b' ' | b'-')
            && bytes.get(i + name.len()).is_none_or(|c| !(c.is_ascii_alphanumeric() || *c == b'_'))
    })
}

/// `/[ -]AUTH\b/i`
fn ehlo_has_auth(ehlo: &str) -> bool {
    ehlo_has(ehlo, "AUTH")
}

/// `/[ -]AUTH(?:(\s+|=)[^\n]*\s+|\s+|=)NAME/i`: the mechanism on an AUTH line.
fn ehlo_offers(ehlo: &str, mechanism: &str) -> bool {
    let pattern = format!(r"(?i)[ -]AUTH(?:(\s+|=)[^\n]*\s+|\s+|=){}", regex::escape(mechanism));
    regex::Regex::new(&pattern).is_ok_and(|re| re.is_match(ehlo))
}

/// `DataStream`: a dot at the start of a line is doubled, a bare LF becomes
/// CRLF, and the message ends with `<CRLF>.<CRLF>`.
pub fn dot_stuff(message: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(message.len() + 8);
    let mut prev: Option<u8> = None;
    for &b in message {
        if b == b'.' && prev.is_none_or(|p| p == b'\n') {
            out.extend_from_slice(b"..");
        } else if b == b'\n' && prev != Some(b'\r') {
            out.extend_from_slice(b"\r\n");
        } else {
            out.push(b);
        }
        prev = Some(b);
    }
    match prev {
        Some(b'\n') => out.extend_from_slice(b".\r\n"),
        Some(b'\r') => out.extend_from_slice(b"\n.\r\n"),
        _ => out.extend_from_slice(b"\r\n.\r\n"),
    }
    out
}

/// Send one message on a fresh connection (smtp-transport `send`): connect,
/// greet, upgrade, authenticate when the server allows it, deliver, close.
pub async fn send(opts: &SmtpOptions, envelope: &Envelope, message: &[u8]) -> Result<SendInfo, SmtpError> {
    let mut conn = Connection::open(opts).await?;
    let result = async {
        conn.handshake().await?;
        if let Some(auth) = &opts.auth
            && conn.allows_auth
        {
            conn.login(auth).await?;
        }
        conn.send(envelope, message).await
    }
    .await;
    conn.close().await;
    result
}

// ── HMAC-MD5 for AUTH CRAM-MD5 (RFC 1321, RFC 2104) ─────────────────────────

fn md5(input: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14,
        20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6,
        10, 15, 21,
    ];
    let k: Vec<u32> = (0..64).map(|i| ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32).collect();
    let (mut a0, mut b0, mut c0, mut d0) = (0x67452301u32, 0xefcdab89u32, 0x98badcfeu32, 0x10325476u32);
    let mut msg = input.to_vec();
    let bit_len = (input.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());
    for chunk in msg.chunks(64) {
        let m: Vec<u32> = chunk.chunks(4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect();
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = [0u8; 16];
    for (i, v) in [a0, b0, c0, d0].iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

fn hmac_md5(key: &[u8], message: &[u8]) -> [u8; 16] {
    let mut k = if key.len() > 64 { md5(key).to_vec() } else { key.to_vec() };
    k.resize(64, 0);
    let mut inner: Vec<u8> = k.iter().map(|b| b ^ 0x36).collect();
    inner.extend_from_slice(message);
    let mut outer: Vec<u8> = k.iter().map(|b| b ^ 0x5c).collect();
    outer.extend_from_slice(&md5(&inner));
    md5(&outer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_and_hmac_match_the_rfc_vectors() {
        assert_eq!(hex::encode(md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex::encode(md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            hex::encode(md5(b"12345678901234567890123456789012345678901234567890123456789012345678901234567890")),
            "57edf4a22be3c955ac49da2e2107b67a"
        );
        // RFC 2104 test case 2.
        assert_eq!(hex::encode(hmac_md5(b"Jefe", b"what do ya want for nothing?")), "750c783e6ab0b503eaa86e310a5db738");
        // RFC 2195 (CRAM-MD5) example.
        let challenge = "<1896.697170952@postoffice.reston.mci.net>";
        assert_eq!(
            hex::encode(hmac_md5(b"tanstaaftanstaaf", challenge.as_bytes())),
            "b913a602c7eda7a495b4e6e7334d3890"
        );
    }

    #[test]
    fn errors_are_formatted_like_nodemailer() {
        let e = format_error("Invalid login", "EAUTH", Some("535 5.7.8 Authentication failed"), "AUTH PLAIN");
        assert_eq!(e.message, "Invalid login: 535 5.7.8 Authentication failed");
        assert_eq!(e.response_code, Some(535.0));
        assert_eq!(e.code, Some("EAUTH"));
        let e = format_error("Invalid greeting. response=", "EPROTOCOL", Some(""), "CONN");
        assert_eq!(e.message, "Invalid greeting. response=");
        assert_eq!(e.response, None);
        assert_eq!(format_error("x", "EAUTH", Some("000 zero"), "").response_code, None);
        let refused = io::Error::from(io::ErrorKind::ConnectionRefused);
        let e = connect_error(&refused, &"127.0.0.1:2525".parse().unwrap());
        assert_eq!(e.message, "connect ECONNREFUSED 127.0.0.1:2525");
        assert_eq!(e.code, Some("ESOCKET"));
        assert_eq!(connect_error(&refused, &"[::1]:25".parse().unwrap()).message, "connect ECONNREFUSED ::1:25");
    }

    #[test]
    fn ehlo_capabilities_are_read_with_the_upstream_patterns() {
        let ehlo = "250-sink.test greets you\n250-PIPELINING\n250-AUTH LOGIN PLAIN\n250-8BITMIME\n250 STARTTLS";
        assert!(ehlo_has(ehlo, "STARTTLS"));
        assert!(ehlo_has(ehlo, "PIPELINING"));
        assert!(!ehlo_has(ehlo, "SMTPUTF8"));
        assert!(ehlo_has_auth(ehlo));
        assert!(ehlo_offers(ehlo, "PLAIN") && ehlo_offers(ehlo, "LOGIN"));
        assert!(!ehlo_offers(ehlo, "CRAM-MD5"));
        assert!(ehlo_offers("250 AUTH=CRAM-MD5", "CRAM-MD5"));
        assert!(!ehlo_has("250 XSTARTTLS", "STARTTLS"));
        assert!(!ehlo_has("250 STARTTLSX", "STARTTLS"));
        assert!(ehlo_has("250-starttls", "STARTTLS"));
    }

    #[test]
    fn dot_stuffing_matches_the_data_stream() {
        assert_eq!(dot_stuff(b".a\r\n.b\nc"), b"..a\r\n..b\r\nc\r\n.\r\n");
        assert_eq!(dot_stuff(b"x\r\n"), b"x\r\n.\r\n");
        assert_eq!(dot_stuff(b"x\r"), b"x\r\n.\r\n");
    }

    /// A scripted SMTP server: answers each received line with the next
    /// reply of `script` (`None` = hang up), after its greeting.
    async fn scripted(greeting: &'static str, script: Vec<Option<&'static str>>) -> u16 {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt as _, BufReader};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let (read, mut write) = socket.into_split();
            write.write_all(greeting.as_bytes()).await.unwrap();
            let mut lines = BufReader::new(read).lines();
            let mut in_data = false;
            let mut script = script.into_iter();
            while let Ok(Some(line)) = lines.next_line().await {
                if in_data && line != "." {
                    continue;
                }
                match script.next().flatten() {
                    Some(reply) => {
                        in_data = reply.starts_with("354");
                        write.write_all(reply.as_bytes()).await.unwrap();
                    }
                    None => return,
                }
            }
        });
        port
    }

    fn options(port: u16, auth: bool) -> SmtpOptions {
        SmtpOptions {
            host: "127.0.0.1".into(),
            port: i64::from(port),
            secure: false,
            require_tls: false,
            auth: auth.then(|| Credentials { user: "u".into(), pass: "p".into() }),
            connection_timeout: CONNECTION_TIMEOUT,
            greeting_timeout: GREETING_TIMEOUT,
            socket_timeout: SOCKET_TIMEOUT,
        }
    }

    fn envelope(to: &[&str]) -> Envelope {
        Envelope { from: "no-reply@sink.test".into(), to: to.iter().map(|t| t.to_string()).collect() }
    }

    #[tokio::test]
    async fn delivers_with_auth_plain_and_reports_the_accepted_recipients() {
        let port = scripted(
            "220 sink\r\n",
            vec![
                Some("250-sink\r\n250 AUTH LOGIN PLAIN\r\n"),
                Some("235 ok\r\n"),
                Some("250 ok\r\n"),
                Some("250 ok\r\n"),
                Some("550 5.1.1 no such user\r\n"),
                Some("354 go\r\n"),
                Some("250 queued\r\n"),
            ],
        )
        .await;
        let info = send(&options(port, true), &envelope(&["a@x.test", "b@x.test"]), b"Subject: x\r\n\r\nhi\r\n")
            .await
            .unwrap();
        assert_eq!(info.accepted, vec!["a@x.test"]);
        assert_eq!(info.rejected, vec!["b@x.test"]);
        assert_eq!(info.response, "250 queued");
    }

    #[tokio::test]
    async fn reports_failures_in_nodemailers_words() {
        let port = scripted("220 sink\r\n", vec![Some("250 AUTH PLAIN\r\n"), Some("535 5.7.8 nope\r\n")]).await;
        let e = send(&options(port, true), &envelope(&["a@x.test"]), b"x").await.unwrap_err();
        assert_eq!((e.code, e.message.as_str()), (Some("EAUTH"), "Invalid login: 535 5.7.8 nope"));

        let port = scripted("554 go away\r\n", vec![]).await;
        let e = send(&options(port, false), &envelope(&["a@x.test"]), b"x").await.unwrap_err();
        assert_eq!(e.message, "Invalid greeting. response=554 go away: 554 go away");
        assert_eq!((e.code, e.response_code), (Some("EPROTOCOL"), Some(554.0)));

        let port = scripted("220 sink\r\n", vec![Some("250 sink\r\n")]).await;
        let e = send(&options(port, false), &envelope(&[]), b"x").await.unwrap_err();
        assert_eq!((e.code, e.message.as_str()), (Some("EENVELOPE"), "No recipients defined"));

        // Offered STARTTLS, accepted it, then hung up on the handshake.
        let port = scripted("220 sink\r\n", vec![Some("250 STARTTLS\r\n"), Some("220 go ahead\r\n"), None]).await;
        let e = send(&options(port, false), &envelope(&["a@x.test"]), b"x").await.unwrap_err();
        assert_eq!(e.code, Some("ESOCKET"));
        assert_eq!(e.message, "Client network socket disconnected before secure TLS connection was established");

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let e = send(&options(port, false), &envelope(&["a@x.test"]), b"x").await.unwrap_err();
        assert_eq!(e.message, format!("connect ECONNREFUSED 127.0.0.1:{port}"));
    }

    #[test]
    fn multi_line_responses_continue_after_a_dash() {
        assert!(continues(b"250-PIPELINING"));
        assert!(!continues(b"250 OK"));
        assert!(!continues(b"-x"));
    }
}
