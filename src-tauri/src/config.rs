use std::{
    fs,
    path::{Path, PathBuf},
    sync::RwLock,
};

use serde::{Deserialize, Serialize};

use crate::error::AppResult;

/// Discord application «SC Desk» (public identifier, same for every user).
pub const DISCORD_APP_ID: &str = "1557882934890078278";

/// Chrome major version used for the User-Agent and client hints.
/// Keep it close to the current stable Chrome; editable in Settings.
pub const DEFAULT_CHROME_VERSION: u32 = 152;

pub const EQ_BANDS: [f32; 10] = [32.0, 64.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NetMode {
    #[default]
    Direct,
    Proxy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EqConfig {
    pub enabled: bool,
    /// dB, applied before the bands
    pub preamp: f32,
    /// dB per band, see `EQ_BANDS`
    pub gains: [f32; 10],
    pub preset: String,
}

impl Default for EqConfig {
    fn default() -> Self {
        Self { enabled: false, preamp: 0.0, gains: [0.0; 10], preset: "flat".into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DiscordConfig {
    pub enabled: bool,
    /// Application ID from discord.com/developers — its name is shown as "Слушает <name>".
    pub app_id: String,
    pub progress: bool,
    pub button: bool,
    pub hide_on_pause: bool,
    /// show a "join" button for an open room (no password) in the status
    pub room_join: bool,
    /// the "join" button became on by default (0.9): done once for configs written before (a missing field
    /// reads as false here, not as the struct default, which is what makes the migration run)
    #[serde(default = "no")]
    pub join_default_set: bool,
}

fn no() -> bool {
    false
}

impl Default for DiscordConfig {
    fn default() -> Self {
        Self { enabled: true, app_id: DISCORD_APP_ID.into(), progress: true, button: true, hide_on_pause: true, room_join: true, join_default_set: true }
    }
}

/// Sound effects and transitions (Эквалайзер → «Эффекты»).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FxConfig {
    /// playback rate: < 1 slowed, > 1 sped up (pitch follows)
    pub speed: f32,
    /// reverb amount 0 … 1
    pub reverb: f32,
    /// bring every track to the same loudness
    pub normalize: bool,
    /// seconds of overlap between tracks, 0 = off
    pub crossfade: f32,
}

impl Default for FxConfig {
    fn default() -> Self {
        Self { speed: 1.0, reverb: 0.0, normalize: true, crossfade: 0.0 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub net_mode: NetMode,
    /// Used only when `net_mode == Proxy` (`socks5://`, `socks5h://`, `http://`, `https://`).
    pub proxy: Option<String>,
    pub chrome_version: u32,
    pub volume: f32,
    pub eq: EqConfig,
    pub fx: FxConfig,
    pub discord: DiscordConfig,
    pub start_minimized: bool,
    /// keep SoundCloud's player warmed up for protected tracks (+50-70 MB, instant start)
    #[serde(default = "yes")]
    pub fast_protected: bool,
    /// address of the sync relay (rooms, shared playlists): https://sc-desk-sync.<name>.workers.dev
    #[serde(default)]
    pub sync_url: String,
    /// pause the music while another program records (a call) / while a full-screen game is in front
    #[serde(default)]
    pub auto_pause_call: bool,
    #[serde(default)]
    pub auto_pause_game: bool,
    /// the equalizer is remembered per output device (speakers, headphones…) and switches with it
    #[serde(default = "yes")]
    pub eq_per_device: bool,
    /// the output device picked in the title bar (None: the system's default)
    #[serde(default)]
    pub output_device: Option<String>,
    /// device name -> its equalizer
    #[serde(default)]
    pub eq_by_device: std::collections::BTreeMap<String, EqConfig>,
    /// «Моя волна» without liked tracks and their re-uploads
    pub wave_no_liked: bool,
    /// Windows notification when a followed artist posts a new track
    #[serde(default = "yes")]
    pub notify_new: bool,
    /// Windows notification when somebody you follow opens a room
    #[serde(default = "yes")]
    pub notify_rooms: bool,
    /// autostart was set up once (enabled by default on the first launch)
    pub autostart_initialized: bool,
    /// Frontend-owned preferences (theme, panels, fullscreen view…), stored verbatim.
    pub ui: serde_json::Value,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            net_mode: NetMode::Direct,
            proxy: None,
            chrome_version: DEFAULT_CHROME_VERSION,
            volume: 0.8,
            eq: EqConfig::default(),
            fx: FxConfig::default(),
            discord: DiscordConfig::default(),
            start_minimized: true,
            fast_protected: true,
            sync_url: DEFAULT_SYNC_URL.to_owned(),
            auto_pause_call: false,
            auto_pause_game: false,
            eq_per_device: true,
            eq_by_device: Default::default(),
            wave_no_liked: true,
            notify_new: true,
            output_device: None,
            notify_rooms: true,
            autostart_initialized: false,
            ui:serde_json::Value::Object(Default::default()),
        }
    }
}

impl AppConfig {
    /// The proxy actually in effect.
    pub fn active_proxy(&self) -> Option<&str> {
        match self.net_mode {
            NetMode::Direct => None,
            NetMode::Proxy => self.proxy.as_deref().map(str::trim).filter(|s| !s.is_empty()),
        }
    }
}

/// Plain JSON file in the app config dir. Secrets never go here (see secrets.rs).
pub struct ConfigStore {
    path: PathBuf,
    inner: RwLock<AppConfig>,
}

impl ConfigStore {
    pub fn load(path: PathBuf) -> Self {
        let cfg = match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "config.json is invalid, using defaults");
                AppConfig::default()
            }),
            Err(_) => AppConfig::default(),
        };
        let mut cfg = cfg;
        if !cfg.discord.join_default_set {
            // the join page exists now: the button is on for everybody once (it can be turned off in Settings)
            cfg.discord.room_join = true;
            cfg.discord.join_default_set = true;
        }
        Self { path, inner: RwLock::new(cfg) }
    }

    pub fn get(&self) -> AppConfig {
        self.inner.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn update(&self, f: impl FnOnce(&mut AppConfig)) -> AppResult<AppConfig> {
        let snapshot = {
            let mut cfg = self.inner.write().unwrap_or_else(|p| p.into_inner());
            f(&mut cfg);
            cfg.clone()
        };
        write_atomic(&self.path, &serde_json::to_vec_pretty(&snapshot)?)?;
        Ok(snapshot)
    }
}

fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, data)?;
    fs::rename(tmp, path)
}

