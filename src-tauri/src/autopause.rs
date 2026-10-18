//! Автопауза: музыка сама встаёт на паузу, когда начался звонок (другая программа
//! заняла микрофон), и/или когда запустилась полноэкранная игра, и продолжает, когда
//! всё закончилось. Включается в Настройках, по умолчанию выключено; пока оба
//! переключателя выключены, ничего не опрашивается.
//!
//! Микрофон: Windows ведёт учёт в реестре
//! HKCU\Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone:
//! у каждой программы `LastUsedTimeStart` / `LastUsedTimeStop`; пока идёт запись, `Stop` равен 0.

use std::{sync::Arc, time::Duration};

use tauri::{AppHandle, Manager};

use crate::{player::Player, state::AppState};

const POLL: Duration = Duration::from_secs(3);
/// calm for this many polls in a row before the music continues (a call blinks the microphone)
const CALM_POLLS: u32 = 2;

pub fn start(app: AppHandle) {
    let _ = std::thread::Builder::new().name("autopause".into()).spawn(move || run(app));
}

fn run(app: AppHandle) {
    let mut paused_by_us = false;
    let mut calm = 0u32;
    loop {
        std::thread::sleep(POLL);
        let (call, game) = {
            let Some(state) = app.try_state::<AppState>() else { continue };
            let c = state.config.get();
            (c.auto_pause_call, c.auto_pause_game)
        };
        if !call && !game {
            paused_by_us = false;
            calm = 0;
            continue;
        }
        let Some(player) = app.try_state::<Arc<Player>>() else { continue };
        let busy = (call && imp::mic_in_use()) || (game && imp::game_in_front());
        if busy {
            calm = 0;
            if !paused_by_us && player.snapshot().playing {
                tracing::info!(call, game, "auto-pause: music paused");
                player.pause();
                paused_by_us = true;
            }
        } else if paused_by_us {
            calm += 1;
            if calm >= CALM_POLLS {
                paused_by_us = false;
                calm = 0;
                // only if nobody has touched the player meanwhile
                if !player.snapshot().playing {
                    tracing::info!("auto-pause: music continues");
                    player.resume();
                }
            }
        }
    }
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
    };

    /// Screen recorders and game overlays that take the microphone to put it into a clip, not to talk (NVIDIA's Instant
    /// Replay / ShadowPlay among them): their use of the microphone must not pause the music.
    const RECORDERS: &[&str] = &[
        "nvidia", "nvcontainer", "nvsphelper", "nvoverlay", "shadowplay", "obs64", "obs-studio", "obs studio", "streamlabs", "medal", "xboxgamingoverlay", "gamebar",
        "overwolf", "radeonsoftware", "amdrsserv", "bandicam", "action.exe", "fraps", "lightshot", "sharex", "outplayed",
    ];

    const MIC: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\microphone";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn qword(key: HKEY, name: &str) -> Option<u64> {
        let n = wide(name);
        let mut ty = 0u32;
        let mut buf = [0u8; 8];
        let mut len = 8u32;
        // SAFETY: `key` is an open key, every pointer is a live local of the stated size.
        let r = unsafe { RegQueryValueExW(key, n.as_ptr(), std::ptr::null(), &mut ty, buf.as_mut_ptr(), &mut len) };
        (r == 0 && len == 8).then(|| u64::from_le_bytes(buf))
    }

    /// Is any program (except SC Desk itself) recording right now, among the subkeys of `key`?
    fn any_recording(key: HKEY, depth: u32) -> bool {
        let mut index = 0;
        loop {
            let mut name = [0u16; 512];
            let mut len = name.len() as u32;
            // SAFETY: as above; `name`/`len` describe a writable buffer.
            let r = unsafe { RegEnumKeyExW(key, index, name.as_mut_ptr(), &mut len, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) };
            if r != 0 {
                return false;
            }
            index += 1;
            let sub = String::from_utf16_lossy(&name[..len as usize]);
            let lower = sub.to_lowercase();
            if lower.contains("sc-desk") || lower.contains("sc desk") || lower.contains("scdesk") {
                continue;
            }
            // a program that records the screen (and the microphone with it) is not a call: the music goes on while it records
            if RECORDERS.iter().any(|r| lower.contains(r)) {
                continue;
            }
            let path = wide(&sub);
            let mut h: HKEY = std::ptr::null_mut();
            // SAFETY: opens a subkey of an open key for reading; closed below.
            if unsafe { RegOpenKeyExW(key, path.as_ptr(), 0, KEY_READ, &mut h) } != 0 {
                continue;
            }
            let busy = match (qword(h, "LastUsedTimeStart"), qword(h, "LastUsedTimeStop")) {
                (Some(start), Some(stop)) => start > 0 && stop == 0,
                _ if depth == 0 && sub == "NonPackaged" => any_recording(h, depth + 1),
                _ => false,
            };
            // SAFETY: `h` was opened above.
            unsafe { RegCloseKey(h) };
            if busy {
                return true;
            }
        }
    }

    pub fn mic_in_use() -> bool {
        let path = wide(MIC);
        let mut root: HKEY = std::ptr::null_mut();
        // SAFETY: opens a registry key for reading; closed below.
        if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, KEY_READ, &mut root) } != 0 {
            return false;
        }
        let busy = any_recording(root, 0);
        // SAFETY: `root` was opened above.
        unsafe { RegCloseKey(root) };
        busy
    }

    pub fn game_in_front() -> bool {
        crate::mini::fullscreen_in_front(0)
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn mic_in_use() -> bool {
        false
    }
    pub fn game_in_front() -> bool {
        false
    }
}

/// A game in front or the microphone in use: the time not to spend the machine on anything we can do later.
pub fn busy() -> bool {
    imp::game_in_front() || imp::mic_in_use()
}

#[cfg(test)]
mod tests {
    #[test]
    fn asking_windows_about_the_microphone_does_not_fail() {
        // nobody is recording while the tests run on a quiet machine; the point is that the
        // registry walk works and returns an answer
        let _ = super::imp::mic_in_use();
        let _ = super::imp::game_in_front();
    }
}
