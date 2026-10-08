//! Discord Rich Presence: "Слушает SC Desk" — track, artist, cover, progress
//! bar (start/end timestamps) and a "Слушать на SoundCloud" button.
//!
//! Runs on its own thread (the IPC client is blocking and not Send-friendly).
//! No timers: Discord animates the progress bar itself from the timestamps,
//! we only push on track change / play / pause / seek. If Discord isn't
//! running, the update is dropped and we retry on the next event.

use std::{
    sync::mpsc,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use discord_rich_presence::{
    activity::{Activity, ActivityType, Assets, Button, Timestamps},
    DiscordIpc, DiscordIpcClient,
};

use crate::config::DiscordConfig;

#[derive(Debug, Clone, PartialEq)]
pub struct Presence {
    pub track_id: u64,
    pub title: String,
    pub artist: String,
    pub artwork: Option<String>,
    pub url: Option<String>,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub playing: bool,
}

enum Cmd {
    Set(Option<Presence>),
    Config(DiscordConfig),
}

pub struct Discord {
    tx: mpsc::Sender<Cmd>,
}

impl Discord {
    pub fn start(cfg: DiscordConfig) -> Self {
        let (tx, rx) = mpsc::channel();
        let spawned = thread::Builder::new().name("discord".into()).spawn(move || Worker::new(cfg).run(rx));
        if let Err(e) = spawned {
            tracing::error!(error = %e, "discord thread not started");
        }
        Self { tx }
    }

    pub fn update(&self, presence: Option<Presence>) {
        let _ = self.tx.send(Cmd::Set(presence));
    }

    pub fn configure(&self, cfg: DiscordConfig) {
        let _ = self.tx.send(Cmd::Config(cfg));
    }
}

struct Worker {
    cfg: DiscordConfig,
    client: Option<DiscordIpcClient>,
    last: Option<Presence>,
    /// what Discord currently shows (for de-duplication: Discord rate-limits updates)
    shown: Option<(u64, bool, i64)>,
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

impl Worker {
    fn new(cfg: DiscordConfig) -> Self {
        Self { cfg, client: None, last: None, shown: None }
    }

    fn run(mut self, rx: mpsc::Receiver<Cmd>) {
        while let Ok(cmd) = rx.recv() {
            // coalesce bursts (e.g. seek drag): only the newest state matters
            let mut cmd = Some(cmd);
            thread::sleep(Duration::from_millis(150));
            while let Ok(next) = rx.try_recv() {
                if let Some(Cmd::Config(c)) = cmd.replace(next) {
                    self.apply_config(c);
                }
            }
            match cmd {
                Some(Cmd::Set(p)) => {
                    self.last = p;
                    self.push();
                }
                Some(Cmd::Config(c)) => {
                    self.apply_config(c);
                    self.shown = None;
                    self.push();
                }
                None => {}
            }
        }
        self.disconnect();
    }

    fn apply_config(&mut self, c: DiscordConfig) {
        if c.app_id != self.cfg.app_id || !c.enabled {
            self.disconnect();
        }
        self.cfg = c;
    }

    fn disconnect(&mut self) {
        if let Some(mut c) = self.client.take() {
            let _ = c.clear_activity();
            let _ = c.close();
        }
        self.shown = None;
    }

    fn ensure_client(&mut self) -> bool {
        if self.client.is_some() {
            return true;
        }
        let id = match self.cfg.app_id.trim() {
            "" => crate::config::DISCORD_APP_ID,
            custom => custom,
        };
        match DiscordIpcClient::new(id) {
            Ok(mut c) => match c.connect() {
                Ok(()) => {
                    tracing::info!("discord connected");
                    self.client = Some(c);
                    true
                }
                Err(e) => {
                    tracing::debug!(error = %e, "discord not running");
                    false
                }
            },
            Err(e) => {
                tracing::warn!(error = %e, "discord client error");
                false
            }
        }
    }

    fn push(&mut self) {
        let want = match (&self.last, self.cfg.enabled) {
            (Some(p), true) if p.playing || !self.cfg.hide_on_pause => Some(p.clone()),
            _ => None,
        };
        let Some(p) = want else {
            if self.shown.is_some() {
                if let Some(c) = self.client.as_mut() {
                    let _ = c.clear_activity();
                }
                self.shown = None;
            }
            return;
        };

        let start = now_secs() - (p.position_ms / 1000) as i64;
        let key = (p.track_id, p.playing, start);
        if let Some((id, playing, s)) = self.shown {
            if id == key.0 && playing == key.1 && (s - start).abs() <= 2 {
                return;
            }
        }
        if !self.ensure_client() {
            return;
        }
        if self.send(&p, start).is_err() {
            // pipe broken (Discord restarted): reconnect once
            self.client = None;
            if !self.ensure_client() || self.send(&p, start).is_err() {
                self.client = None;
                return;
            }
        }
        self.shown = Some(key);
    }

    fn send(&mut self, p: &Presence, start: i64) -> Result<(), ()> {
        let state = if p.playing { p.artist.clone() } else { format!("{} · на паузе", p.artist) };
        let details = truncate(&p.title, 120);
        let state = truncate(&state, 120);
        let mut activity = Activity::new().activity_type(ActivityType::Listening).details(&details).state(&state);
        if p.playing && p.duration_ms > 0 {
            let ts = if self.cfg.progress {
                Timestamps::new().start(start).end(start + (p.duration_ms / 1000) as i64)
            } else {
                Timestamps::new().start(start)
            };
            activity = activity.timestamps(ts);
        }
        let large_text = truncate(&format!("{} — {}", p.artist, p.title), 120);
        if let Some(art) = &p.artwork {
            activity = activity.assets(Assets::new().large_image(art).large_text(&large_text));
        }
        let button;
        if self.cfg.button {
            if let Some(url) = &p.url {
                button = vec![Button::new("Слушать на SoundCloud", url)];
                activity = activity.buttons(button);
            }
        }
        let client = self.client.as_mut().ok_or(())?;
        client.set_activity(activity).map_err(|e| {
            tracing::debug!(error = %e, "discord set_activity failed");
        })
    }
}

/// Discord limits fields to 128 bytes; cut on a char boundary.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let mut end = max - 1;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}
