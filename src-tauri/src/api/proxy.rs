//! Optional, explicit proxy. Never enabled implicitly.
//!
//! Typical use: a local VPN client exposing a SOCKS inbound, e.g.
//! `socks5h://127.0.0.1:10808` (`socks5h` resolves DNS on the proxy side).

use url::Url;

use crate::error::{AppError, AppResult};

const SCHEMES: [&str; 4] = ["socks5", "socks5h", "http", "https"];

/// Normalises user input: empty → `None`, otherwise validates scheme/host.
pub fn validate(input: Option<&str>) -> AppResult<Option<String>> {
    let Some(raw) = input.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let url = Url::parse(raw).map_err(|e| AppError::Config(format!("адрес прокси: {e}")))?;
    if !SCHEMES.contains(&url.scheme()) {
        return Err(AppError::Config(format!(
            "схема прокси '{}' не поддерживается, используйте socks5://, socks5h://, http:// или https://",
            url.scheme()
        )));
    }
    if url.host_str().is_none() || url.port_or_known_default().is_none() {
        return Err(AppError::Config("в адресе прокси нужен хост и порт".into()));
    }
    Ok(Some(raw.to_owned()))
}

pub fn build(raw: &str) -> AppResult<reqwest::Proxy> {
    let url = validate(Some(raw))?.ok_or_else(|| AppError::Config("пустой адрес прокси".into()))?;
    reqwest::Proxy::all(url).map_err(|e| AppError::Config(e.without_url().to_string()))
}

/// For logs: hides user:password.
pub fn redact(raw: &str) -> String {
    match Url::parse(raw) {
        Ok(mut u) => {
            if !u.username().is_empty() {
                let _ = u.set_username("***");
            }
            if u.password().is_some() {
                let _ = u.set_password(Some("***"));
            }
            u.to_string()
        }
        Err(_) => "<invalid>".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates() {
        assert_eq!(validate(Some("  ")).unwrap(), None);
        assert!(validate(Some("socks5h://127.0.0.1:10808")).unwrap().is_some());
        assert!(validate(Some("vless://abc@host:443")).is_err());
        assert!(validate(Some("not a url")).is_err());
    }

    #[test]
    fn redacts_credentials() {
        let r = redact("socks5://user:secret@10.0.0.1:1080");
        assert!(!r.contains("secret") && !r.contains("user"));
    }
}