fn yes() -> bool {
    true
}

/// The relay that ships with the app.
pub const DEFAULT_SYNC_URL: &str = "https://sc-desk-sync.sc-desk-sync.workers.dev";

/// Always the built-in relay; `SCDESK_SYNC_URL` points a test build at another one (a local
/// `wrangler dev`). Not a user setting: an old saved address is ignored.
pub fn sync_base(_saved: &str) -> String {
    std::env::var("SCDESK_SYNC_URL").ok().map(|u| u.trim().trim_end_matches('/').to_owned()).filter(|u| !u.is_empty()).unwrap_or_else(|| DEFAULT_SYNC_URL.to_owned())
}

#[cfg(test)]
mod join_tests {
    use super::*;

    fn temp(name: &str, json: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("scdesk-cfg-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.json");
        fs::write(&p, json).unwrap();
        p
    }

    #[test]
    fn an_old_config_gets_the_join_button_once_and_the_choice_after_that_sticks() {
        // written before 0.9: the button was off, the marker does not exist
        let p = temp("old", r#"{"discord":{"enabled":true,"room_join":false}}"#);
        let store = ConfigStore::load(p.clone());
        assert!(store.get().discord.room_join, "on after the first start of the new version");
        // the user turns it off in Settings; the marker is saved with it
        store.update(|c| c.discord.room_join = false).unwrap();
        let again = ConfigStore::load(p);
        assert!(!again.get().discord.room_join, "the choice is kept");
        assert!(again.get().discord.join_default_set);
    }

    #[test]
    fn a_new_config_has_the_button_on() {
        let p = temp("new", "{}");
        assert!(ConfigStore::load(p).get().discord.room_join);
    }
}
