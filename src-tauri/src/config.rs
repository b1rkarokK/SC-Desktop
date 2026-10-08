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
}

impl Default for DiscordConfig {
    fn default() -> Self {
        Self { enabled: true, app_id: DISCORD_APP_ID.into(), progress: true, button: true, hide_on_pause: true }
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
    pub discord: DiscordConfig,
    pub start_minimized: bool,
    /// «Моя волна» without liked tracks and their re-uploads
    pub wave_no_liked: bool,
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
            discord: DiscordConfig::default(),
            start_minimized: true,
            wave_no_liked: true,
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
