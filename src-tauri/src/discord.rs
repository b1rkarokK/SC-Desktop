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

/// The room we are in, for the status.
#[derive(Debug, Clone, PartialEq)]
pub struct RoomInfo {
    pub name: String,
    pub members: usize,
    /// no password: the status may offer joining
    pub open: bool,
    pub channel: String,
}

/// A page that opens `scdesk://room/<channel>` or offers to download the app (docs/join in the repository,
/// served by GitHub Pages). The button is shown only for an open room and only if the user turned it on.
pub const JOIN_PAGE: &str = "https://b1rkarokk.github.io/SC-Desktop/join/";

enum Cmd {
    Set(Option<Presence>),
    Config(DiscordConfig),
    Section(Option<String>),
    Room(Option<RoomInfo>),
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

    /// Where in the app we are ("likes", "home" …): shown while nothing plays.
    pub fn set_section(&self, key: Option<String>) {
        let _ = self.tx.send(Cmd::Section(key));
    }

    pub fn set_room(&self, room: Option<RoomInfo>) {
        let _ = self.tx.send(Cmd::Room(room));
    }
}

struct Worker {
    cfg: DiscordConfig,
    client: Option<DiscordIpcClient>,
    last: Option<Presence>,
    /// what Discord currently shows (for de-duplication: Discord rate-limits updates)
    shown: Option<(u64, bool, i64)>,
    section: Option<String>,
    room: Option<RoomInfo>,
    /// the "not listening" status on screen
    ambient_shown: Option<String>,
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

impl Worker {
    fn new(cfg: DiscordConfig) -> Self {
        Self { cfg, client: None, last: None, shown: None, section: None, room: None, ambient_shown: None }
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
                    self.ambient_shown = None;
                    self.push();
                }
                Some(Cmd::Section(k)) => {
                    self.section = k;
                    self.push();
                }
                Some(Cmd::Room(r)) => {
                    // the track line carries the room too: show it again
                    if self.room != r {
                        self.shown = None;
                    }
                    self.room = r;
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
            self.shown = None;
            // nothing plays: say where we are, or that we are in a room
            if let (true, Some((details, state))) = (self.cfg.enabled, self.ambient()) {
                self.show_ambient(details, state);
            } else if self.ambient_shown.take().is_some() || self.client.is_some() {
                if let Some(c) = self.client.as_mut() {
                    let _ = c.clear_activity();
                }
            }
            return;
        };
        self.ambient_shown = None;

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

    /// What to say when no track is on: (details, state).
    fn ambient(&self) -> Option<(String, String)> {
        if let Some(r) = &self.room {
            let who = if r.members == 1 { crate::lang::pick("1 слушает", "1 listening").to_owned() } else { format!("{} {}", r.members, crate::lang::pick("слушают", "listening")) };
            return Some((format!("{} «{}»", crate::lang::pick("В комнате", "In the room"), r.name), who));
        }
        self.section.as_deref().and_then(section_text).map(|t| (t, "SC Desk".to_owned()))
    }

    fn show_ambient(&mut self, details: String, state: String) {
        let key = format!("{details}|{state}");
        if self.ambient_shown.as_deref() == Some(key.as_str()) {
            return;
        }
        if !self.ensure_client() {
            return;
        }
        let (d, st) = (truncate(&details, 120), truncate(&state, 120));
        let activity = Activity::new().activity_type(ActivityType::Playing).details(&d).state(&st);
        let sent = self.client.as_mut().map(|c| c.set_activity(activity).is_ok()).unwrap_or(false);
        if sent {
            self.ambient_shown = Some(key);
        } else {
            // the pipe broke: connect again next time
            self.client = None;
        }
    }

    fn send(&mut self, p: &Presence, start: i64) -> Result<(), ()> {
        let mut state = if p.playing { p.artist.clone() } else { format!("{} · {}", p.artist, crate::lang::pick("на паузе", "paused")) };
        if let Some(r) = &self.room {
            state = format!("{state} · {} «{}»", crate::lang::pick("в комнате", "in the room"), r.name);
        }
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
        // no large_text: Discord shows it as a third line, which only repeated artist + title
        if let Some(art) = &p.artwork {
            activity = activity.assets(Assets::new().large_image(art));
        }
        let button;
        if self.cfg.button {
            if let Some(url) = &p.url {
                button = vec![Button::new(crate::lang::pick("Слушать на SoundCloud", "Listen on SoundCloud"), url)];
                activity = activity.buttons(button);
            }
        }
        // an open room, with the page that opens it (or sends to the download): a join button
        let join;
        if !JOIN_PAGE.is_empty() && self.cfg.room_join {
            if let Some(r) = self.room.as_ref().filter(|r| r.open) {
                join = format!("{JOIN_PAGE}#room/{}", r.channel);
                activity = activity.buttons(vec![Button::new(crate::lang::pick("Присоединиться", "Join"), &join)]);
            }
        }
        let client = self.client.as_mut().ok_or(())?;
        client.set_activity(activity).map_err(|e| {
            tracing::debug!(error = %e, "discord set_activity failed");
        })
    }
}

/// «Листает лайки» and the like, from the section key the window reports.
fn section_text(key: &str) -> Option<String> {
    use crate::lang::pick;
    Some(match key {
        "home" => pick("На главной", "On the home page"),
        "likes" => pick("Листает лайки", "Browsing likes"),
        "wave" => pick("Выбирает волну", "Picking a wave"),
        "history" => pick("Смотрит историю", "Looking at history"),
        "stats" => pick("Смотрит итоги", "Looking at the stats"),
        "rooms" => pick("Выбирает комнату", "Choosing a room"),
        "shared" => pick("В общих плейлистах", "In shared playlists"),
        "search" => pick("Ищет музыку", "Searching for music"),
        "settings" => pick("В настройках", "In settings"),
        "profile" => pick("В профиле", "In the profile"),
        "artist" => pick("Смотрит артиста", "Looking at an artist"),
        "playlist" => pick("Смотрит плейлист", "Looking at a playlist"),
        "track" => pick("Смотрит трек", "Looking at a track"),
        "category" | "chart" | "mix" => pick("Выбирает, что послушать", "Choosing what to play"),
        _ => return None,
    }.into())
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
