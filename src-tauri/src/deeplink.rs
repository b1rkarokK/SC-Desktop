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
/// must not steal the links from the program that is installed. Written through the registry API
/// (no helper programs are started, which security software dislikes).
#[cfg(windows)]
pub fn register() {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
    };
    if cfg!(debug_assertions) || crate::secrets::profile().is_some() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let command = format!("\"{}\" \"%1\"", exe.display());

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
    // the default value of the key (name = null) or a named one
    fn read(path: &str, name: Option<&str>) -> Option<String> {
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: opens a key of the current user for reading; closed below.
        if unsafe { RegCreateKeyExW(HKEY_CURRENT_USER, wide(path).as_ptr(), 0, std::ptr::null(), REG_OPTION_NON_VOLATILE, KEY_READ, std::ptr::null(), &mut key, std::ptr::null_mut()) } != 0 {
            return None;
        }
        let n = name.map(wide);
        let mut size = 0u32;
        let name_ptr = n.as_ref().map_or(std::ptr::null(), |v| v.as_ptr());
        // SAFETY: asks for the size, then reads into a buffer of that size.
        let ok = unsafe { RegQueryValueExW(key, name_ptr, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), &mut size) } == 0;
        let mut out = None;
        if ok && size > 0 {
            let mut buf = vec![0u16; (size as usize).div_ceil(2)];
            // SAFETY: `buf` holds `size` bytes.
            if unsafe { RegQueryValueExW(key, name_ptr, std::ptr::null(), std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size) } == 0 {
                out = Some(String::from_utf16_lossy(&buf).trim_end_matches('\0').to_owned());
            }
        }
        // SAFETY: `key` was opened above.
        unsafe { RegCloseKey(key) };
        out
    }
    fn write(path: &str, name: Option<&str>, value: &str) {
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: creates / opens a key of the current user for writing; closed below.
        if unsafe { RegCreateKeyExW(HKEY_CURRENT_USER, wide(path).as_ptr(), 0, std::ptr::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, std::ptr::null(), &mut key, std::ptr::null_mut()) } != 0 {
            return;
        }
        let n = name.map(wide);
        let v = wide(value);
        let name_ptr = n.as_ref().map_or(std::ptr::null(), |x| x.as_ptr());
        // SAFETY: `v` is a NUL-terminated UTF-16 string of `len * 2` bytes.
        unsafe {
            RegSetValueExW(key, name_ptr, 0, REG_SZ, v.as_ptr().cast(), (v.len() * 2) as u32);
            RegCloseKey(key);
        }
    }

    let open = r"Software\Classes\scdesk\shell\open\command";
    // nothing to do when it already points here
    if read(open, None).as_deref() == Some(command.as_str()) {
        return;
    }
    write(r"Software\Classes\scdesk", None, "URL:SC Desk");
    write(r"Software\Classes\scdesk", Some("URL Protocol"), "");
    write(open, None, &command);
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
