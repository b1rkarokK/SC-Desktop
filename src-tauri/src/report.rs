//! «Отчёт об ошибке» (Настройки → Система): the version, the system, how the network is set up and the end of the
//! log in one text file the user can send. Nothing secret goes in: tokens, keys, subscription and server links, the
//! name of the Windows user and every long random-looking string are cut out before the file is written.

use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

use crate::{
    error::{AppError, AppResult},
    state::AppState,
};

/// Hosts whose addresses are kept whole in a report (everything else keeps the host only).
const KNOWN_HOSTS: &[&str] = &[
    "soundcloud.com",
    "sndcdn.com",
    "soundcloud.cloud",
    "lrclib.net",
    "genius.com",
    "musixmatch.com",
    "deezer.com",
    "music.163.com",
    "music.yandex.net",
    "github.com",
    "workers.dev",
    "cloudflare-dns.com",
    "dns.google",
];

const SECRET_KEYS: &[&str] = &["oauth_token", "client_id", "usertoken", "track_authorization", "token", "password", "secret", "key", "x-hwid", "hwid", "authorization"];
const SECRET_SCHEMES: &[&str] = &["vless://", "vmess://", "trojan://", "ss://", "ssr://", "hysteria2://", "hy2://", "hysteria://", "tuic://", "happ://", "socks5://", "socks5h://", "socks4://"];

/// The text with the secrets cut out.
pub fn redact(text: &str) -> String {
    let user = std::env::var("USERNAME").ok().filter(|u| u.len() >= 3);
    text.lines().map(|l| redact_line(l, user.as_deref())).collect::<Vec<_>>().join("\n")
}

fn redact_line(line: &str, user: Option<&str>) -> String {
    let mut s = line.to_owned();
    if let Some(u) = user {
        s = s.replace(&format!("\\Users\\{u}"), "\\Users\\<user>").replace(&format!("/Users/{u}"), "/Users/<user>");
    }
    // "OAuth abc…" (an Authorization header)
    s = cut_after(&s, "OAuth ", "<hidden>");
    // server and proxy links: all of it
    for scheme in SECRET_SCHEMES {
        while let Some(i) = s.to_lowercase().find(scheme) {
            let end = s[i..].find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ')' || c == ',').map_or(s.len(), |e| i + e);
            s.replace_range(i..end, "<link hidden>");
        }
    }
    // key=value and "key":"value" and key: value
    for key in SECRET_KEYS {
        s = cut_value(&s, key);
    }
    // http(s) addresses: known hosts whole (without a query), others the host only
    s = shorten_urls(&s);
    // anything long and random-looking
    s = hide_long_tokens(&s);
    s
}

/// `prefix` followed by a word: the word becomes `to`.
fn cut_after(s: &str, prefix: &str, to: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find(prefix) {
        out.push_str(&rest[..i + prefix.len()]);
        let after = &rest[i + prefix.len()..];
        let end = after.find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',' || c == '&').unwrap_or(after.len());
        out.push_str(to);
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

/// `key=value`, `key: value`, `"key":"value"`: the value is hidden.
fn cut_value(s: &str, key: &str) -> String {
    let lower = s.to_lowercase();
    let mut out = String::new();
    let mut at = 0;
    let bytes = lower.as_bytes();
    while let Some(rel) = lower[at..].find(key) {
        let i = at + rel;
        let after_key = i + key.len();
        // the key is a whole word: not the tail of another word
        let before_ok = i == 0 || !(bytes[i - 1] as char).is_ascii_alphanumeric();
        let mut j = after_key;
        // an optional closing quote, then : or =, then an optional space / quote
        while j < s.len() && (bytes[j] == b'"' || bytes[j] == b'\'') {
            j += 1;
        }
        if !before_ok || j >= s.len() || !(bytes[j] == b'=' || bytes[j] == b':') {
            out.push_str(&s[at..after_key]);
            at = after_key;
            continue;
        }
        j += 1;
        while j < s.len() && (bytes[j] == b' ' || bytes[j] == b'"' || bytes[j] == b'\'') {
            j += 1;
        }
        let end = s[j..].find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',' || c == '&' || c == '}').map_or(s.len(), |e| j + e);
        out.push_str(&s[at..j]);
        out.push_str("<hidden>");
        at = end;
    }
    out.push_str(&s[at..]);
    out
}

fn shorten_urls(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    loop {
        let Some(i) = ["https://", "http://"].iter().filter_map(|p| rest.find(p)).min() else { break };
        out.push_str(&rest[..i]);
        let url_part = &rest[i..];
        let end = url_part.find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ')' || c == '>').unwrap_or(url_part.len());
        let url = &url_part[..end];
        let host = url.split("://").nth(1).unwrap_or("").split(['/', '?', ':']).next().unwrap_or("");
        let known = KNOWN_HOSTS.iter().any(|h| host == *h || host.ends_with(&format!(".{h}")));
        if known {
            // the query goes anyway (it carries ids and keys)
            out.push_str(url.split('?').next().unwrap_or(url));
        } else {
            out.push_str(&format!("{}://{host}/<hidden>", if url.starts_with("https") { "https" } else { "http" }));
        }
        rest = &url_part[end..];
    }
    out.push_str(rest);
    out
}

