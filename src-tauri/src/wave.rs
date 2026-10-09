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

#[derive(Default)]
pub struct WaveState {
    recent_seeds: Mutex<VecDeque<u64>>,
    mood: Mutex<String>,
    /// track id → "похоже на «A», «B»"
    reasons: Mutex<HashMap<u64, String>>,
    /// «Без лайкнутых»: no liked tracks and no re-uploads of them
    no_liked: std::sync::atomic::AtomicBool,
}

impl WaveState {
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

struct Profile {
    genres: HashMap<String, f64>,
    tags: HashMap<String, f64>,
    artists: HashMap<u64, f64>,
    max_genre: f64,
    max_tag: f64,
}

impl Profile {
    fn build(liked: &[ScTrack]) -> Self {
        let mut p = Profile {
            genres: HashMap::new(),
            tags: HashMap::new(),
            artists: HashMap::new(),
            max_genre: 1.0,
            max_tag: 1.0,
        };
        for (i, t) in liked.iter().enumerate() {
            let w = recency_weight(i);
            if let Some(g) = norm_genre(t) {
                *p.genres.entry(g).or_default() += w;
            }
            for tag in parse_tags(t.tag_list.as_deref().unwrap_or("")) {
                *p.tags.entry(tag).or_default() += w;
            }
            *p.artists.entry(t.artist_id()).or_default() += 1.0;
        }
        p.max_genre = p.genres.values().copied().fold(1e-9, f64::max);
        p.max_tag = p.tags.values().copied().fold(1e-9, f64::max);
        p
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

pub async fn generate(state: &AppState, batch: usize, exclude: &HashSet<u64>) -> AppResult<Vec<TrackDto>> {
    let liked = state.db.liked_full(PROFILE_SIZE).await?;
    if liked.is_empty() {
        return Err(AppError::Other("Нет лайков в кэше: откройте «Лайки» и нажмите «Синхронизировать».".into()));
    }
    let disliked_artists: HashSet<u64> = state.db.disliked_artists().await?.into_iter().map(|a| a.user_id).collect();
    let disliked_tracks = state.db.disliked_track_ids().await?;
    let recent = state.db.recent_plays(RECENT_PLAYS_EXCLUDED).await?;
    let liked_ids: HashSet<u64> = liked.iter().map(|t| t.id).collect();
    let profile = Profile::build(&liked);
    let mut rng = StdRng::from_entropy();

    let allowed = |t: &ScTrack| {
        !disliked_artists.contains(&t.artist_id())
            && !disliked_tracks.contains(&t.id)
            && !exclude.contains(&t.id)
            && t.is_fully_playable()
            && (MIN_DURATION_MS..=MAX_DURATION_MS).contains(&t.duration)
    };

    // ---- seeds
    let seeds: Vec<u64> = {
        let mut recent_seeds = state.wave.recent_seeds.lock().unwrap_or_else(|p| p.into_inner());
        let pool: Vec<(f64, u64)> = liked
            .iter()
            .enumerate()
            .filter(|(_, t)| !disliked_artists.contains(&t.artist_id()) && !recent_seeds.contains(&t.id))
            .map(|(i, t)| (recency_weight(i), t.id))
            .collect();
        let seeds: Vec<u64> = weighted_order(pool, &mut rng).into_iter().take(SEEDS_PER_BATCH).collect();
        for &s in &seeds {
            recent_seeds.push_back(s);
            if recent_seeds.len() > RECENT_SEED_MEMORY {
                recent_seeds.pop_front();
            }
        }
        seeds
    };

    // one favourite artist (weighted by like count) for "more from this artist"
    let fav_artist = {
        let pool: Vec<(f64, u64)> = profile
            .artists
            .iter()
            .filter(|(id, _)| **id != 0 && !disliked_artists.contains(id))
            .map(|(id, n)| (*n, *id))
            .collect();
        weighted_order(pool, &mut rng).into_iter().next()
    };

    // ---- candidates
    let sc = state.sc()?;
    let fetches = seeds.iter().map(|&id| related_cached(state, &sc, id));
    let results = futures::future::join_all(fetches).await;

    let mood = state.wave.mood();
    let seed_title: HashMap<u64, String> = liked.iter().map(|t| (t.id, t.title.clone())).collect();
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
        .filter(|(t, _)| !liked_ids.contains(&t.id) && !recent.contains(&t.id) && allowed(t))
        .map(|(t, n)| {
            let known_artist = profile.artists.contains_key(&t.artist_id());
            let (artist_w, novelty) = match mood.as_str() {
                "fresh" => (0.1, if known_artist { 0.6 } else { 1.6 }),
                "familiar" => (1.5, if known_artist { 1.4 } else { 0.7 }),
                _ => (0.5, 1.0),
            };
            let w = (1.0
                + 0.6 * (n.saturating_sub(1)) as f64
                + 1.5 * profile.genre_affinity(&t)
                + 1.0 * profile.tag_affinity(&t)
                + artist_w * profile.artist_affinity(&t))
                * novelty
                * mood_factor(&mood, &t);
            (w, t)
        })
        .collect();

    let no_liked = state.wave.no_liked();
    let familiar_share = match mood.as_str() {
        _ if no_liked => 0.0,
        "fresh" => 0.0,
        "familiar" => 0.5,
        _ => 0.15,
    };
    let familiar_n = ((batch as f64) * familiar_share).round() as usize;
    let fresh_n = batch.saturating_sub(familiar_n);

    // re-uploads / typo'd copies of liked songs ("Showdoun shadowraze")
    let liked_keys: Vec<dedupe::TitleKey> = if no_liked { liked.iter().map(dedupe::key).collect() } else { Vec::new() };
    let scored: Vec<(f64, ScTrack)> = scored
        .into_iter()
        .filter(|(_, t)| {
            if liked_keys.is_empty() {
                return true;
            }
            let k = dedupe::key(t);
            !liked_keys.iter().any(|l| dedupe::same_song(l, &k))
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
    let familiar_pool: Vec<(f64, ScTrack)> = liked
        .iter()
        .enumerate()
        .filter(|(_, t)| !recent.contains(&t.id) && allowed(t))
        .map(|(i, t)| (recency_weight(i) * mood_factor(&mood, t), t.clone()))
        .collect();
    // if discovery came up short, fill with more familiar tracks (unless liked are off)
    let familiar_want = if no_liked { 0 } else { familiar_n + fresh_n.saturating_sub(fresh.len()) };
    let familiar = take_capped(weighted_order(familiar_pool, &mut rng), familiar_want);

    // ---- reasons shown in the Wave view
    {
        let mut reasons = state.wave.reasons.lock().unwrap_or_else(|p| p.into_inner());
        if reasons.len() > 2000 {
            reasons.clear();
        }
        for t in &fresh {
            let titles: Vec<String> = via
                .get(&t.id)
                .map(|s| s.iter().filter_map(|id| seed_title.get(id)).take(2).map(|n| format!("«{n}»")).collect())
                .unwrap_or_default();
            let text = if titles.is_empty() {
                format!("больше от {}", t.artist_name())
            } else {
                format!("похоже на {}", titles.join(", "))
            };
            reasons.insert(t.id, text);
        }
        for t in &familiar {
            reasons.insert(t.id, "из ваших лайков".into());
        }
    }

    // ---- mix
    let mut mixed: Vec<ScTrack> = fresh;
    for t in familiar {
        let at = rng.gen_range(0..=mixed.len());
        mixed.insert(at, t);
    }
    spread_artists(&mut mixed);

    tracing::info!(seeds = seeds.len(), out = mixed.len(), "wave batch generated");
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
