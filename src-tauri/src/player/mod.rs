//! Queue + playback state. Lives in Rust so tray and global hotkeys work even
//! when the window (and its JS) is hidden.

pub mod audio;
pub mod eq;

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
};
use crate::{
    config::{DiscordConfig, EqConfig},
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
}

impl State {
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

pub struct Player {
    app: AppHandle,
    audio: AudioHandle,
    eq: Arc<EqShared>,
    discord: Discord,
    st: Mutex<State>,
    /// Incremented on every load; stale downloads/events are dropped.
    generation: AtomicU64,
}

impl Player {
    pub fn start(app: AppHandle) -> AppResult<Arc<Self>> {
        let cfg = app.state::<AppState>().config.get();
        let eq = EqShared::new(cfg.eq.clone());
        let (audio, mut events) = audio::spawn(eq.clone())?;
        let volume = cfg.volume.clamp(0.0, 1.0);
        audio.send(AudioCmd::Volume(volume));

        let player = Arc::new(Self {
            app,
            audio,
            eq,
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
            }),
            generation: AtomicU64::new(0),
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
            position_ms: self.audio.position_ms(),
            volume: s.volume,
            shuffle: s.shuffle,
            repeat: s.repeat,
            source: s.source,
            queue_len: s.queue.len(),
            queue_pos: s.pos,
        }
    }

    pub fn position_ms(&self) -> u64 {
        self.audio.position_ms()
    }

    pub fn current(&self) -> Option<TrackDto> {
        self.lock().current().cloned()
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
        {
            let mut s = self.lock();
            if tracks.is_empty() {
                return;
            }
            let start = start.min(tracks.len() - 1);
            s.queue = tracks;
            s.source = source;
            s.active = true;
            s.failures = 0;
            s.rebuild_order(Some(start));
        }
        self.load_current();
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
            self.audio.send(AudioCmd::Play);
            s.playing = true;
            drop(s);
            self.emit();
        }
    }

    pub fn pause(&self) {
        let mut s = self.lock();
        if s.playing {
            self.audio.send(AudioCmd::Pause);
            s.playing = false;
            drop(s);
            self.emit();
        }
    }

    pub fn seek(&self, ms: u64) {
        if self.lock().active {
            self.audio.send(AudioCmd::Seek(Duration::from_millis(ms)));
            let snap = self.snapshot();
            self.push_presence(&snap, Some(ms));
            let _ = self.app.emit("player:seek", ms);
        }
    }

    pub fn set_volume(&self, v: f32) {
        let v = v.clamp(0.0, 1.0);
        self.lock().volume = v;
        self.audio.send(AudioCmd::Volume(v));
    }

    pub fn set_shuffle(&self, on: bool) {
        {
            let mut s = self.lock();
            if s.shuffle == on {
                return;
            }
            s.shuffle = on;
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
                self.audio.send(AudioCmd::Stop);
                self.emit();
            }
            Next::Nothing => {}
        }
    }

    // ------------------------------------------------------------- wave

    pub async fn start_wave(self: &Arc<Self>) -> AppResult<usize> {
        let state = self.app.state::<AppState>();
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
    /// tracks) first, then related; further batches come from the normal wave.
    pub async fn start_wave_seeded(self: &Arc<Self>, track_id: Option<u64>, artist_id: Option<u64>) -> AppResult<usize> {
        use crate::api::soundcloud::UserSection;
        use rand::seq::SliceRandom;
        let state = self.app.state::<AppState>();
        let sc = state.sc()?;
        let disliked_tracks = state.db.disliked_track_ids().await?;
        let disliked_artists: HashSet<u64> =
            state.db.disliked_artists().await?.into_iter().map(|a| a.user_id).collect();

        let mut tracks = Vec::new();
        if let Some(id) = track_id {
            tracks.push(sc.track(id).await?);
            tracks.extend(sc.related(id).await?);
        } else if let Some(uid) = artist_id {
            if let UserSection::Tracks(mut top) = sc.user_section(uid, "popular").await? {
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
        let mut seen = HashSet::new();
        tracks.retain(|t| {
            seen.insert(t.id)
                && t.is_fully_playable()
                && !disliked_tracks.contains(&t.id)
                && (Some(t.id) == track_id || !disliked_artists.contains(&t.artist_id()))
        });
        if tracks.is_empty() {
            return Err(AppError::Other("Не нашлось треков для волны".into()));
        }
        state.db.upsert_tracks(tracks.clone()).await?;
        let dtos: Vec<TrackDto> = tracks.iter().map(TrackDto::from).collect();
        let n = dtos.len();
        self.lock().shuffle = false;
        self.play_queue(dtos, 0, QueueSource::Wave);
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
        let mut s = self.lock();
        let pos = s.pos;
        let queue = std::mem::take(&mut s.queue);
        let keep: Vec<bool> = queue.iter().map(|t| t.user_id != user_id).collect();
        let old_order = std::mem::take(&mut s.order);
        // played part + current stay untouched
        let mut new_order = old_order[..=pos.min(old_order.len().saturating_sub(1))].to_vec();
        new_order.extend(old_order.iter().skip(pos + 1).copied().filter(|&i| keep[i]));
        s.queue = queue;
        s.order = new_order;
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
        self.audio.send(AudioCmd::Stop);
        self.emit();
        let _ = self.app.emit("player:track", &track);

        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            let result = this.fetch_audio(track.id).await;
            if this.generation.load(Ordering::SeqCst) != generation {
                return; // user already moved on
            }
            match result {
                Ok(data) => {
                    this.audio.send(AudioCmd::Load { data: Arc::new(data), generation });
                    {
                        let mut s = this.lock();
                        s.loading = false;
                        s.playing = true;
                        s.failures = 0;
                    }
                    this.emit();
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

    async fn fetch_audio(&self, track_id: u64) -> AppResult<Vec<u8>> {
        let state = self.app.state::<AppState>();
        let used = state.db.streams_last_24h().await?;
        if used >= STREAM_QUOTA {
            return Err(AppError::StreamQuota(used));
        }
        let sc = state.sc()?;
        // always fresh: track_authorization in cached JSON expires
        let full = sc.track(track_id).await?;
        let data = sc.download_audio(&full).await?;
        state.db.log_stream().await?;
        state.db.upsert_tracks(vec![full]).await?;
        Ok(data)
    }

    fn on_audio_event(self: &Arc<Self>, ev: AudioEvent) {
        let current = self.generation.load(Ordering::SeqCst);
        match ev {
            AudioEvent::Ended { generation } if generation == current => {
                let repeat_one = self.lock().repeat == RepeatMode::One;
                if repeat_one {
                    self.audio.send(AudioCmd::Replay { generation });
                } else {
                    self.advance(true);
                }
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
