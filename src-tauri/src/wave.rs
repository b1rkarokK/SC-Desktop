//! "Моя волна": local, explainable recommendations — no ML.
//!
//! 1. Taste profile from likes (recency-weighted genres, tags, artists).
//! 2. Seeds: a few liked tracks picked by recency-weighted random, rotating.
//! 3. Candidates: `/tracks/{id}/related` per seed (cached 12 h) plus the
//!    uploads of one favourite artist (`/users/{id}/tracks`).
//! 4. Filter: disliked artists/tracks, recently played, already queued,
//!    preview-only/blocked, too short/long.
//! 5. Score = how many seeds recommended it + genre/tag/artist affinity.
//! 6. Weighted random sample without replacement (Efraimidis–Spirakis),
//!    max 2 tracks per artist, ~15 % familiar liked tracks mixed in,
//!    no artist twice in a row.
//! 7. Mood: fresh (no liked tracks, unknown artists up), familiar (half
//!    liked, known artists up), calm / energetic (genre & tag keywords).
//! 8. Listening feedback: a wave track listened to the end is a soft like
//!    (profile + seed), one skipped in the first seconds lowers its artist and
//!    genre (smoothed ratio, so one skip is not a ban). Within the session an
//!    artist skipped twice disappears from the wave.
//! 9. Context: a wave "по плейлисту / альбому / треку / артисту" takes its
//!    seeds and profile from those tracks instead of the likes; the same
//!    engine fills Smart Shuffle (recommendations between playlist tracks).

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Mutex,
};

use rand::{rngs::StdRng, Rng, SeedableRng};

use crate::{
    api::soundcloud::{ScTrack, SoundCloud},
    dedupe,
    error::{AppError, AppResult},
    models::TrackDto,
    state::AppState,
};

const PROFILE_SIZE: u32 = 1500;
const SEEDS_PER_BATCH: usize = 5;
const RECENT_SEED_MEMORY: usize = 40;
const RELATED_TTL_SECS: i64 = 12 * 3600;
const RECENT_PLAYS_EXCLUDED: u32 = 300;
const MAX_PER_ARTIST: usize = 2;
const MIN_DURATION_MS: u64 = 30_000;
const MAX_DURATION_MS: u64 = 15 * 60_000;

/// SoundCloud has no BPM/energy data — moods are approximated by genre/tag words.
const CALM_WORDS: &[&str] = &[
    "ambient", "chill", "lofi", "lo-fi", "acoustic", "piano", "downtempo", "classical", "jazz", "sleep", "relax",
    "soul", "folk", "indie", "dream", "slow", "r&b", "rnb",
];
const ENERGY_WORDS: &[&str] = &[
    "techno", "dnb", "drum", "hardstyle", "house", "edm", "trap", "phonk", "rock", "metal", "dubstep", "hard",
    "rave", "electro", "bass", "punk", "jungle", "trance",
];

/// Signals: listened to the end / skipped early.
pub const LISTENED: i8 = 1;
pub const SKIPPED: i8 = -1;
/// This many liked tracks make the user a fan: skips never push the artist out.
pub const FAN_LIKES: u64 = 2;
const SIGNALS_USED: u32 = 1500;
/// Session memory of what was listened to the end (extra seeds).
const SESSION_LISTENED: usize = 20;

/// What a wave is built around when it is not the likes.
pub struct WaveContext {
    /// "playlist" | "album" | "track" | "artist" | "shuffle"
    pub kind: &'static str,
    pub title: String,
    pub tracks: Vec<ScTrack>,
}

impl WaveContext {
    pub fn label(&self) -> String {
        match self.kind {
            "album" => format!("по альбому «{}»", self.title),
            "track" => format!("по треку «{}»", self.title),
            "artist" => format!("по артисту {}", self.title),
            _ => format!("по плейлисту «{}»", self.title),
        }
    }
}

#[derive(Default)]
struct Session {
    skipped_artists: HashMap<u64, u32>,
    listened: VecDeque<u64>,
}

#[derive(Default)]
pub struct WaveState {
    recent_seeds: Mutex<VecDeque<u64>>,
    mood: Mutex<String>,
    /// track id → "похоже на «A», «B»"
    reasons: Mutex<HashMap<u64, String>>,
    /// «Без лайкнутых»: no liked tracks and no re-uploads of them
    no_liked: std::sync::atomic::AtomicBool,
    context: Mutex<Option<std::sync::Arc<WaveContext>>>,
    session: Mutex<Session>,
}