fn hide_long_tokens(s: &str) -> String {
    let mut out = String::new();
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        if run.chars().count() >= 32 {
            out.push_str("<hidden>");
        } else {
            out.push_str(run);
        }
        run.clear();
    };
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '=' {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

fn windows_version() -> String {
    std::process::Command::new("cmd")
        .args(["/C", "ver"])
        .creation_flags_hidden()
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| std::env::consts::OS.to_owned())
}

trait Hidden {
    fn creation_flags_hidden(&mut self) -> &mut Self;
}

impl Hidden for std::process::Command {
    #[cfg(windows)]
    fn creation_flags_hidden(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        self.creation_flags(0x0800_0000)
    }
    #[cfg(not(windows))]
    fn creation_flags_hidden(&mut self) -> &mut Self {
        self
    }
}

/// The report text: facts, then the end of the log (secrets cut out).
pub async fn text(app: &AppHandle, log_lines: usize) -> AppResult<String> {
    let state = app.state::<AppState>();
    let cfg = state.config.get();
    let net = match cfg.net_mode {
        crate::config::NetMode::Direct => "прямое подключение".to_owned(),
        crate::config::NetMode::Proxy => format!("прокси ({})", if cfg.active_proxy().is_some() { "адрес задан" } else { "адрес пуст" }),
    };
    let ui = &cfg.ui;
    let facts = format!(
        "SC Desk {}\nWindows: {}\nАрхитектура: {}\nСеть: {}\nОбход блокировки: {}\nСвой сервер: {}\nВход в SoundCloud: {}\nЯзык: {}\n",
        env!("CARGO_PKG_VERSION"),
        windows_version(),
        std::env::consts::ARCH,
        net,
        ui["scBypass"].as_str().unwrap_or("auto"),
        if crate::net::vpn::load().selected.is_some() { "выбран" } else { "нет" },
        if state.credentials().is_some() { "есть" } else { "нет" },
        ui["lang"].as_str().unwrap_or("ru"),
    );
    let log = crate::commands::log_tail(app.clone(), log_lines).await.map_err(|e| AppError::Other(e.to_string()))?;
    Ok(format!("{facts}\n--- конец журнала ---\n{}\n", redact(&log)))
}

/// Writes the report to the desktop (a folder with it opens) and returns the path.
pub async fn save(app: &AppHandle) -> AppResult<String> {
    let body = text(app, 1500).await?;
    let dir = app.path().desktop_dir().or_else(|_| app.path().download_dir()).map_err(|e| AppError::Other(e.to_string()))?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let path = dir.join(format!("SC Desk отчёт {stamp}.txt"));
    tokio::fs::write(&path, body).await?;
    let _ = app.opener().reveal_item_in_dir(&path);
    Ok(path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_cut_out() {
        let line = "GET https://api-v2.soundcloud.com/me?client_id=AbCdEf0123456789AbCdEf0123456789&oauth_token=2-123456-789-abcdefg HTTP authorization: OAuth 2-111-222-secret proxy vless://uuid@host:443?type=ws#name sub https://panel.example.net/api/sub/abcd1234efgh5678abcd1234efgh5678";
        let out = redact_line(line, None);
        assert!(!out.contains("AbCdEf0123456789"), "{out}");
        assert!(!out.contains("2-123456-789-abcdefg"), "{out}");
        assert!(!out.contains("2-111-222-secret"), "{out}");
        assert!(!out.contains("uuid@host"), "{out}");
        assert!(!out.contains("abcd1234efgh5678"), "{out}");
        assert!(out.contains("api-v2.soundcloud.com/me"), "the host and the path stay: {out}");
        assert!(out.contains("panel.example.net"), "the host of an unknown address stays: {out}");
    }

    #[test]
    fn the_windows_user_goes() {
        let out = redact_line(r"read C:\Users\petya\AppData\Roaming\x failed", Some("petya"));
        assert_eq!(out, r"read C:\Users\<user>\AppData\Roaming\x failed");
    }

    #[test]
    fn a_plain_line_is_left_alone() {
        let line = "2026-10-13T11:40:25Z DEBUG sc_desk::player: track fetched ms=0 ok=true";
        assert_eq!(redact_line(line, None), line);
    }

    #[test]
    fn json_keys_are_cut() {
        let out = redact_line(r#"{"oauth_token":"2-1-2-abc","name":"x"}"#, None);
        assert!(!out.contains("2-1-2-abc"), "{out}");
        assert!(out.contains("name"), "{out}");
    }
}
