//! Credentials live only in the OS keyring
//! (Windows Credential Manager / macOS Keychain / Secret Service).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::AppResult;

const SERVICE: &str = "dev.scdesk.app";

/// A second copy for tests (env `SCDESK_PROFILE`, e.g. a second account) lives apart: own keyring entry.
pub fn profile() -> Option<String> {
    std::env::var("SCDESK_PROFILE").ok().map(|p| p.trim().to_owned()).filter(|p| !p.is_empty())
}
const ACCOUNT: &str = "credentials";

#[derive(Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub client_id: String,
    pub oauth_token: String,
    #[serde(default)]
    pub genius_token: Option<String>,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("client_id", &"<redacted>")
            .field("oauth_token", &"<redacted>")
            .field("genius_token", &self.genius_token.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

fn entry() -> AppResult<keyring::Entry> {
    let service = match profile() {
        Some(p) => format!("{SERVICE}.{p}"),
        None => SERVICE.to_owned(),
    };
    Ok(keyring::Entry::new(&service, ACCOUNT)?)
}

pub fn load() -> AppResult<Option<Credentials>> {
    match entry()?.get_password() {
        Ok(json) => Ok(Some(serde_json::from_str(&json)?)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn save(creds: &Credentials) -> AppResult<()> {
    entry()?.set_password(&serde_json::to_string(creds)?)?;
    Ok(())
}

pub fn clear() -> AppResult<()> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}