impl WaveState {
    pub fn context(&self) -> Option<std::sync::Arc<WaveContext>> {
        self.context.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn set_context(&self, ctx: Option<WaveContext>) {
        *self.context.lock().unwrap_or_else(|p| p.into_inner()) = ctx.map(std::sync::Arc::new);
        self.recent_seeds.lock().unwrap_or_else(|p| p.into_inner()).clear();
    }

    /// Remembers how a wave track went in this session. Returns how many
    /// times its artist has been skipped in this session.
    pub fn note(&self, track_id: u64, artist_id: u64, signal: i8) -> u32 {
        let mut s = self.session.lock().unwrap_or_else(|p| p.into_inner());
        if signal == LISTENED {
            s.listened.retain(|&id| id != track_id);
            s.listened.push_front(track_id);
            s.listened.truncate(SESSION_LISTENED);
            if let Some(n) = s.skipped_artists.get_mut(&artist_id) {
                *n = n.saturating_sub(1);
            }
            0
        } else {
            let n = s.skipped_artists.entry(artist_id).or_default();
            *n += 1;
            *n
        }
    }

    fn session_skips(&self, artist_id: u64) -> u32 {
        self.session.lock().unwrap_or_else(|p| p.into_inner()).skipped_artists.get(&artist_id).copied().unwrap_or(0)
    }

    fn session_listened(&self) -> Vec<u64> {
        self.session.lock().unwrap_or_else(|p| p.into_inner()).listened.iter().copied().collect()
    }

    pub fn no_liked(&self) -> bool {
        self.no_liked.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_no_liked(&self, on: bool) {
        self.no_liked.store(on, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn mood(&self) -> String {
        let m = self.mood.lock().unwrap_or_else(|p| p.into_inner()).clone();
        if m.is_empty() {
            "normal".into()
        } else {
            m
        }
    }

    pub fn set_mood(&self, mood: &str) {
        *self.mood.lock().unwrap_or_else(|p| p.into_inner()) = mood.to_owned();
        self.recent_seeds.lock().unwrap_or_else(|p| p.into_inner()).clear();
    }

    pub fn reason(&self, track_id: u64) -> Option<String> {
        self.reasons.lock().unwrap_or_else(|p| p.into_inner()).get(&track_id).cloned()
    }
}

fn mood_factor(mood: &str, t: &ScTrack) -> f64 {
    let words = |list: &[&str]| {
        let hay = format!("{} {}", t.genre.as_deref().unwrap_or(""), t.tag_list.as_deref().unwrap_or("")).to_lowercase();
        list.iter().any(|w| hay.contains(w))
    };
    match mood {
        "calm" => {
            if words(CALM_WORDS) {
                2.5
            } else if words(ENERGY_WORDS) {
                0.3
            } else {
                1.0
            }
        }
        "energetic" => {
            if words(ENERGY_WORDS) {
                2.5
            } else if words(CALM_WORDS) {
                0.3
            } else {
                1.0
            }
        }
        _ => 1.0,
    }
}

#[derive(Default)]
struct Profile {
    genres: HashMap<String, f64>,
    tags: HashMap<String, f64>,
    artists: HashMap<u64, f64>,
    max_genre: f64,
    max_tag: f64,
    /// listened-to-the-end / skipped counts from wave feedback
    artist_fb: HashMap<u64, (f64, f64)>,
    genre_fb: HashMap<String, (f64, f64)>,
}

impl Profile {
    /// `weighted`: taste tracks with their weight (likes by recency, context
    /// tracks heavier). `signals`: wave feedback, newest first.
    fn build(weighted: &[(&ScTrack, f64)], signals: &[(ScTrack, i8)]) -> Self {
        let mut p = Profile::default();
        for (t, w) in weighted {
            p.add(t, *w);
        }
        for (i, (t, s)) in signals.iter().enumerate() {
            let w = recency_weight(i);
            let slot = if *s == LISTENED { 0 } else { 1 };
            let a = p.artist_fb.entry(t.artist_id()).or_default();
            if slot == 0 { a.0 += w } else { a.1 += w }
            if let Some(g) = norm_genre(t) {
                let e = p.genre_fb.entry(g).or_default();
                if slot == 0 { e.0 += w } else { e.1 += w }
            }
            if *s == LISTENED {
                // a soft like: counts, but less than a real one
                p.add(t, 0.4 * w);
            }
        }
        p.max_genre = p.genres.values().copied().fold(1e-9, f64::max);
        p.max_tag = p.tags.values().copied().fold(1e-9, f64::max);
        p
    }

    fn add(&mut self, t: &ScTrack, w: f64) {
        if let Some(g) = norm_genre(t) {
            *self.genres.entry(g).or_default() += w;
        }
        for tag in parse_tags(t.tag_list.as_deref().unwrap_or("")) {
            *self.tags.entry(tag).or_default() += w;
        }
        *self.artists.entry(t.artist_id()).or_default() += w.min(1.0);
    }

    /// 1.0 = no opinion; below 1 when skipped more than listened (≥ 0.25),
    /// a bit above when mostly listened (≤ 1.3). Smoothed: one skip ≈ 0.7.
    fn feedback_factor(&self, t: &ScTrack) -> f64 {
        let ratio = |(listened, skipped): (f64, f64)| ((listened + 2.0) / (listened + skipped + 4.0) * 2.0).clamp(0.25, 1.3);
        let mut artist = self.artist_fb.get(&t.artist_id()).copied().map_or(1.0, ratio);
        if self.artists.get(&t.artist_id()).is_some_and(|w| *w >= 1.5) {
            // liked many times: skips only nudge it
            artist = artist.max(0.85);
        }
        let genre = norm_genre(t).and_then(|g| self.genre_fb.get(&g).copied()).map_or(1.0, ratio);
        // genre opinion is broad, so it counts half
        artist * (1.0 + (genre - 1.0) * 0.5)
    }

    fn genre_affinity(&self, t: &ScTrack) -> f64 {
        norm_genre(t).and_then(|g| self.genres.get(&g)).map_or(0.0, |v| v / self.max_genre)
    }

    fn tag_affinity(&self, t: &ScTrack) -> f64 {
        let sum: f64 = parse_tags(t.tag_list.as_deref().unwrap_or(""))
            .iter()
            .filter_map(|tag| self.tags.get(tag))
            .map(|v| v / self.max_tag)
            .sum();
        (sum / 3.0).min(1.0)
    }

    fn artist_affinity(&self, t: &ScTrack) -> f64 {
        self.artists.get(&t.artist_id()).map_or(0.0, |n| (1.0 + n).ln())
    }
}

/// Newest likes count most; a like from 400 positions ago still counts ~1/3.
fn recency_weight(position: usize) -> f64 {
    1.0 / (1.0 + position as f64 / 200.0)
}

fn norm_genre(t: &ScTrack) -> Option<String> {
    t.genre
        .as_deref()
        .map(|g| g.trim().to_lowercase())
        .filter(|g| !g.is_empty())
}

/// SoundCloud tag_list: space separated, multi-word tags in double quotes.
pub fn parse_tags(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut flush = |cur: &mut String| {
        let t = cur.trim().to_lowercase();
        if !t.is_empty() {
            out.push(t);
        }
        cur.clear();
    };
    for c in s.chars() {
        match c {
            '"' => {
                if quoted {
                    flush(&mut cur);
                }
                quoted = !quoted;
            }
            c if c.is_whitespace() && !quoted => flush(&mut cur),
            c => cur.push(c),
        }
    }
    flush(&mut cur);
    out
}

/// Efraimidis–Spirakis: key = u^(1/w); top-k keys = weighted sample w/o replacement.
fn weighted_order<T>(items: Vec<(f64, T)>, rng: &mut impl Rng) -> Vec<T> {
    let mut keyed: Vec<(f64, T)> = items
        .into_iter()
        .filter(|(w, _)| *w > 0.0)
        .map(|(w, item)| {
            let u: f64 = rng.gen_range(f64::EPSILON..1.0);
            (u.powf(1.0 / w), item)
        })
        .collect();
    keyed.sort_by(|a, b| b.0.total_cmp(&a.0));
    keyed.into_iter().map(|(_, t)| t).collect()
}

/// What to generate: the wave itself (its context, liked tracks mixed in) or
/// recommendations only around given tracks (Smart Shuffle).
pub struct Gen {
    pub context: Option<std::sync::Arc<WaveContext>>,
    /// mix familiar tracks (likes / context tracks) in
    pub familiar: bool,
    /// None: the wave's mood
    pub mood: Option<&'static str>,
}

impl Gen {
    pub fn wave(state: &AppState) -> Self {
        Gen { context: state.wave.context(), familiar: true, mood: None }
    }
}

pub async fn generate(state: &AppState, batch: usize, exclude: &HashSet<u64>) -> AppResult<Vec<TrackDto>> {
    generate_with(state, batch, exclude, Gen::wave(state)).await
}

pub async fn generate_with(state: &AppState, batch: usize, exclude: &HashSet<u64>, gen: Gen) -> AppResult<Vec<TrackDto>> {
    let liked = state.db.liked_full(PROFILE_SIZE).await?;
    let ctx = gen.context.clone();
    let ctx_tracks: &[ScTrack] = ctx.as_ref().map_or(&[], |c| &c.tracks);
    if liked.is_empty() && ctx_tracks.is_empty() {
        return Err(AppError::Other("Нет лайков в кэше: откройте «Лайки» и нажмите «Синхронизировать».".into()));
    }
    let disliked_artists: HashSet<u64> = state.db.disliked_artists().await?.into_iter().map(|a| a.user_id).collect();
    let disliked_tracks = state.db.disliked_track_ids().await?;
    let recent = state.db.recent_plays(RECENT_PLAYS_EXCLUDED).await?;
    let signals = state.db.wave_signals(SIGNALS_USED).await?;
    let liked_ids: HashSet<u64> = liked.iter().map(|t| t.id).collect();
    let ctx_ids: HashSet<u64> = ctx_tracks.iter().map(|t| t.id).collect();
    let profile = {
        // with a context its tracks lead; the likes only colour it a little
        let like_w = if ctx_tracks.is_empty() { 1.0 } else { 0.25 };
        let mut weighted: Vec<(&ScTrack, f64)> = ctx_tracks.iter().map(|t| (t, 3.0)).collect();
        weighted.extend(liked.iter().enumerate().map(|(i, t)| (t, like_w * recency_weight(i))));
        Profile::build(&weighted, &signals)
    };
    let mut rng = StdRng::from_entropy();

    let allowed = |t: &ScTrack| {
        !disliked_artists.contains(&t.artist_id())
            && !disliked_tracks.contains(&t.id)
            && !exclude.contains(&t.id)
            && t.is_fully_playable()
            && (MIN_DURATION_MS..=MAX_DURATION_MS).contains(&t.duration)
            && state.wave.session_skips(t.artist_id()) < 2
    };

    // ---- seeds: context tracks or likes, plus what was just listened to the end
    let listened_now = state.wave.session_listened();
    // a context wave snowballs: what it already played seeds it further, or a
    // wave around a single track runs dry after the first batch
    let played: Vec<ScTrack> = if ctx_tracks.is_empty() {
        Vec::new()
    } else {
        let ids: Vec<u64> = exclude.iter().filter(|id| !ctx_ids.contains(id)).take(80).copied().collect();
        state.db.tracks_by_ids(ids).await?
    };
    let titles: HashMap<u64, String> = ctx_tracks
        .iter()
        .chain(liked.iter())
        .chain(played.iter())
        .chain(signals.iter().map(|(t, _)| t))
        .map(|t| (t.id, t.title.clone()))
        .collect();
    let seeds: Vec<u64> = {
        let mut recent_seeds = state.wave.recent_seeds.lock().unwrap_or_else(|p| p.into_inner());
        let mut pool: Vec<(f64, u64)> = if ctx_tracks.is_empty() {
            liked
                .iter()
                .enumerate()
                .filter(|(_, t)| !disliked_artists.contains(&t.artist_id()) && !recent_seeds.contains(&t.id))
                .map(|(i, t)| (recency_weight(i), t.id))
                .collect()
        } else {
            ctx_tracks.iter().filter(|t| !recent_seeds.contains(&t.id)).map(|t| (1.0, t.id)).collect()
        };
        pool.extend(
            played
                .iter()
                .filter(|t| {
                    !recent_seeds.contains(&t.id)
                        && !disliked_artists.contains(&t.artist_id())
                        && state.wave.session_skips(t.artist_id()) == 0
                })
                .map(|t| (0.4, t.id)),
        );
        if pool.is_empty() && !ctx_tracks.is_empty() {
            // a small playlist: all its tracks were seeds already, go round again
            recent_seeds.clear();
            pool = ctx_tracks.iter().map(|t| (1.0, t.id)).collect();
        }
        // tracks listened to the end in this session steer the next batches
        let session_seeds: Vec<u64> = listened_now
            .iter()
            .filter(|id| !recent_seeds.contains(id) && titles.contains_key(id))
            .take(2)
            .copied()
            .collect();
        pool.retain(|(_, id)| !session_seeds.contains(id));
        let take = SEEDS_PER_BATCH.saturating_sub(session_seeds.len());
        let mut seeds = session_seeds;
        seeds.extend(weighted_order(pool, &mut rng).into_iter().take(take));
        for &s in &seeds {
            recent_seeds.push_back(s);
            if recent_seeds.len() > RECENT_SEED_MEMORY {
                recent_seeds.pop_front();
            }
        }
        seeds
    };

    // one favourite artist (by likes, or by the context's tracks) for "more from this artist"
    let fav_artist = {
        let pool: Vec<(f64, u64)> = if ctx_tracks.is_empty() {
            profile.artists.iter().map(|(id, n)| (*n, *id)).collect()
        } else {
            let mut n: HashMap<u64, f64> = HashMap::new();
            for t in ctx_tracks {
                *n.entry(t.artist_id()).or_default() += 1.0;
            }
            n.into_iter().map(|(id, c)| (c, id)).collect()
        };
        let pool = pool.into_iter().filter(|(_, id)| *id != 0 && !disliked_artists.contains(id)).collect();
        weighted_order(pool, &mut rng).into_iter().next()
    };

    // ---- candidates
    let sc = state.sc()?;
    let fetches = seeds.iter().map(|&id| related_cached(state, &sc, id));
    let results = futures::future::join_all(fetches).await;

    let mood = gen.mood.map(String::from).unwrap_or_else(|| state.wave.mood());
    let mut hits: HashMap<u64, (ScTrack, u32)> = HashMap::new();
    let mut via: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut failures = 0;
    let mut last_err = None;
    for (seed, r) in seeds.iter().zip(results) {
        match r {
            Ok(tracks) => {
                for t in tracks {
                    via.entry(t.id).or_default().push(*seed);
                    hits.entry(t.id).or_insert_with(|| (t, 0)).1 += 1;
                }
            }
            Err(e) => {
                failures += 1;
                tracing::warn!(error = %e, "related fetch failed");
                last_err = Some(e);
            }
        }
    }
    if failures == seeds.len() && !seeds.is_empty() {
        return Err(last_err.unwrap_or_else(|| AppError::Other("related failed".into())));
    }
    if let Some(artist) = fav_artist {
        match sc.user_tracks(artist).await {
            Ok(tracks) => {
                state.db.upsert_tracks(tracks.clone()).await?;
                for t in tracks {
                    hits.entry(t.id).or_insert_with(|| (t, 0)).1 += 1;
                }
            }
            Err(e) => tracing::debug!(error = %e, "artist tracks fetch failed"),
        }
    }

    // ---- score
    let scored: Vec<(f64, ScTrack)> = hits
        .into_values()
        .filter(|(t, _)| !liked_ids.contains(&t.id) && !ctx_ids.contains(&t.id) && !recent.contains(&t.id) && allowed(t))
        .map(|(t, n)| {
            let known_artist = profile.artists.contains_key(&t.artist_id());
            let (artist_w, novelty) = match mood.as_str() {
                "fresh" => (0.1, if known_artist { 0.6 } else { 1.6 }),
                "familiar" => (1.5, if known_artist { 1.4 } else { 0.7 }),
                _ => (0.5, 1.0),
            };
            // skipped once in this session: less of this artist for now
            let session = if state.wave.session_skips(t.artist_id()) == 1 { 0.5 } else { 1.0 };
            let w = (1.0
                + 0.6 * (n.saturating_sub(1)) as f64
                + 1.5 * profile.genre_affinity(&t)
                + 1.0 * profile.tag_affinity(&t)
                + artist_w * profile.artist_affinity(&t))
                * novelty
                * mood_factor(&mood, &t)
                * profile.feedback_factor(&t)
                * session;
            (w, t)
        })
        .collect();

    let no_liked = state.wave.no_liked();
    let familiar_share = match mood.as_str() {
        _ if !gen.familiar => 0.0,
        _ if no_liked && ctx_tracks.is_empty() => 0.0,
        "fresh" => 0.0,
        "familiar" => 0.5,
        _ if !ctx_tracks.is_empty() => 0.25,
        _ => 0.15,
    };
    let familiar_n = ((batch as f64) * familiar_share).round() as usize;
    let fresh_n = batch.saturating_sub(familiar_n);

    // re-uploads / typo'd copies of liked songs ("Showdoun shadowraze"), and of
    // the context's own songs
    let mut known_keys: Vec<dedupe::TitleKey> = if no_liked { liked.iter().map(dedupe::key).collect() } else { Vec::new() };
    known_keys.extend(ctx_tracks.iter().map(dedupe::key));
    let scored: Vec<(f64, ScTrack)> = scored
        .into_iter()
        .filter(|(_, t)| {
            if known_keys.is_empty() {
                return true;
            }
            let k = dedupe::key(t);
            !known_keys.iter().any(|l| dedupe::same_song(l, &k))
        })
        .collect();

    let mut per_artist: HashMap<u64, usize> = HashMap::new();
    // no two copies of the same song inside one batch either
    let mut chosen_keys: Vec<dedupe::TitleKey> = Vec::new();
    let mut take_capped = |ordered: Vec<ScTrack>, n: usize| -> Vec<ScTrack> {
        let mut out = Vec::with_capacity(n);
        for t in ordered {
            if out.len() >= n {
                break;
            }
            let k = dedupe::key(&t);
            if chosen_keys.iter().any(|c| dedupe::same_song(c, &k)) {
                continue;
            }
            let c = per_artist.entry(t.artist_id()).or_default();
            if *c < MAX_PER_ARTIST {
                *c += 1;
                chosen_keys.push(k);
                out.push(t);
            }
        }
        out
    };

    let fresh = take_capped(weighted_order(scored, &mut rng), fresh_n);
    let familiar_pool: Vec<(f64, ScTrack)> = if ctx_tracks.is_empty() {
        liked
            .iter()
            .enumerate()
            .filter(|(_, t)| !recent.contains(&t.id) && allowed(t))
            .map(|(i, t)| (recency_weight(i) * mood_factor(&mood, t), t.clone()))
            .collect()
    } else {
        ctx_tracks
            .iter()
            .filter(|t| !recent.contains(&t.id) && allowed(t))
            .map(|t| (mood_factor(&mood, t), t.clone()))
            .collect()
    };
    // if discovery came up short, fill with more familiar tracks (unless they are off)
    let familiar_off = !gen.familiar || (no_liked && ctx_tracks.is_empty());
    let familiar_want = if familiar_off { 0 } else { familiar_n + fresh_n.saturating_sub(fresh.len()) };
    let familiar = take_capped(weighted_order(familiar_pool, &mut rng), familiar_want);

    // ---- reasons shown in the Wave view
    {
        let mut reasons = state.wave.reasons.lock().unwrap_or_else(|p| p.into_inner());
        if reasons.len() > 2000 {
            reasons.clear();
        }
        for t in &fresh {
            let names: Vec<String> = via
                .get(&t.id)
                .map(|s| s.iter().filter_map(|id| titles.get(id)).take(2).map(|n| format!("«{n}»")).collect())
                .unwrap_or_default();
            let text = if names.is_empty() {
                format!("больше от {}", t.artist_name())
            } else {
                format!("похоже на {}", names.join(", "))
            };
            reasons.insert(t.id, text);
        }
        for t in &familiar {
            let text = match ctx.as_ref().map(|c| c.kind) {
                Some("album") => "из альбома",
                Some("artist") => "этого артиста",
                Some("track") => "исходный трек",
                Some(_) => "из плейлиста",
                None => "из ваших лайков",
            };
            reasons.insert(t.id, text.into());
        }
    }

    // ---- mix
    let mut mixed: Vec<ScTrack> = fresh;
    for t in familiar {
        let at = rng.gen_range(0..=mixed.len());
        mixed.insert(at, t);
    }
    spread_artists(&mut mixed);

    tracing::info!(seeds = seeds.len(), out = mixed.len(), context = ctx.is_some(), "wave batch generated");
    if mixed.is_empty() && ctx.is_some() && gen.familiar {
        // the context ran dry: carry on from the likes rather than stop
        tracing::info!("context wave exhausted, falling back to the likes");
        return Box::pin(generate_with(state, batch, exclude, Gen { context: None, familiar: true, mood: gen.mood })).await;
    }
    Ok(mixed.iter().map(TrackDto::from).collect())
}

pub async fn related_cached(state: &AppState, sc: &SoundCloud, seed: u64) -> AppResult<Vec<ScTrack>> {
    if let Some(ids) = state.db.related_get(seed, RELATED_TTL_SECS).await? {
        let tracks = state.db.tracks_by_ids(ids).await?;
        if !tracks.is_empty() {
            return Ok(tracks);
        }
    }
    let tracks = sc.related(seed).await?;
    state.db.upsert_tracks(tracks.clone()).await?;
    state.db.related_put(seed, tracks.iter().map(|t| t.id).collect()).await?;
    Ok(tracks)
}

/// Greedy pass so the same artist never plays twice in a row (when avoidable).
fn spread_artists(list: &mut [ScTrack]) {
    for i in 1..list.len() {
        if list[i].artist_id() == list[i - 1].artist_id() {
            if let Some(j) = (i + 1..list.len()).find(|&j| list[j].artist_id() != list[i - 1].artist_id()) {
                list.swap(i, j);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_tags() {
        assert_eq!(
            parse_tags(r#"techno "deep house" Berlin  "lo fi""#),
            vec!["techno", "deep house", "berlin", "lo fi"]
        );
    }

    fn track(id: u64, artist: u64, genre: &str) -> ScTrack {
        serde_json::from_value(serde_json::json!({
            "id": id, "title": format!("t{id}"), "duration": 180000, "genre": genre,
            "user": { "id": artist, "username": format!("a{artist}") }
        }))
        .unwrap()
    }

    #[test]
    fn skips_lower_and_listens_raise_an_artist() {
        let skipped = track(1, 10, "phonk");
        let listened = track(2, 20, "phonk");
        let signals = vec![
            (track(3, 10, "phonk"), SKIPPED),
            (track(4, 10, "phonk"), SKIPPED),
            (track(5, 10, "phonk"), SKIPPED),
            (track(6, 20, "phonk"), LISTENED),
            (track(7, 20, "phonk"), LISTENED),
        ];
        let p = Profile::build(&[], &signals);
        let unknown = track(8, 30, "jazz");
        assert!(p.feedback_factor(&skipped) < 0.6, "{}", p.feedback_factor(&skipped));
        assert!(p.feedback_factor(&listened) > 1.0, "{}", p.feedback_factor(&listened));
        assert_eq!(p.feedback_factor(&unknown), 1.0);
        // one skip alone is no ban
        let p1 = Profile::build(&[], &[(track(9, 40, "rock"), SKIPPED)]);
        assert!(p1.feedback_factor(&track(10, 40, "rock")) > 0.6);
    }

    #[test]
    fn session_skips_twice_and_listen_forgives() {
        let w = WaveState::default();
        assert_eq!(w.note(1, 10, SKIPPED), 1);
        assert_eq!(w.note(2, 10, SKIPPED), 2);
        w.note(3, 10, LISTENED);
        assert_eq!(w.session_skips(10), 1);
        assert_eq!(w.session_listened(), vec![3]);
    }

    #[test]
    fn weighted_order_drops_zero_weights_and_prefers_heavy() {
        let mut rng = StdRng::seed_from_u64(7);
        let mut heavy_first = 0;
        for _ in 0..1000 {
            let order = weighted_order(vec![(0.0, 'z'), (1.0, 'a'), (20.0, 'b')], &mut rng);
            assert!(!order.contains(&'z'));
            if order[0] == 'b' {
                heavy_first += 1;
            }
        }
        assert!(heavy_first > 900);
    }
}
