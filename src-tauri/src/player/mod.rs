//! Queue + playback state. Lives in Rust so tray and global hotkeys work even
//! when the window (and its JS) is hidden.

pub mod audio;
pub mod eq;
pub mod fx;
mod widget;

use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
    time::Duration,
};

use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use self::{
    audio::{AudioCmd, AudioEvent, AudioHandle},
    eq::EqShared,
    fx::FxShared,
};
use crate::{
    config::{DiscordConfig, EqConfig, FxConfig},
    discord::{Discord, Presence},
    error::{AppError, AppResult},
    models::TrackDto,
    state::AppState,
    tray, wave,
};

/// SoundCloud allows 15 000 play-stream resolutions per 24 h; stay below it.
const STREAM_QUOTA: u32 = 14_500;
/// Consecutive unplayable tracks to skip automatically before stopping.
const MAX_AUTO_SKIPS: u32 = 5;
/// "Previous" restarts the track if we're further in than this.
const PREV_RESTART_MS: u64 = 3_000;
/// Start fetching the next wave batch when this few tracks are left.
const WAVE_PREFETCH_LEFT: usize = 3;
const WAVE_BATCH: usize = 25;
/// Smart Shuffle: a recommendation after every this many list tracks, at most this many.
const SMART_EVERY: usize = 3;
const SMART_MAX: usize = 40;
/// Moving on before this (or before this % of the track) counts as a skip.
const SKIP_EARLY_MS: u64 = 30_000;
const SKIP_EARLY_PCT: u64 = 35;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepeatMode {
    Off,
    All,
    One,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QueueSource {
    List,
    Likes,
    Wave,
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub track: Option<TrackDto>,
    pub playing: bool,
    pub loading: bool,
    pub position_ms: u64,
    pub volume: f32,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub source: QueueSource,
    pub queue_len: usize,
    pub queue_pos: usize,
    /// Smart Shuffle: recommendations are mixed into the list
    pub smart: bool,
    /// the current track is such a recommendation
    pub recommended: bool,
}

struct State {
    queue: Vec<TrackDto>,
    /// Play order (indices into `queue`); identity unless shuffled.
    order: Vec<usize>,
    pos: usize,
    active: bool,
    playing: bool,
    loading: bool,
    shuffle: bool,
    repeat: RepeatMode,
    volume: f32,
    source: QueueSource,
    failures: u32,
    wave_fetching: bool,
    smart: bool,
    /// tracks Smart Shuffle put into the queue
    smart_ids: HashSet<u64>,
    /// bumped on every new list: a late Smart Shuffle fill for an old list is dropped
    list_gen: u64,
}

impl State {
    /// Wave tracks and Smart Shuffle recommendations teach the wave.
    fn learns_from_current(&self) -> bool {
        self.source == QueueSource::Wave || self.current().is_some_and(|t| self.smart_ids.contains(&t.id))
    }

    /// Takes upcoming tracks matching `drop` out of the play order.
    fn drop_upcoming(&mut self, drop: impl Fn(&TrackDto) -> bool) {
        let pos = self.pos;
        let keep: Vec<bool> = self.queue.iter().map(|t| !drop(t)).collect();
        let old_order = std::mem::take(&mut self.order);
        // played part + current stay untouched
        let mut new_order = old_order[..=pos.min(old_order.len().saturating_sub(1))].to_vec();
        new_order.extend(old_order.iter().skip(pos + 1).copied().filter(|&i| keep[i]));
        self.order = new_order;
    }

    fn current(&self) -> Option<&TrackDto> {
        if !self.active {
            return None;
        }
        self.order.get(self.pos).and_then(|&i| self.queue.get(i))
    }

    fn rebuild_order(&mut self, keep_index: Option<usize>) {
        let n = self.queue.len();
        let mut order: Vec<usize> = (0..n).collect();
        match (self.shuffle, keep_index) {
            (true, Some(cur)) => {
                order.retain(|&i| i != cur);
                order.shuffle(&mut rand::thread_rng());
                order.insert(0, cur);
                self.pos = 0;
            }
            (true, None) => {
                order.shuffle(&mut rand::thread_rng());
                self.pos = 0;
            }
            (false, Some(cur)) => self.pos = cur,
            (false, None) => self.pos = 0,
        }
        self.order = order;
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct QueueItem {
    /// place in the play order
    pub at: usize,
    pub track: TrackDto,
    /// added by Smart Shuffle
    pub recommended: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueueView {
    /// place of the current track (None: nothing playing yet)
    pub current: Option<usize>,
    pub items: Vec<QueueItem>,
    /// how many follow the current one in total
    pub upcoming: usize,
    pub source: QueueSource,
}

#[derive(Default)]
struct SleepTimer {
    until: Option<std::time::Instant>,
    end_of_track: bool,
    /// bumped on every change: an older timer task finds it stale
    id: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SleepState {
    pub remaining_s: Option<u64>,
    pub end_of_track: bool,
}

const DRM_ONLY_KEY: &str = "drm_only";
/// a public DRM-only track (gazz / меня плавит), for the warm-up when none is known yet
const WARM_SAMPLE: u64 = 1_505_419_138;

enum Fetched {
    /// decoded natively
    Bytes(Vec<u8>),
    /// DRM-only: SoundCloud's widget plays it
    /// the track id the widget plays (a full upload may stand in for a Go+ preview)
    Widget(u64),
}

pub struct Player {
    app: AppHandle,
    audio: AudioHandle,
    /// SoundCloud's own player for DRM-only tracks (see widget.rs)
    widget: widget::Widget,
    /// the current track plays in the widget, not natively
    external: std::sync::atomic::AtomicBool,
    /// tracks known to be DRM-only: go straight to the widget (kv "drm_only")
    drm_only: Mutex<HashSet<u64>>,
    /// a Go+ preview played through someone else's upload: (track id, seconds the
    /// recording is shifted by in that upload). The lyrics shift with it.
    stand_in: Mutex<Option<(u64, f32)>>,
    /// the next track's audio, fetched while the current one plays (one slot)
    prefetched: Mutex<Option<(u64, Arc<Vec<u8>>, Option<f32>)>>,
    eq: Arc<EqShared>,
    fx: Arc<FxShared>,
    /// loudness gain per track, measured once
    gains: Mutex<std::collections::HashMap<u64, f32>>,
    /// the next load crossfades from the playing track
    crossfade_next: std::sync::atomic::AtomicBool,
    crossfade: Mutex<f32>,
    /// sleep timer: (deadline, after this track, id)
    sleep: Mutex<SleepTimer>,
    discord: Discord,
    st: Mutex<State>,
    /// Incremented on every load; stale downloads/events are dropped.
    generation: AtomicU64,
}

impl Player {
    pub fn start(app: AppHandle) -> AppResult<Arc<Self>> {
        let cfg = app.state::<AppState>().config.get();
        let eq = EqShared::new(cfg.eq.clone());
        let fx = FxShared::new(cfg.fx.reverb, cfg.fx.speed, cfg.fx.normalize);
        let (audio, mut events) = audio::spawn(eq.clone(), fx.clone())?;
        audio.send(AudioCmd::Crossfade(Duration::from_secs_f32(cfg.fx.crossfade.clamp(0.0, 12.0))));
        let volume = cfg.volume.clamp(0.0, 1.0);
        audio.send(AudioCmd::Volume(volume));

        let widget = widget::Widget::new(app.clone(), audio.events(), volume);
        let player = Arc::new(Self {
            app,
            audio,
            widget,
            external: std::sync::atomic::AtomicBool::new(false),
            drm_only: Mutex::new(HashSet::new()),
            stand_in: Mutex::new(None),
            prefetched: Mutex::new(None),
            eq,
            fx,
            gains: Mutex::new(std::collections::HashMap::new()),
            crossfade_next: std::sync::atomic::AtomicBool::new(false),
            crossfade: Mutex::new(cfg.fx.crossfade.clamp(0.0, 12.0)),
            sleep: Mutex::new(SleepTimer::default()),
            discord: Discord::start(cfg.discord.clone()),
            st: Mutex::new(State {
                queue: Vec::new(),
                order: Vec::new(),
                pos: 0,
                active: false,
                playing: false,
                loading: false,
                shuffle: false,
                repeat: RepeatMode::Off,
                volume,
                source: QueueSource::List,
                failures: 0,
                wave_fetching: false,
                smart: false,
                smart_ids: HashSet::new(),
                list_gen: 0,
            }),
            generation: AtomicU64::new(0),
        });

        let p = player.clone();
        tauri::async_runtime::spawn(async move {
            let known = p.app.state::<AppState>().db.kv_get::<Vec<u64>>(DRM_ONLY_KEY).await;
            if let Ok(Some((ids, _))) = known {
                p.drm_only.lock().unwrap_or_else(|e| e.into_inner()).extend(ids);
            }
            // once the app has settled, warm SoundCloud's player up for protected tracks
            tokio::time::sleep(std::time::Duration::from_secs(20)).await;
            let state = p.app.state::<AppState>();
            if state.credentials().is_some() && state.config.get().fast_protected {
                let sample = p.drm_only.lock().unwrap_or_else(|e| e.into_inner()).iter().next().copied().unwrap_or(WARM_SAMPLE);
                p.widget.warm_up(sample);
            }
        });
        let p = player.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(ev) = events.recv().await {
                p.on_audio_event(ev);
            }
        });
        Ok(player)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn snapshot(&self) -> Snapshot {
        let s = self.lock();
        Snapshot {
            track: s.current().cloned(),
            playing: s.playing,
            loading: s.loading,
            position_ms: self.position_ms(),
            volume: s.volume,
            shuffle: s.shuffle,
            repeat: s.repeat,
            source: s.source,
            queue_len: s.queue.len(),
            queue_pos: s.pos,
            smart: s.smart,
            recommended: s.current().is_some_and(|t| s.smart_ids.contains(&t.id)),
        }
    }

    /// «Мгновенный старт защищённых треков» switched in Settings.
    pub fn set_fast_protected(&self, on: bool) {
        if on {
            let sample = self.drm_only.lock().unwrap_or_else(|e| e.into_inner()).iter().next().copied().unwrap_or(WARM_SAMPLE);
            self.widget.warm_up(sample);
        } else {
            self.widget.cool_down();
        }
    }

    pub fn position_ms(&self) -> u64 {
        if self.external.load(Ordering::SeqCst) {
            self.widget.position_ms()
        } else {
            self.audio.position_ms()
        }
    }

    /// Sends a command to whichever engine plays the current track.
    fn out(&self, cmd: AudioCmd) {
        match cmd {
            AudioCmd::Stop => {
                self.external.store(false, Ordering::SeqCst);
                self.widget.stop();
                self.audio.send(AudioCmd::Stop);
            }
            AudioCmd::Volume(v) => {
                self.widget.send(AudioCmd::Volume(v));
                self.audio.send(AudioCmd::Volume(v));
            }
            AudioCmd::Load { .. } => {
                self.external.store(false, Ordering::SeqCst);
                self.widget.stop();
                self.audio.send(cmd);
            }
            cmd if self.external.load(Ordering::SeqCst) => self.widget.send(cmd),
            cmd => self.audio.send(cmd),
        }
    }

    pub fn current(&self) -> Option<TrackDto> {
        self.lock().current().cloned()
    }

    /// The queue window: the current track and what follows, in play order.
    /// `at` of an item is its place in the order (stable until the queue changes).
    pub fn queue_view(&self, limit: usize) -> QueueView {
        let s = self.lock();
        let start = if s.active { s.pos } else { s.pos.min(s.order.len()) };
        let items = s
            .order
            .iter()
            .enumerate()
            .skip(start)
            .take(limit)
            .filter_map(|(at, &i)| {
                s.queue.get(i).map(|t| QueueItem { at, track: t.clone(), recommended: s.smart_ids.contains(&t.id) })
            })
            .collect();
        QueueView {
            current: if s.active { Some(s.pos) } else { None },
            items,
            upcoming: s.order.len().saturating_sub(s.pos + 1),
            source: s.source,
        }
    }

    /// Drag and drop: the upcoming track at `from` goes to `to` (both order places
    /// after the current one).
    pub fn queue_move(self: &Arc<Self>, from: usize, to: usize) {
        {
            let mut s = self.lock();
            let first = s.pos + 1;
            let len = s.order.len();
            if from < first || to < first || from >= len || to >= len || from == to {
                return;
            }
            let item = s.order.remove(from);
            s.order.insert(to, item);
        }
        self.after_queue_edit();
    }

    pub fn queue_remove(self: &Arc<Self>, at: usize) {
        {
            let mut s = self.lock();
            if at <= s.pos || at >= s.order.len() {
                return;
            }
            s.order.remove(at);
        }
        self.after_queue_edit();
    }

    /// Everything after the current track goes.
    pub fn queue_clear(self: &Arc<Self>) {
        {
            let mut s = self.lock();
            let keep = (s.pos + 1).min(s.order.len());
            s.order.truncate(keep);
        }
        self.after_queue_edit();
    }

    /// Click in the queue: play that one now (the ones before it are skipped).
    pub fn queue_play(self: &Arc<Self>, at: usize) {
        {
            let mut s = self.lock();
            if at >= s.order.len() {
                return;
            }
            s.pos = at;
            s.active = true;
            s.failures = 0;
        }
        self.load_current();
        self.maybe_prefetch_wave();
    }

    /// The next track may have changed: drop a stale prefetch, look ahead again.
    fn after_queue_edit(self: &Arc<Self>) {
        let next = self.upcoming(1).first().map(|t| t.id);
        {
            let mut slot = self.prefetched.lock().unwrap_or_else(|e| e.into_inner());
            if slot.as_ref().is_some_and(|(id, ..)| Some(*id) != next) {
                *slot = None;
            }
        }
        self.emit();
        let _ = self.app.emit("player:queue", ());
        if self.lock().playing {
            self.lookahead();
        }
        self.maybe_prefetch_wave();
    }

    /// Next `n` tracks in play order (for "Далее в волне").
    pub fn upcoming(&self, n: usize) -> Vec<TrackDto> {
        let s = self.lock();
        s.order.iter().skip(s.pos + 1).take(n).filter_map(|&i| s.queue.get(i).cloned()).collect()
    }

    fn emit(&self) {
        let snap = self.snapshot();
        tray::set_now_playing(&self.app, snap.track.as_ref(), snap.playing);
        self.push_presence(&snap, None);
        if let Err(e) = self.app.emit("player:state", &snap) {
            tracing::warn!(error = %e, "emit player:state");
        }
    }

    fn push_presence(&self, snap: &Snapshot, position_ms: Option<u64>) {
        let presence = snap.track.as_ref().filter(|_| !snap.loading).map(|t| Presence {
            track_id: t.id,
            title: t.title.clone(),
            artist: t.artist.clone(),
            artwork: t.artwork_url.as_deref().map(|a| a.replace("-large.", "-t500x500.")),
            url: t.permalink_url.clone(),
            position_ms: position_ms.unwrap_or(snap.position_ms),
            duration_ms: t.duration_ms,
            playing: snap.playing,
        });
        self.discord.update(presence);
    }

    pub fn set_eq(&self, cfg: EqConfig) {
        self.eq.set(cfg);
    }

    pub fn set_fx(&self, cfg: &FxConfig) {
        self.fx.set(cfg.reverb, cfg.speed, cfg.normalize);
        let cross = cfg.crossfade.clamp(0.0, 12.0);
        *self.crossfade.lock().unwrap_or_else(|e| e.into_inner()) = cross;
        self.audio.send(AudioCmd::Crossfade(Duration::from_secs_f32(cross)));
    }

    /// Loudness gain of a track: measured once (a few tens of ms), then cached.
    async fn gain_for(&self, track_id: u64, data: &Arc<Vec<u8>>) -> f32 {
        if !self.fx.normalize() {
            return 1.0;
        }
        if let Some(g) = self.gains.lock().unwrap_or_else(|e| e.into_inner()).get(&track_id) {
            return *g;
        }
        let bytes = data.clone();
        let g = tauri::async_runtime::spawn_blocking(move || fx::measure_gain(&bytes)).await.unwrap_or(1.0);
        let mut gains = self.gains.lock().unwrap_or_else(|e| e.into_inner());
        if gains.len() > 2000 {
            gains.clear();
        }
        gains.insert(track_id, g);
        tracing::debug!(track = track_id, gain = g, "loudness measured");
        g
    }

    // ------------------------------------------------------- sleep timer

    /// `minutes`: pause that much later (fading out); `end_of_track`: pause
    /// when the current track ends. Both off = no timer.
    pub fn set_sleep(self: &Arc<Self>, minutes: Option<u32>, end_of_track: bool) {
        let id = {
            let mut s = self.sleep.lock().unwrap_or_else(|e| e.into_inner());
            s.id += 1;
            s.until = minutes.filter(|m| *m > 0).map(|m| std::time::Instant::now() + Duration::from_secs(m as u64 * 60));
            s.end_of_track = end_of_track;
            s.id
        };
        let _ = self.app.emit("player:sleep", self.sleep_state());
        let Some(m) = minutes.filter(|m| *m > 0) else { return };
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(m as u64 * 60)).await;
            if this.sleep.lock().unwrap_or_else(|e| e.into_inner()).id == id {
                this.fade_to_sleep().await;
            }
        });
    }

    pub fn sleep_state(&self) -> SleepState {
        let s = self.sleep.lock().unwrap_or_else(|e| e.into_inner());
        SleepState {
            remaining_s: s.until.map(|u| u.saturating_duration_since(std::time::Instant::now()).as_secs()),
            end_of_track: s.end_of_track,
        }
    }

    /// Volume down over 8 s, pause, volume back for next time.
    async fn fade_to_sleep(self: &Arc<Self>) {
        let volume = self.lock().volume;
        for i in (0..=20).rev() {
            self.out(AudioCmd::Volume(volume * i as f32 / 20.0));
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
        self.pause();
        self.out(AudioCmd::Volume(volume));
        self.clear_sleep();
        tracing::info!("sleep timer: paused");
    }

    fn clear_sleep(&self) {
        {
            let mut s = self.sleep.lock().unwrap_or_else(|e| e.into_inner());
            s.id += 1;
            s.until = None;
            s.end_of_track = false;
        }
        let _ = self.app.emit("player:sleep", self.sleep_state());
    }

    /// The track ended and the timer says "after this track".
    fn sleep_after_track(&self) -> bool {
        let due = self.sleep.lock().unwrap_or_else(|e| e.into_inner()).end_of_track;
        if due {
            self.clear_sleep();
        }
        due
    }

    pub fn set_discord(&self, cfg: DiscordConfig) {
        self.discord.configure(cfg);
        let snap = self.snapshot();
        self.push_presence(&snap, None);
    }

    fn emit_error(&self, err: &AppError) {
        let _ = self.app.emit("player:error", err);
    }

    // ------------------------------------------------------------ queue

    pub fn play_queue(self: &Arc<Self>, tracks: Vec<TrackDto>, start: usize, source: QueueSource) {
        let smart = {
            let mut s = self.lock();
            if tracks.is_empty() {
                return;
            }
            let start = start.min(tracks.len() - 1);
            s.queue = tracks;
            s.source = source;
            s.active = true;
            s.failures = 0;
            s.list_gen += 1;
            s.smart_ids.clear();
            if source == QueueSource::Wave {
                s.smart = false;
            }
            s.rebuild_order(Some(start));
            s.smart
        };
        self.load_current();
        if smart {
            self.smart_fill();
        }
    }

    // ---------------------------------------------------- smart shuffle

    /// Smart Shuffle (as in Spotify): the list plays shuffled and every few
    /// tracks a recommendation built around the list itself is mixed in.
    /// Recommendations that are skipped or listened to teach the wave too.
    pub fn set_smart_shuffle(self: &Arc<Self>, on: bool) {
        let fill = {
            let mut s = self.lock();
            if s.smart == on {
                return;
            }
            s.smart = on;
            if on {
                if !s.shuffle {
                    s.shuffle = true;
                    let cur = s.order.get(s.pos).copied();
                    s.rebuild_order(cur);
                }
            } else {
                let ids = std::mem::take(&mut s.smart_ids);
                s.drop_upcoming(|t| ids.contains(&t.id));
            }
            on && s.active && s.source != QueueSource::Wave
        };
        self.emit();
        if fill {
            self.smart_fill();
        }
    }

    fn smart_fill(self: &Arc<Self>) {
        let (gen, list): (u64, Vec<u64>) = {
            let s = self.lock();
            (s.list_gen, s.queue.iter().filter(|t| !s.smart_ids.contains(&t.id)).map(|t| t.id).collect())
        };
        if list.is_empty() {
            return;
        }
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            let result = this.smart_recommend(list).await;
            let mut s = this.lock();
            if s.list_gen != gen || !s.smart {
                return;
            }
            match result {
                Ok(recs) => {
                    // one recommendation after every SMART_EVERY upcoming tracks
                    let mut at = s.pos + 1 + SMART_EVERY;
                    let mut n = 0;
                    for t in recs {
                        if at > s.order.len() {
                            break;
                        }
                        let idx = s.queue.len();
                        s.smart_ids.insert(t.id);
                        s.queue.push(t);
                        s.order.insert(at, idx);
                        at += SMART_EVERY + 1;
                        n += 1;
                    }
                    tracing::info!(n, "smart shuffle: recommendations mixed in");
                    drop(s);
                    this.emit();
                }
                Err(e) => {
                    drop(s);
                    tracing::warn!(error = %e, "smart shuffle failed");
                    this.emit_error(&e);
                }
            }
        });
    }

    async fn smart_recommend(&self, list: Vec<u64>) -> AppResult<Vec<TrackDto>> {
        let state = self.app.state::<AppState>();
        // the list's own taste: a sample of its tracks with full metadata
        let mut sample = list.clone();
        sample.shuffle(&mut rand::thread_rng());
        sample.truncate(200);
        let mut tracks = state.db.tracks_by_ids(sample.clone()).await?;
        if tracks.len() < sample.len() {
            let have: HashSet<u64> = tracks.iter().map(|t| t.id).collect();
            let missing: Vec<u64> = sample.iter().copied().filter(|id| !have.contains(id)).take(100).collect();
            if let Ok(more) = state.sc()?.tracks_by_ids(&missing).await {
                state.db.upsert_tracks(more.clone()).await?;
                tracks.extend(more);
            }
        }
        let want = (list.len() / SMART_EVERY).clamp(1, SMART_MAX);
        let ctx = Arc::new(wave::WaveContext { kind: "shuffle", title: String::new(), tracks });
        let mut exclude: HashSet<u64> = list.into_iter().collect();
        let mut out: Vec<TrackDto> = Vec::new();
        // one batch gives up to ~25: ask again for long lists
        for _ in 0..3 {
            if out.len() >= want {
                break;
            }
            let gen = wave::Gen { context: Some(ctx.clone()), familiar: false, mood: Some("normal") };
            let batch = wave::generate_with(&state, want - out.len(), &exclude, gen).await?;
            exclude.extend(batch.iter().map(|t| t.id));
            if batch.is_empty() {
                break;
            }
            out.extend(batch);
        }
        Ok(out)
    }

    // ------------------------------------------------- wave feedback

    /// The user moved on from the current track: a skip in its first seconds
    /// tells the wave this one missed.
    fn feedback_on_leave(self: &Arc<Self>) {
        let (track, learns) = {
            let s = self.lock();
            (s.current().cloned(), s.learns_from_current())
        };
        let Some(track) = track.filter(|_| learns) else { return };
        let pos = self.position_ms();
        let early = pos < SKIP_EARLY_MS || (track.duration_ms > 0 && pos * 100 < track.duration_ms * SKIP_EARLY_PCT);
        if early {
            self.feedback(track, wave::SKIPPED);
        }
    }

    fn feedback(self: &Arc<Self>, track: TrackDto, signal: i8) {
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            let state = this.app.state::<AppState>();
            if signal == wave::SKIPPED {
                // a liked song skipped means "not now", not "I don't like it"
                if state.db.likes_ids().await.is_ok_and(|l| l.contains(&track.id)) {
                    return;
                }
            }
            // an artist with several of the user's likes is never pushed out by skips
            let fan = state.db.liked_by_artist(track.user_id).await.unwrap_or(0) >= wave::FAN_LIKES;
            if !(fan && signal == wave::SKIPPED) {
                let skips = state.wave.note(track.id, track.user_id, signal);
                if skips >= 2 {
                    // skipped this artist twice now: the rest of the wave can do without
                    let mut s = this.lock();
                    if s.source == QueueSource::Wave {
                        s.drop_upcoming(|t| t.user_id == track.user_id);
                    } else {
                        let ids = s.smart_ids.clone();
                        s.drop_upcoming(|t| t.user_id == track.user_id && ids.contains(&t.id));
                    }
                }
            }
            if let Err(e) = state.db.wave_signal_put(track.id, signal).await {
                tracing::debug!(error = %e, "wave signal");
            }
        });
    }

    pub fn append(&self, tracks: Vec<TrackDto>) {
        let mut s = self.lock();
        let first_new = s.queue.len();
        s.queue.extend(tracks);
        let mut new_idx: Vec<usize> = (first_new..s.queue.len()).collect();
        if s.shuffle {
            new_idx.shuffle(&mut rand::thread_rng());
        }
        s.order.extend(new_idx);
    }

    /// "Играть следующим" (next = true) / "В конец очереди". With an empty
    /// queue the track simply starts playing.
    pub fn enqueue(self: &Arc<Self>, track: TrackDto, next: bool) {
        {
            let mut s = self.lock();
            if s.active && !s.queue.is_empty() {
                let idx = s.queue.len();
                s.queue.push(track);
                let at = if next { (s.pos + 1).min(s.order.len()) } else { s.order.len() };
                s.order.insert(at, idx);
                drop(s);
                self.emit();
                return;
            }
        }
        self.play_queue(vec![track], 0, QueueSource::List);
    }

    // --------------------------------------------------------- controls

    pub fn toggle(self: &Arc<Self>) {
        let (active, playing, loading) = {
            let s = self.lock();
            (s.active, s.playing, s.loading)
        };
        if loading {
            return;
        }
        if !active {
            let has_queue = !self.lock().queue.is_empty();
            if has_queue {
                self.lock().active = true;
                self.load_current();
            } else {
                // nothing queued yet (fresh start): ▶ plays the likes from the top
                let this = self.clone();
                tauri::async_runtime::spawn(async move {
                    let state = this.app.state::<AppState>();
                    match state.db.likes_all().await {
                        Ok(tracks) if !tracks.is_empty() => this.play_queue(tracks, 0, QueueSource::Likes),
                        Ok(_) => this.emit_error(&AppError::Other("Лайков пока нет — откройте «Лайки» и синхронизируйте".into())),
                        Err(e) => this.emit_error(&e),
                    }
                });
            }
            return;
        }
        if playing {
            self.pause();
        } else {
            self.resume();
        }
    }

    pub fn resume(&self) {
        let mut s = self.lock();
        if s.active && !s.loading {
            self.out(AudioCmd::Play);
            s.playing = true;
            drop(s);
            self.emit();
        }
    }

    pub fn pause(&self) {
        let mut s = self.lock();
        if s.playing {
            self.out(AudioCmd::Pause);
            s.playing = false;
            drop(s);
            self.emit();
        }
    }

    pub fn seek(&self, ms: u64) {
        if self.lock().active {
            self.out(AudioCmd::Seek(Duration::from_millis(ms)));
            let snap = self.snapshot();
            self.push_presence(&snap, Some(ms));
            let _ = self.app.emit("player:seek", ms);
        }
    }

    pub fn set_volume(&self, v: f32) {
        let v = v.clamp(0.0, 1.0);
        self.lock().volume = v;
        self.out(AudioCmd::Volume(v));
    }

    pub fn set_shuffle(&self, on: bool) {
        {
            let mut s = self.lock();
            if s.shuffle == on {
                return;
            }
            s.shuffle = on;
            if !on && s.smart {
                s.smart = false;
                let ids = std::mem::take(&mut s.smart_ids);
                s.drop_upcoming(|t| ids.contains(&t.id));
            }
            let cur = s.order.get(s.pos).copied();
            s.rebuild_order(cur);
        }
        self.emit();
    }

    pub fn set_repeat(&self, mode: RepeatMode) {
        self.lock().repeat = mode;
        self.emit();
    }

    pub fn next(self: &Arc<Self>) {
        self.feedback_on_leave();
        self.advance(false);
    }

    pub fn prev(self: &Arc<Self>) {
        if self.position_ms() > PREV_RESTART_MS {
            self.seek(0);
            return;
        }
        {
            let mut s = self.lock();
            if !s.active || s.order.is_empty() {
                return;
            }
            if s.pos > 0 {
                s.pos -= 1;
            } else if s.repeat == RepeatMode::All {
                s.pos = s.order.len() - 1;
            } else {
                drop(s);
                self.seek(0);
                return;
            }
            s.failures = 0;
        }
        self.load_current();
    }

    /// `auto`: called because the track ended / failed, not by the user.
    fn advance(self: &Arc<Self>, auto: bool) {
        enum Next {
            Load,
            ExtendWave,
            Stop,
            Nothing,
        }
        let next = {
            let mut s = self.lock();
            if s.order.is_empty() {
                Next::Nothing
            } else if s.pos + 1 < s.order.len() {
                s.pos += 1;
                s.active = true;
                Next::Load
            } else if s.source == QueueSource::Wave {
                Next::ExtendWave
            } else if s.repeat == RepeatMode::All {
                if s.shuffle {
                    let cur = s.order[s.pos];
                    s.order.shuffle(&mut rand::thread_rng());
                    // don't repeat the same track back-to-back
                    if s.order.len() > 1 && s.order[0] == cur {
                        let last = s.order.len() - 1;
                        s.order.swap(0, last);
                    }
                }
                s.pos = 0;
                Next::Load
            } else if auto {
                s.playing = false;
                Next::Stop
            } else {
                Next::Nothing
            }
        };
        match next {
            Next::Load => {
                self.load_current();
                self.maybe_prefetch_wave();
            }
            Next::ExtendWave => self.extend_wave(true),
            Next::Stop => {
                self.out(AudioCmd::Stop);
                self.emit();
            }
            Next::Nothing => {}
        }
    }

    // ------------------------------------------------------------- wave

    pub async fn start_wave(self: &Arc<Self>) -> AppResult<usize> {
        let state = self.app.state::<AppState>();
        state.wave.set_context(None);
        let batch = wave::generate(&state, WAVE_BATCH, &Default::default()).await?;
        let n = batch.len();
        if n == 0 {
            return Err(AppError::Other("Волна пуста: не найдено подходящих треков".into()));
        }
        {
            // the wave is its own order; shuffle would only fight the weighting
            self.lock().shuffle = false;
        }
        self.play_queue(batch, 0, QueueSource::Wave);
        Ok(n)
    }

    /// "Волна по треку / по артисту": seed track (or the artist's popular
    /// tracks) first, then related; further batches stay around that track /
    /// artist (wave context).
    pub async fn start_wave_seeded(self: &Arc<Self>, track_id: Option<u64>, artist_id: Option<u64>) -> AppResult<usize> {
        use crate::api::soundcloud::UserSection;
        use rand::seq::SliceRandom;
        let state = self.app.state::<AppState>();
        let sc = state.sc()?;
        let disliked_tracks = state.db.disliked_track_ids().await?;
        let disliked_artists: HashSet<u64> =
            state.db.disliked_artists().await?.into_iter().map(|a| a.user_id).collect();

        let mut tracks = Vec::new();
        let mut context = None;
        if let Some(id) = track_id {
            let seed = sc.track(id).await?;
            context = Some(wave::WaveContext { kind: "track", title: seed.title.clone(), tracks: vec![seed.clone()] });
            tracks.push(seed);
            tracks.extend(sc.related(id).await?);
        } else if let Some(uid) = artist_id {
            if let UserSection::Tracks(mut top) = sc.user_section(uid, "popular").await? {
                if let Some(first) = top.first() {
                    context = Some(wave::WaveContext { kind: "artist", title: first.artist_name(), tracks: top.clone() });
                }
                top.shuffle(&mut rand::thread_rng());
                let related = match top.first() {
                    Some(t) => sc.related(t.id).await.unwrap_or_default(),
                    None => Vec::new(),
                };
                // alternate artist / similar so it doesn't become a one-artist playlist
                let mut rel = related.into_iter();
                for t in top.into_iter().take(12) {
                    tracks.push(t);
                    if let Some(r) = rel.next() {
                        tracks.push(r);
                    }
                }
                tracks.extend(rel);
            }
        }
        let liked: HashSet<u64> = if state.wave.no_liked() { state.db.likes_ids().await? } else { HashSet::new() };
        let mut seen = HashSet::new();
        tracks.retain(|t| {
            let is_seed = Some(t.id) == track_id;
            seen.insert(t.id)
                && t.is_fully_playable()
                && !disliked_tracks.contains(&t.id)
                && (is_seed || !disliked_artists.contains(&t.artist_id()))
                && (is_seed || !liked.contains(&t.id))
        });
        if tracks.is_empty() {
            return Err(AppError::Other("Не нашлось треков для волны".into()));
        }
        state.db.upsert_tracks(tracks.clone()).await?;
        state.wave.set_context(context);
        let dtos: Vec<TrackDto> = tracks.iter().map(TrackDto::from).collect();
        let n = dtos.len();
        self.lock().shuffle = false;
        self.play_queue(dtos, 0, QueueSource::Wave);
        Ok(n)
    }

    /// "Волна по плейлисту / по альбому": endless recommendations around the
    /// playlist, its own tracks mixed in now and then.
    pub async fn start_wave_playlist(self: &Arc<Self>, playlist_id: u64) -> AppResult<usize> {
        let state = self.app.state::<AppState>();
        let (pl, tracks) = state.sc()?.playlist(playlist_id).await?;
        if tracks.is_empty() {
            return Err(AppError::Other("В плейлисте нет треков".into()));
        }
        state.db.upsert_tracks(tracks.clone()).await?;
        let kind = if pl.is_album.unwrap_or(false) { "album" } else { "playlist" };
        state.wave.set_context(Some(wave::WaveContext { kind, title: pl.title.clone(), tracks }));
        let batch = wave::generate(&state, WAVE_BATCH, &Default::default()).await?;
        let n = batch.len();
        if n == 0 {
            return Err(AppError::Other("Не нашлось треков для волны".into()));
        }
        self.lock().shuffle = false;
        self.play_queue(batch, 0, QueueSource::Wave);
        Ok(n)
    }

    /// Restart the wave with a new mood (keeps the current track playing).
    pub async fn restart_wave(self: &Arc<Self>) -> AppResult<usize> {
        let (is_wave, current) = {
            let s = self.lock();
            (s.source == QueueSource::Wave, s.current().cloned())
        };
        if !is_wave {
            return self.start_wave().await;
        }
        let exclude: HashSet<u64> = current.iter().map(|t| t.id).collect();
        let state = self.app.state::<AppState>();
        let batch = wave::generate(&state, WAVE_BATCH, &exclude).await?;
        let n = batch.len();
        let mut s = self.lock();
        let cur_idx = s.order.get(s.pos).copied();
        let keep: Vec<TrackDto> = cur_idx.and_then(|i| s.queue.get(i).cloned()).into_iter().collect();
        s.queue = keep.into_iter().chain(batch).collect();
        s.order = (0..s.queue.len()).collect();
        s.pos = 0;
        drop(s);
        self.emit();
        Ok(n)
    }

    fn maybe_prefetch_wave(self: &Arc<Self>) {
        let need = {
            let s = self.lock();
            s.source == QueueSource::Wave && !s.wave_fetching && s.order.len().saturating_sub(s.pos + 1) < WAVE_PREFETCH_LEFT
        };
        if need {
            self.extend_wave(false);
        }
    }

    fn extend_wave(self: &Arc<Self>, then_advance: bool) {
        let exclude: HashSet<u64> = {
            let mut s = self.lock();
            if s.wave_fetching {
                return;
            }
            s.wave_fetching = true;
            s.queue.iter().map(|t| t.id).collect()
        };
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            let result = {
                let state = this.app.state::<AppState>();
                wave::generate(&state, WAVE_BATCH, &exclude).await
            };
            this.lock().wave_fetching = false;
            match result {
                Ok(batch) if !batch.is_empty() => {
                    tracing::info!(n = batch.len(), "wave extended");
                    this.append(batch);
                    if then_advance {
                        this.advance(true);
                    } else {
                        this.emit();
                    }
                }
                Ok(_) => tracing::warn!("wave produced no new tracks"),
                Err(e) => {
                    tracing::warn!(error = %e, "wave extension failed");
                    this.emit_error(&e);
                }
            }
        });
    }

    /// Wave: drop upcoming tracks of an artist the user just disliked.
    pub fn purge_artist(&self, user_id: u64) {
        self.lock().drop_upcoming(|t| t.user_id == user_id);
    }

    // ----------------------------------------------------------- loading

    fn load_current(self: &Arc<Self>) {
        let track = {
            let mut s = self.lock();
            let Some(t) = s.current().cloned() else { return };
            s.loading = true;
            s.playing = false;
            t
        };
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        // a crossfade keeps the playing track going under the new one
        let crossfade = self.crossfade_next.swap(false, Ordering::SeqCst);
        if !crossfade {
            self.out(AudioCmd::Stop);
        }
        self.emit();
        let _ = self.app.emit("player:track", &track);

        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            // boxed: the download future is large, and debug builds keep it on the worker's stack
            let t0 = std::time::Instant::now();
            // already fetched while the previous track played: start at once
            let ready = {
                let mut slot = this.prefetched.lock().unwrap_or_else(|e| e.into_inner());
                match slot.take() {
                    Some((id, data, stand_in)) if id == track.id => Some((data, stand_in)),
                    other => {
                        *slot = other;
                        None
                    }
                }
            };
            let result = match ready {
                Some((data, stand_in)) => {
                    if let Some(shift) = stand_in {
                        *this.stand_in.lock().unwrap_or_else(|e| e.into_inner()) = Some((track.id, shift));
                    }
                    tracing::debug!(track = track.id, "prefetched audio used");
                    Ok(Fetched::Bytes(Arc::try_unwrap(data).unwrap_or_else(|a| (*a).clone())))
                }
                None => Box::pin(this.fetch_audio(track.id)).await,
            };
            tracing::debug!(ms = t0.elapsed().as_millis() as u64, ok = result.is_ok(), "track fetched");
            if this.generation.load(Ordering::SeqCst) != generation {
                return; // user already moved on
            }
            match result {
                Ok(fetched) => {
                    match fetched {
                        Fetched::Bytes(data) => {
                            let data = Arc::new(data);
                            let gain = this.gain_for(track.id, &data).await;
                            if this.generation.load(Ordering::SeqCst) != generation {
                                return;
                            }
                            let fade = crossfade
                                .then(|| *this.crossfade.lock().unwrap_or_else(|e| e.into_inner()))
                                .filter(|s| *s > 0.0)
                                .map(Duration::from_secs_f32);
                            this.out(AudioCmd::Load { data, generation, gain, duration_ms: track.duration_ms, fade });
                            // a stand-in for a Go+ preview may be another edit (longer intro …):
                            // the lyrics must follow the recording that really plays
                            let stand_in = this.stand_in.lock().unwrap_or_else(|e| e.into_inner()).take();
                            if let Some((_, shift)) = stand_in.filter(|(for_id, _)| *for_id == track.id) {
                                let _ = this.app.emit("player:lyrics-shift", serde_json::json!({ "id": track.id, "ms": (shift * 1000.0).round() as i64 }));
                            }
                        }
                        Fetched::Widget(play_id) => {
                            tracing::info!(track = track.id, "DRM-only stream: playing in SoundCloud's widget");
                            this.out(AudioCmd::Stop);
                            this.external.store(true, Ordering::SeqCst);
                            this.widget.load(play_id, generation);
                        }
                    }
                    let widget = this.external.load(Ordering::SeqCst);
                    {
                        let mut s = this.lock();
                        // widget: still "loading" until the sound really starts (AudioEvent::Started)
                        s.loading = widget;
                        s.playing = true;
                        s.failures = 0;
                    }
                    this.emit();
                    this.lookahead();
                    let state = this.app.state::<AppState>();
                    if let Err(e) = state.db.record_play(track.id).await {
                        tracing::warn!(error = %e, "record_play");
                    }
                }
                Err(e) => {
                    tracing::warn!(track = track.id, error = %e, "track load failed");
                    this.emit_error(&e);
                    let failures = {
                        let mut s = this.lock();
                        s.loading = false;
                        s.playing = false;
                        s.failures += 1;
                        s.failures
                    };
                    this.emit();
                    if e.is_track_specific() && failures < MAX_AUTO_SKIPS {
                        this.advance(true);
                    }
                }
            }
        });
    }

    async fn fetch_audio(&self, track_id: u64) -> AppResult<Fetched> {
        let state = self.app.state::<AppState>();
        // downloaded: play the file (offline, no quota, no region blocks)
        if let Some(path) = crate::downloads::file_of(&state, track_id).await {
            match tokio::fs::read(&path).await {
                Ok(data) => return Ok(Fetched::Bytes(data)),
                Err(e) => tracing::warn!(error = %e, "downloaded file unreadable, streaming instead"),
            }
        }
        if self.drm_only.lock().unwrap_or_else(|e| e.into_inner()).contains(&track_id) {
            return Ok(Fetched::Widget(track_id));
        }
        let used = state.db.streams_last_24h().await?;
        if used >= STREAM_QUOTA {
            return Err(AppError::StreamQuota(used));
        }
        let sc = state.sc()?;
        // always fresh: track_authorization in cached JSON expires
        let mut full = sc.track(track_id).await?;
        let mut ready: Option<Vec<u8>> = None;
        if full.is_preview() {
            // 30 s Go+ preview: play someone else's full upload of the very same recording
            let shift;
            (full, ready, shift) = match self.full_upload(&state, &sc, &full).await? {
                Some(found) => found,
                None => {
                    return Err(AppError::UnsupportedStream(
                        "в SoundCloud это только 30-секундное превью для Go+, а полной загрузки этой записи нет".into(),
                    ))
                }
            };
            *self.stand_in.lock().unwrap_or_else(|e| e.into_inner()) = Some((track_id, shift));
        }
        let downloaded = match ready {
            Some(bytes) => Ok(bytes),
            None => Box::pin(sc.download_audio(&full)).await,
        };
        let data = match downloaded {
            Ok(d) => Fetched::Bytes(d),
            // only DRM streams are actually served: SoundCloud's own player can play them
            Err(AppError::NotFound | AppError::UnsupportedStream(_)) if crate::api::soundcloud::has_drm_stream(&full) => {
                // a stand-in for a preview is found again next time, don't cache the preview's id
                if full.id == track_id {
                    self.remember_drm(track_id).await;
                }
                Fetched::Widget(full.id)
            }
            Err(AppError::NotFound) => {
                return Err(AppError::UnsupportedStream("он удалён, закрыт автором или недоступен в вашей стране".into()))
            }
            Err(e) => return Err(e),
        };
        state.db.log_stream().await?;
        state.db.upsert_tracks(vec![full]).await?;
        Ok(data)
    }

    /// Full upload for a Go+ preview, remembered per track (kv "full_alt:<id>").
    /// Full upload for a Go+ preview, checked by ear once and then remembered
    /// with its shift (kv "full_alt3:<id>"). Returns the audio too when it was
    /// just downloaded for the check.
    async fn full_upload(
        &self,
        state: &AppState,
        sc: &crate::api::soundcloud::SoundCloud,
        preview: &crate::api::soundcloud::ScTrack,
    ) -> AppResult<Option<(crate::api::soundcloud::ScTrack, Option<Vec<u8>>, f32)>> {
        let key = format!("full_alt3:{}", preview.id);
        if let Some(((alt_id, shift), _)) = state.db.kv_get::<(u64, f32)>(&key).await? {
            if let Ok(alt) = sc.track(alt_id).await {
                if alt.is_fully_playable() {
                    return Ok(Some((alt, None, shift)));
                }
            }
        }
        Ok(match Box::pin(crate::home::full_version(sc, preview)).await? {
            Some(v) => {
                tracing::info!(preview = preview.id, full = v.track.id, shift = v.shift_secs, "Go+ preview: playing a verified full upload instead");
                state.db.kv_put(&key, &(v.track.id, v.shift_secs)).await?;
                Some((v.track, Some(v.audio), v.shift_secs))
            }
            None => None,
        })
    }

    async fn remember_drm(&self, track_id: u64) {
        let ids: Vec<u64> = {
            let mut set = self.drm_only.lock().unwrap_or_else(|e| e.into_inner());
            if !set.insert(track_id) {
                return;
            }
            set.iter().copied().collect()
        };
        let _ = self.app.state::<AppState>().db.kv_put(DRM_ONLY_KEY, &ids).await;
    }

    /// While a track plays, get the next one ready so it starts at once:
    /// its audio is downloaded (a Go+ preview's stand-in found and checked by
    /// ear too), or, if only SoundCloud's own player can play it, that player
    /// loads it paused. Only one track ahead: little memory, little traffic.
    fn lookahead(self: &Arc<Self>) {
        let Some(next) = self.upcoming(1).into_iter().next() else { return };
        if self.prefetched.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|(id, ..)| *id == next.id) {
            return;
        }
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            // let the current track settle first
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            if this.upcoming(1).first().map(|t| t.id) != Some(next.id) {
                return;
            }
            let fetched = Box::pin(this.fetch_audio(next.id)).await;
            let still_next = this.upcoming(1).first().map(|t| t.id) == Some(next.id);
            match fetched {
                Ok(Fetched::Bytes(data)) if still_next => {
                    let stand_in = {
                        let mut s = this.stand_in.lock().unwrap_or_else(|e| e.into_inner());
                        match s.take() {
                            Some((id, shift)) if id == next.id => Some(shift),
                            other => {
                                *s = other;
                                None
                            }
                        }
                    };
                    tracing::debug!(track = next.id, bytes = data.len(), "next track prefetched");
                    let data = Arc::new(data);
                    // measured now, so the next start stays instant
                    this.gain_for(next.id, &data).await;
                    *this.prefetched.lock().unwrap_or_else(|e| e.into_inner()) = Some((next.id, data, stand_in));
                }
                Ok(Fetched::Widget(play_id)) if still_next => this.widget.preload(play_id),
                Ok(_) => {}
                Err(e) => tracing::debug!(track = next.id, error = %e, "prefetch failed"),
            }
        });
    }

    /// A few seconds before the end: the next track (already downloaded) starts
    /// under this one. Only between two native tracks, never into "repeat one"
    /// or past a sleep timer.
    fn start_crossfade(self: &Arc<Self>) {
        let next = self.upcoming(1).into_iter().next();
        let ready = next.as_ref().is_some_and(|n| {
            self.prefetched.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|(id, ..)| *id == n.id)
        });
        let (repeat_one, finished) = {
            let s = self.lock();
            (s.repeat == RepeatMode::One, s.current().cloned().filter(|_| s.learns_from_current()))
        };
        let sleeping = self.sleep.lock().unwrap_or_else(|e| e.into_inner()).end_of_track;
        if !ready || repeat_one || sleeping || self.external.load(Ordering::SeqCst) {
            return;
        }
        if let Some(t) = finished {
            self.feedback(t, wave::LISTENED);
        }
        tracing::debug!("crossfade into the next track");
        self.crossfade_next.store(true, Ordering::SeqCst);
        self.advance(true);
    }

    fn on_audio_event(self: &Arc<Self>, ev: AudioEvent) {
        let current = self.generation.load(Ordering::SeqCst);
        match ev {
            AudioEvent::NearEnd { generation } if generation == current => self.start_crossfade(),
            AudioEvent::Ended { generation } if generation == current => {
                if self.sleep_after_track() {
                    {
                        let mut s = self.lock();
                        s.playing = false;
                    }
                    self.emit();
                    tracing::info!("sleep timer: stopped after the track");
                    return;
                }
                let (repeat_one, finished) = {
                    let s = self.lock();
                    (s.repeat == RepeatMode::One, s.current().cloned().filter(|_| s.learns_from_current()))
                };
                if let Some(t) = finished {
                    self.feedback(t, wave::LISTENED);
                }
                if repeat_one {
                    self.out(AudioCmd::Replay { generation });
                } else {
                    self.advance(true);
                }
            }
            AudioEvent::Started { generation } if generation == current => {
                self.lock().loading = false;
                self.emit();
                let _ = self.app.emit("player:seek", self.position_ms());
            }
            AudioEvent::Error { generation, message } if generation == current => {
                let err = AppError::Audio(message);
                self.emit_error(&err);
                let failures = {
                    let mut s = self.lock();
                    s.playing = false;
                    s.failures += 1;
                    s.failures
                };
                self.emit();
                if failures < MAX_AUTO_SKIPS {
                    self.advance(true);
                }
            }
            _ => {}
        }
    }
}
