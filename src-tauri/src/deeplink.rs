//! Links `scdesk://room/<code>` and `scdesk://playlist/<code>#<key>`: Windows opens SC Desk with them
//! (from the page a Discord status button leads to, from a message, from the address bar).
//!
//! The scheme is written to the current user's registry (no admin rights) when a released build
//! starts, pointing at the program that runs now; the installer does the same. A copy launched
//! while one is already running hands its link over through the single-instance hook. The window
//! asks before it joins anything.

use std::sync::Mutex;

use tauri::{AppHandle, Emitter};

/// A link that arrived before the window could listen.
static PENDING: Mutex<Option<String>> = Mutex::new(None);

const PREFIX: &str = "scdesk://";

/// The first `scdesk://` argument of a command line.
pub fn from_args<I: IntoIterator<Item = String>>(args: I) -> Option<String> {
    args.into_iter().find(|a| a.to_ascii_lowercase().starts_with(PREFIX))
}

/// Only room and playlist links, with a plausible code; anything else is dropped.
pub fn valid(url: &str) -> bool {
    let Some(rest) = url.get(PREFIX.len()..).filter(|_| url.to_ascii_lowercase().starts_with(PREFIX)) else { return false };
    let (kind, tail) = rest.split_once('/').unwrap_or((rest, ""));
    let code: String = tail.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    matches!(kind, "room" | "playlist") && (16..=64).contains(&code.len()) && tail.len() <= 200
}

/// A link came in: show the window and let it ask the user.
pub fn handle(app: &AppHandle, url: String) {
    if !valid(&url) {
        tracing::debug!("deep link ignored");
        return;
    }
    *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = Some(url.clone());
    crate::tray::show_main(app);
    let _ = app.emit("deeplink", url);
}

/// The window asks for what arrived before it was ready (once).
pub fn take() -> Option<String> {
    PENDING.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// `scdesk://` → this program, for the current user. Released builds only: a development build
/// must not steal the links from the program that is installed.
#[cfg(windows)]
pub fn register() {
    use std::os::windows::process::CommandExt;
    if cfg!(debug_assertions) || crate::secrets::profile().is_some() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let command = format!("\"{}\" \"%1\"", exe.display());
    let run = |args: &[&str]| {
        let _ = std::process::Command::new("reg").args(args).creation_flags(0x0800_0000).output();
    };
    // nothing to do when it already points here
    let key = r"HKCU\Software\Classes\scdesk\shell\open\command";
    if let Ok(out) = std::process::Command::new("reg").args(["query", key, "/ve"]).creation_flags(0x0800_0000).output() {
        if String::from_utf8_lossy(&out.stdout).contains(&command) {
            return;
        }
    }
    run(&["add", r"HKCU\Software\Classes\scdesk", "/ve", "/d", "URL:SC Desk", "/f"]);
    run(&["add", r"HKCU\Software\Classes\scdesk", "/v", "URL Protocol", "/d", "", "/f"]);
    run(&["add", key, "/ve", "/d", &command, "/f"]);
}

#[cfg(not(windows))]
pub fn register() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_the_link_from_the_command_line_and_checks_it() {
        let args = vec!["C:\\x\\sc-desk.exe".to_owned(), "scdesk://room/abcdefghijklmnop1234".to_owned()];
        assert_eq!(from_args(args).as_deref(), Some("scdesk://room/abcdefghijklmnop1234"));
        assert!(valid("scdesk://room/abcdefghijklmnop1234"));
        assert!(valid("SCDESK://playlist/abcdefghijklmnop1234#some-key_1"));
        assert!(!valid("scdesk://room/short"));
        assert!(!valid("scdesk://evil/abcdefghijklmnop1234"));
        assert!(!valid("https://example.com/room/abcdefghijklmnop1234"));
        assert!(from_args(vec!["a".into(), "b".into()]).is_none());
    }
}
