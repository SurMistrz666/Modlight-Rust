//! The API token in the login Keychain (the macOS counterpart of DPAPI), written and
//! read through `/usr/bin/security`.
//!
//! The secret never appears on a command line: writes go through `security -i`, which
//! reads its command from standard input, and the value is stored hex-encoded so no
//! quoting rule of that parser can be tripped by the token's characters. Reads use
//! `find-generic-password -w`, whose output is captured and never logged.

use std::io::Write;
use std::process::{Command, Stdio};

use modlight_core::account::CredentialStore;

const TOOL: &str = "/usr/bin/security";

/// One generic-password item: service + account.
pub struct KeychainStore {
    service: String,
    account: String,
}

impl KeychainStore {
    /// The item the app keeps its token in.
    pub fn for_app() -> KeychainStore {
        KeychainStore {
            service: "Modlight".to_string(),
            account: "api-token".to_string(),
        }
    }

    pub fn new(service: &str, account: &str) -> KeychainStore {
        KeychainStore {
            service: service.to_string(),
            account: account.to_string(),
        }
    }
}

/// Only characters `security -i` reads as a plain word.
fn plain(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

impl CredentialStore for KeychainStore {
    fn load(&self) -> Option<String> {
        let output = Command::new(TOOL)
            .args([
                "find-generic-password",
                "-a",
                &self.account,
                "-s",
                &self.service,
                "-w",
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let hex_text = String::from_utf8(output.stdout).ok()?;
        let bytes = hex::decode(hex_text.trim()).ok()?;
        String::from_utf8(bytes).ok().filter(|t| !t.is_empty())
    }

    fn save(&self, token: &str) -> Result<(), String> {
        if !plain(&self.service) || !plain(&self.account) {
            return Err("invalid keychain item name".into());
        }
        let command = format!(
            "add-generic-password -U -a {} -s {} -l {} -w {}\n",
            self.account,
            self.service,
            self.service,
            hex::encode(token.as_bytes())
        );
        let mut child = Command::new(TOOL)
            .arg("-i")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        child
            .stdin
            .take()
            .ok_or("no stdin")?
            .write_all(command.as_bytes())
            .map_err(|e| e.to_string())?;
        let status = child.wait().map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("security exited with {status}"));
        }
        // `security -i` reports a failed command on stderr but may still exit 0, so the
        // write is confirmed by reading it back.
        match self.load() {
            Some(stored) if stored == token => Ok(()),
            _ => Err("the keychain did not keep the token".into()),
        }
    }

    fn delete(&self) -> Result<(), String> {
        let status = Command::new(TOOL)
            .args([
                "delete-generic-password",
                "-a",
                &self.account,
                "-s",
                &self.service,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| e.to_string())?;
        // 44 is errSecItemNotFound as an exit code: nothing to delete is fine.
        if status.success() || status.code() == Some(44) {
            Ok(())
        } else {
            Err(format!("security exited with {status}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::plain;

    #[test]
    fn item_names_are_plain_words() {
        assert!(plain("Modlight"));
        assert!(plain("api-token"));
        assert!(!plain("a b"));
        assert!(!plain("a\"b"));
        assert!(!plain(""));
    }
}
