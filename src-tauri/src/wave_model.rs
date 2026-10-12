//! The taste model of «Моя волна»: a small factorization machine trained on
//! the user's own history, on the device. Same family of ideas the big
//! services use, scaled down to what one person's data can carry:
//!
//! * hashed sparse features (genre, tags, artist, title words, script of the
//!   title, length, popularity) like a ranking model's input layer;
//! * pairwise feature interactions through small latent vectors (a
//!   factorization machine, the cousin of a two-tower model): it learns that
//!   "phonk" is liked in Russian but skipped in English, which no sum of
//!   separate weights can say;
//! * listening context: time of day and weekday are crossed with genre and
//!   language, so night taste can differ from morning taste;
//! * graded feedback: likes > listened to the end > plain plays, early skips
//!   and «Не рекомендовать» push down, never-liked cached tracks count weakly
//!   as negatives; newer history weighs more;
//! * exploration: features seen rarely get a bonus (an upper-confidence idea
//!   from bandits), so the wave tries new things instead of replaying what it
//!   already knows; how much depends on the mood.
//!
//! Trains from scratch on every batch (a few thousand samples, ~10 ms, a few
//! hundred KB, no files, no network).

use rand::{rngs::StdRng, seq::SliceRandom, Rng, SeedableRng};

use crate::api::soundcloud::ScTrack;

const DIM: usize = 1 << 14;
/// latent size of the interaction vectors
const K: usize = 6;
const MIN_POSITIVE: usize = 40;
const MIN_NEGATIVE: usize = 15;

/// Title words that say what a track *is* (version, scene…).
const TITLE_WORDS: &[&str] = &[
    "slowed", "sped", "speed", "remix", "cover", "reverb", "nightcore", "edit", "live", "acoustic", "instrumental",
    "mashup", "bass", "boosted", "feat", "prod", "mix", "ost", "demo", "freestyle",
];

/// Time of day (4 parts) x weekday/weekend = 8 listening contexts.
pub fn context(hour: u32, weekday: u32) -> u8 {
    (hour.min(23) / 6) as u8 + if weekday == 0 || weekday == 6 { 4 } else { 0 }
}

/// Training knobs (tuned offline on real history, see `eval_on_real_history`).
#[derive(Clone, Copy)]
pub struct Params {
    pub epochs: usize,
    pub lr: f32,
    pub l2: f32,
    /// 0 = plain logistic regression, 1 = with feature interactions
    pub pair: f32,
    /// models trained on different shuffles / halves of the data, averaged (1 = a single one)
    pub seeds: usize,
    /// the share of the samples each of them sees
    pub bag: f32,
    /// artist history as features: how much of this artist was kept / skipped
    pub artist_stats: bool,
}

impl Default for Params {
    fn default() -> Self {
        Params { epochs: 30, lr: 0.3, l2: 2e-4, pair: 1.0, seeds: 1, bag: 1.0, artist_stats: false }
    }
}

pub struct Sample {
    pub track: ScTrack,
    pub positive: bool,
    pub weight: f32,
    /// when it happened, if known (likes carry no time)
    pub ctx: Option<u8>,
}

/// One trained set of weights; the model averages a few of them.
struct Core {
    w: Vec<f32>,
    v: Vec<f32>,
    bias: f32,
}

/// What the training data says about each artist: the weight of the times they were kept / skipped.
#[derive(Default, Clone)]
struct ArtistStats {
    by_artist: std::collections::HashMap<u64, (f32, f32)>,
}

impl ArtistStats {
    fn build(samples: &[Sample]) -> Self {
        let mut by_artist: std::collections::HashMap<u64, (f32, f32)> = std::collections::HashMap::new();
        for s in samples {
            let e = by_artist.entry(s.track.artist_id()).or_default();
            if s.positive {
                e.0 += s.weight;
            } else {
                e.1 += s.weight;
            }
        }
        ArtistStats { by_artist }
    }
}

/// A count of kept / skipped as a small bucket: 0, 1, 2, 3, 4-5, 6-9, 10+.
fn count_bin(x: f32) -> u8 {
    match x {
        x if x < 0.5 => 0,
        x if x < 1.5 => 1,
        x if x < 2.5 => 2,
        x if x < 3.5 => 3,
        x if x < 5.5 => 4,
        x if x < 9.5 => 5,
        _ => 6,
    }
}

pub struct Model {
    cores: Vec<Core>,
    /// how many training samples had each feature
    seen: Vec<u16>,
    ctx: Option<u8>,
    pair: f32,
    stats: Option<ArtistStats>,
}

fn bucket(s: &str) -> usize {
    // FNV-1a
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    (h % DIM as u64) as usize
}

/// Optional feature groups (bit set); all on in the app, switched in the offline eval.
pub static FEATURES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(F_ARTIST_NAME);
// measured on real history (AUC, newest likes vs never-liked): base 0.826, + artist name 0.828,
// + title words 0.802, + age 0.762, + likes count 0.823, + 25 tags 0.823; only artist name stays on
const F_TITLE: u32 = 1;
const F_ARTIST_NAME: u32 = 2;
const F_AGE: u32 = 4;
const F_LIKES: u32 = 8;
const F_TAGS_ALL: u32 = 16;
const F_NO_ARTIST_ID: u32 = 32;
const F_FOLLOW: u32 = 64;

/// SoundCloud ids of the artists the user follows (set by the wave before it trains).
#[allow(dead_code)] // measured (no gain on real history), kept for the offline eval
fn following() -> &'static std::sync::RwLock<std::collections::HashSet<u64>> {
    static F: std::sync::OnceLock<std::sync::RwLock<std::collections::HashSet<u64>>> = std::sync::OnceLock::new();
    F.get_or_init(Default::default)
}

#[allow(dead_code)]
pub fn set_following(ids: std::collections::HashSet<u64>) {
    *following().write().unwrap_or_else(|e| e.into_inner()) = ids;
}

fn on(bit: u32) -> bool {
    FEATURES.load(std::sync::atomic::Ordering::Relaxed) & bit != 0
}

/// stats: the artist history of the training data; own: the sample itself when it is in that data (its
/// own part is taken out, otherwise a training track would "know" its own label).
fn features(t: &ScTrack, ctx: Option<u8>, stats: Option<&ArtistStats>, own: Option<(bool, f32)>) -> Vec<usize> {
    let mut f = Vec::with_capacity(32);
    let genre = t.genre.as_deref().map(|g| g.trim().to_lowercase()).filter(|g| !g.is_empty());
    if let Some(g) = &genre {
        f.push(bucket(&format!("g:{g}")));
    }
    for tag in crate::wave::parse_tags(t.tag_list.as_deref().unwrap_or("")).into_iter().take(if on(F_TAGS_ALL) { 25 } else { 10 }) {
        f.push(bucket(&format!("t:{tag}")));
    }
    if !on(F_NO_ARTIST_ID) {
        f.push(bucket(&format!("a:{}", t.artist_id())));
    }
    if on(F_FOLLOW) && following().read().unwrap_or_else(|e| e.into_inner()).contains(&t.artist_id()) {
        f.push(bucket("fo:1"));
    }
    if let Some(st) = stats {
        let (mut kept, mut skipped) = st.by_artist.get(&t.artist_id()).copied().unwrap_or((0.0, 0.0));
        if let Some((positive, w)) = own {
            if positive {
                kept = (kept - w).max(0.0);
            } else {
                skipped = (skipped - w).max(0.0);
            }
        }
        let (k, n) = (count_bin(kept), count_bin(skipped));
        f.push(bucket(&format!("ak:{k}")));
        f.push(bucket(&format!("an0:{n}")));
        f.push(bucket(&format!("akn:{k}:{n}")));
    }
    let title = t.title.to_lowercase();
    for word in title.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
        if TITLE_WORDS.contains(&word) {
            f.push(bucket(&format!("w:{word}")));
        }
    }
    if on(F_TITLE) {
        // every word of the title: names, themes, the slang of a scene
        for word in title.split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() >= 3).take(12) {
            f.push(bucket(&format!("tw:{word}")));
        }
    }
    if on(F_ARTIST_NAME) {
        for word in t.artist_name().to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() >= 3).take(4) {
            f.push(bucket(&format!("an:{word}")));
        }
    }
    if on(F_AGE) {
        // when it came out: a scene has its years
        if let Some(y) = t.release_date.as_deref().or(t.created_at.as_deref()).and_then(|d| d.get(..4)) {
            f.push(bucket(&format!("y:{y}")));
        }
    }
    // script of the title: the language of the scene one listens to
    let (mut cyr, mut all) = (0usize, 0usize);
    for c in title.chars().filter(|c| c.is_alphabetic()) {
        all += 1;
        if ('\u{0400}'..='\u{04FF}').contains(&c) {
            cyr += 1;
        }
    }
    let script = (all > 0).then(|| if cyr * 2 >= all { "cyr" } else { "lat" });
    if let Some(s) = script {
        f.push(bucket(&format!("s:{s}")));
    }
    // how it sounds (only for tracks that were played: see audio_feat.rs)
    if let Some(a) = crate::audio_feat::get(t.id) {
        for (i, v) in a.iter().enumerate() {
            f.push(bucket(&format!("x{i}:{v}")));
        }
        // feel of the track: tempo with brightness, loudness with bass
        f.push(bucket(&format!("xa:{}:{}", a[0], a[1])));
        f.push(bucket(&format!("xb:{}:{}", a[2], a[4])));
        if let Some(g) = &genre {
            f.push(bucket(&format!("xg:{g}:{}", a[1] / 2)));
        }
    }
    f.push(bucket(&format!("d:{}", (t.duration / 60_000).min(10))));
    if let Some(n) = t.playback_count {
        // underground vs. everyone knows it, on a log scale
        f.push(bucket(&format!("p:{}", ((n as f64 + 1.0).ln() / 1.7) as u32)));
        if let Some(l) = t.likes_count {
            let ratio = (l as f64 / (n as f64 + 1.0) * 100.0) as u32; // % of listeners who liked
            f.push(bucket(&format!("r:{}", ratio.min(12))));
        }
    }
    if on(F_LIKES) {
        if let Some(l) = t.likes_count {
            f.push(bucket(&format!("lk:{}", ((l as f64 + 1.0).ln() / 1.2) as u32)));
        }
    }
    // the same genre / language at this time of day
    if let Some(c) = ctx {
        if let Some(g) = &genre {
            f.push(bucket(&format!("c{c}:g:{g}")));
        }
        if let Some(s) = script {
            f.push(bucket(&format!("c{c}:s:{s}")));
        }
    }
    f
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// Score before the sigmoid, and the per-factor sums the update needs.
fn forward(core: &Core, pair: f32, x: &[usize]) -> (f32, [f32; K]) {
    let n = x.len().max(1) as f32;
    let linear: f32 = x.iter().map(|&j| core.w[j]).sum::<f32>() / n.sqrt();
    let mut sums = [0.0f32; K];
    let mut squares = 0.0f32;
    for &j in x {
        for f in 0..K {
            let v = core.v[j * K + f];
            sums[f] += v;
            squares += v * v;
        }
    }
    let p = 0.5 * (sums.iter().map(|s| s * s).sum::<f32>() - squares) / n;
    (linear + pair * p + core.bias, sums)
}

impl Model {
    /// `None` while there is too little history to say anything.
    /// `now`: the context the wave is built in.
    pub fn train(samples: &[Sample], now: Option<u8>) -> Option<Model> {
        Self::train_with(samples, now, Params::default())
    }

    pub fn train_with(samples: &[Sample], now: Option<u8>, par: Params) -> Option<Model> {
        let pos = samples.iter().filter(|s| s.positive).count();
        if pos < MIN_POSITIVE || samples.len() - pos < MIN_NEGATIVE {
            return None;
        }
        let stats = par.artist_stats.then(|| ArtistStats::build(samples));
        let feats: Vec<Vec<usize>> = samples.iter().map(|s| features(&s.track, s.ctx, stats.as_ref(), Some((s.positive, s.weight)))).collect();
        // positives and negatives weigh the same in total, whatever their numbers
        let (wp, wn): (f32, f32) =
            samples.iter().fold((0.0, 0.0), |(p, n), s| if s.positive { (p + s.weight, n) } else { (p, n + s.weight) });
        let (kp, kn) = (if wp > 0.0 { 0.5 / wp } else { 0.0 }, if wn > 0.0 { 0.5 / wn } else { 0.0 });
        let scale = samples.len() as f32;

        let mut seen = vec![0u16; DIM];
        for x in &feats {
            for &j in x {
                seen[j] = seen[j].saturating_add(1);
            }
        }
        let mut cores = Vec::new();
        for seed in 0..par.seeds.max(1) {
            let mut rng = StdRng::seed_from_u64(7 + seed as u64);
            let mut core = Core { w: vec![0.0; DIM], v: (0..DIM * K).map(|_| rng.gen_range(-0.05..0.05)).collect(), bias: 0.0 };
            let mut order: Vec<usize> = (0..samples.len()).collect();
            if par.seeds > 1 && par.bag < 1.0 {
                // each of them sees a different part of the history
                order.shuffle(&mut rng);
                order.truncate(((samples.len() as f32) * par.bag) as usize);
            }
            for epoch in 0..par.epochs {
                order.shuffle(&mut rng);
                let lr = par.lr / (1.0 + epoch as f32 * 0.25);
                for &i in &order {
                    let s = &samples[i];
                    let x = &feats[i];
                    let y = if s.positive { 1.0 } else { 0.0 };
                    let k = s.weight * if s.positive { kp } else { kn } * scale;
                    let (raw, sums) = forward(&core, par.pair, x);
                    let err = (sigmoid(raw) - y) * k;
                    let n = x.len().max(1) as f32;
                    let (lin_norm, pair_norm) = (1.0 / n.sqrt(), 1.0 / n);
                    for &j in x {
                        core.w[j] -= lr * (err * lin_norm + par.l2 * core.w[j]);
                        for f in 0..K {
                            let vjf = core.v[j * K + f];
                            let grad = err * pair_norm * par.pair * (sums[f] - vjf);
                            core.v[j * K + f] -= lr * (grad + par.l2 * vjf);
                        }
                    }
                    core.bias -= lr * err;
                }
            }
            // a diverged fit must never reach the wave: better no model than NaN scores
            if !core.bias.is_finite() || core.w.iter().chain(&core.v).any(|x| !x.is_finite()) {
                tracing::warn!("wave model diverged, ignored");
                return None;
            }
            cores.push(core);
        }
        Some(Model { cores, seen, ctx: now, pair: par.pair, stats })
    }

    /// Mean score before the sigmoid.
    fn logit(&self, x: &[usize]) -> f32 {
        self.cores.iter().map(|c| forward(c, self.pair, x).0).sum::<f32>() / self.cores.len().max(1) as f32
    }

    /// 0 … 1: how much this track looks like what the user keeps (now).
    pub fn score(&self, t: &ScTrack) -> f32 {
        sigmoid(self.logit(&features(t, self.ctx, self.stats.as_ref(), None)))
    }

    /// 0 … 1: how little of this track the model has seen (unknown artist,
    /// rare tags): the exploration bonus.
    pub fn novelty(&self, t: &ScTrack) -> f32 {
        let x = features(t, self.ctx, self.stats.as_ref(), None);
        let sum: f32 = x.iter().map(|&j| 1.0 / (1.0 + self.seen[j] as f32).sqrt()).sum();
        sum / x.len().max(1) as f32
    }

    /// Multiplier for the wave score. `explore` 0 … 1: how much novelty is
    /// worth (mood "fresh" high, "familiar" low).
    pub fn factor(&self, t: &ScTrack, explore: f64) -> f64 {
        let f = (0.5 + self.score(t) as f64) * (1.0 + 0.4 * explore * self.novelty(t) as f64);
        if f.is_finite() {
            f
        } else {
            1.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: u64, artist: u64, genre: &str, title: &str) -> ScTrack {
        serde_json::from_value(serde_json::json!({
            "id": id, "title": title, "duration": 180000, "genre": genre, "user_id": artist,
        }))
        .unwrap()
    }

    fn sample(t: ScTrack, positive: bool, ctx: Option<u8>) -> Sample {
        Sample { track: t, positive, weight: 1.0, ctx }
    }

    #[test]
    fn learns_the_genre_that_is_kept_and_the_one_that_is_skipped() {
        let mut samples = Vec::new();
        for i in 0..80 {
            samples.push(sample(track(i, i, "phonk", "ночной драйв"), true, None));
        }
        for i in 0..40 {
            samples.push(sample(track(1000 + i, 5000 + i, "pop", "summer hit"), false, None));
        }
        let m = Model::train(&samples, None).expect("enough data");
        let like = m.factor(&track(9001, 777, "phonk", "тёмный район"), 0.0);
        let dislike = m.factor(&track(9002, 778, "pop", "summer love"), 0.0);
        assert!(like > 1.2, "{like}");
        assert!(dislike < 0.8, "{dislike}");
    }

    #[test]
    fn learns_an_interaction_a_plain_sum_cannot() {
        // kept: phonk in Russian, pop in English. skipped: the other two pairs
        let mut samples = Vec::new();
        for i in 0..60u64 {
            samples.push(sample(track(i, i, "phonk", "ночной драйв"), true, None));
            samples.push(sample(track(100 + i, 100 + i, "pop", "summer hit"), true, None));
            samples.push(sample(track(200 + i, 200 + i, "phonk", "night drive"), false, None));
            samples.push(sample(track(300 + i, 300 + i, "pop", "летний хит"), false, None));
        }
        let m = Model::train(&samples, None).expect("enough data");
        let ru_phonk = m.score(&track(9001, 9001, "phonk", "тёмный район"));
        let en_phonk = m.score(&track(9002, 9002, "phonk", "dark district"));
        let en_pop = m.score(&track(9003, 9003, "pop", "sunny day"));
        let ru_pop = m.score(&track(9004, 9004, "pop", "солнечный день"));
        assert!(ru_phonk > 0.6 && en_pop > 0.6, "{ru_phonk} {en_pop}");
        assert!(en_phonk < 0.4 && ru_pop < 0.4, "{en_phonk} {ru_pop}");
    }

    #[test]
    fn night_taste_differs_from_day_taste() {
        let night = context(2, 3);
        let day = context(14, 3);
        let mut samples = Vec::new();
        for i in 0..60u64 {
            samples.push(sample(track(i, i, "ambient", "тихо"), true, Some(night)));
            samples.push(sample(track(100 + i, 100 + i, "ambient", "тихо"), false, Some(day)));
            samples.push(sample(track(200 + i, 200 + i, "rock", "громко"), true, Some(day)));
            samples.push(sample(track(300 + i, 300 + i, "rock", "громко"), false, Some(night)));
        }
        let ambient = track(9001, 9001, "ambient", "тихо");
        let rock = track(9002, 9002, "rock", "громко");
        let at_night = Model::train(&samples, Some(night)).expect("data");
        let at_day = Model::train(&samples, Some(day)).expect("data");
        assert!(at_night.score(&ambient) > at_night.score(&rock), "night");
        assert!(at_day.score(&rock) > at_day.score(&ambient), "day");
    }

    #[test]
    fn unknown_tracks_are_more_novel_than_familiar_ones() {
        let mut samples = Vec::new();
        for i in 0..60u64 {
            samples.push(sample(track(i, 1, "phonk", "ночной драйв"), true, None));
            samples.push(sample(track(100 + i, 2, "pop", "summer hit"), false, None));
        }
        let m = Model::train(&samples, None).unwrap();
        let known = m.novelty(&track(9001, 1, "phonk", "ночной драйв"));
        let unknown = m.novelty(&track(9002, 9999, "dnb", "rare tune"));
        assert!(unknown > known * 2.0, "{unknown} vs {known}");
    }

    #[test]
    fn stays_silent_without_history() {
        let few = vec![sample(track(1, 1, "a", "b"), true, None)];
        assert!(Model::train(&few, None).is_none());
    }

    /// Rolling origin on the real history: likes newest first, six blocks; each of the five newest blocks is
    /// the test in turn and the model learns only from the blocks OLDER than it (what happens in life: learn
    /// from the past, rank what comes next). The score is how well it ranks the held-out likes above tracks
    /// that were never liked. `SCDB=<copy of cache.sqlite3> cargo test cv_on_real_history -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn cv_on_real_history() {
        let Ok(path) = std::env::var("SCDB") else { return };
        let c = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let load = |sql: &str| -> Vec<ScTrack> {
            let mut st = c.prepare(sql).unwrap();
            let rows = st.query_map([], |r| r.get::<_, String>(0)).unwrap();
            rows.filter_map(|r| r.ok()).filter_map(|j| serde_json::from_str::<ScTrack>(&j).ok()).collect()
        };
        let likes = load("SELECT t.json FROM likes l JOIN tracks t ON t.id = l.track_id ORDER BY l.position ASC");
        let unliked = load("SELECT json FROM tracks WHERE id NOT IN (SELECT track_id FROM likes)");
        let skips = load("SELECT t.json FROM wave_signals w JOIN tracks t ON t.id = w.track_id WHERE w.signal < 0");
        let listened = load("SELECT t.json FROM wave_signals w JOIN tracks t ON t.id = w.track_id WHERE w.signal > 0");
        println!("likes {} unliked {} skips {} listened {}", likes.len(), unliked.len(), skips.len(), listened.len());
        // the follow list (kv "lib:artists": [{"id": …}])
        let follows: std::collections::HashSet<u64> = c
            .query_row("SELECT json FROM kv WHERE key = 'lib:artists'", [], |r| r.get::<_, String>(0))
            .ok()
            .and_then(|j| serde_json::from_str::<Vec<serde_json::Value>>(&j).ok())
            .map(|v| v.iter().filter_map(|u| u["id"].as_u64()).collect())
            .unwrap_or_default();
        println!("follows {}", follows.len());
        set_following(follows);
        const BLOCKS: usize = 6;
        let chunk = likes.len() / BLOCKS;

        let run_h = |name: &str, par: Params, neg_n: usize, neg_w: f32, flags: u32, half: f32| {
            FEATURES.store(flags, std::sync::atomic::Ordering::Relaxed);
            let mut aucs = Vec::new();
            let started = std::time::Instant::now();
            for f in 0..BLOCKS - 1 {
                let test_pos: Vec<&ScTrack> = likes[f * chunk..(f + 1) * chunk].iter().collect();
                // only the likes older than the test block
                let train_likes: Vec<&ScTrack> = likes[(f + 1) * chunk..].iter().collect();
                let (mut test_neg, mut train_neg) = (Vec::new(), Vec::new());
                for (i, t) in unliked.iter().enumerate() {
                    if i % BLOCKS == f { test_neg.push(t) } else { train_neg.push(t) }
                }
                let mut v = Vec::new();
                for (i, t) in train_likes.iter().enumerate() {
                    v.push(Sample { track: (*t).clone(), positive: true, weight: if half > 0.0 { 1.0 / (1.0 + i as f32 / half) } else { 1.0 }, ctx: None });
                }
                for t in &listened { v.push(Sample { track: t.clone(), positive: true, weight: 0.6, ctx: None }); }
                for t in &skips { v.push(Sample { track: t.clone(), positive: false, weight: 1.3, ctx: None }); }
                for t in train_neg.iter().take(neg_n) { v.push(Sample { track: (*t).clone(), positive: false, weight: neg_w, ctx: None }); }
                let m = Model::train_with(&v, None, par).expect("data");
                let ps: Vec<f32> = test_pos.iter().map(|t| m.score(t)).collect();
                let ns: Vec<f32> = test_neg.iter().map(|t| m.score(t)).collect();
                let mut wins = 0.0;
                for p in &ps { for n in &ns { wins += if p > n { 1.0 } else if p == n { 0.5 } else { 0.0 }; } }
                aucs.push(wins / (ps.len() * ns.len()) as f32);
            }
            let mean = aucs.iter().sum::<f32>() / aucs.len() as f32;
            let sd = (aucs.iter().map(|a| (a - mean).powi(2)).sum::<f32>() / aucs.len() as f32).sqrt();
            let each: Vec<String> = aucs.iter().map(|a| format!("{a:.3}")).collect();
            println!("{name:34} AUC {mean:.4} +- {sd:.3}  [{}]  {} ms", each.join(" "), started.elapsed().as_millis());
        };

        let run = |name: &str, par: Params, neg_n: usize, neg_w: f32, flags: u32| run_h(name, par, neg_n, neg_w, flags, 200.0);
        let now = F_ARTIST_NAME;
        let d = Params::default();
        run("baseline (400 neg)", d, 400, 0.5, now);
        run("neg 1500", d, 1500, 0.5, now);
        run("neg 4000", d, 4000, 0.5, now);
        run("artist stats", Params { artist_stats: true, ..d }, 400, 0.5, now);
        run("5 models, 80% each", Params { seeds: 5, bag: 0.8, ..d }, 400, 0.5, now);
        run("no exact artist id", d, 400, 0.5, now | F_NO_ARTIST_ID);
        run("title words", d, 400, 0.5, now | F_TITLE);
        run("title words, no artist id", d, 400, 0.5, now | F_TITLE | F_NO_ARTIST_ID);
        run_h("neg 1500, no decay", d, 1500, 0.5, now, 0.0);
        run_h("neg 1500, half-life 80", d, 1500, 0.5, now, 80.0);
        run_h("neg 1500, half-life 500", d, 1500, 0.5, now, 500.0);
        run("neg 1500, neg weight 0.25", d, 1500, 0.25, now);
        run("neg 1500, neg weight 1.0", d, 1500, 1.0, now);
        run("neg 800", d, 800, 0.5, now);
        run("neg 2500", d, 2500, 0.5, now);
        run("follows", d, 400, 0.5, now | F_FOLLOW);
        run("follows + neg 1500", d, 1500, 0.5, now | F_FOLLOW);
        run("follows + neg 1500 + title", d, 1500, 0.5, now | F_FOLLOW | F_TITLE);
        run("epochs 60", Params { epochs: 60, ..d }, 400, 0.5, now);
        run("l2 1e-3", Params { l2: 1e-3, ..d }, 400, 0.5, now);
        run("pair 0.5", Params { pair: 0.5, ..d }, 400, 0.5, now);
        FEATURES.store(F_ARTIST_NAME, std::sync::atomic::Ordering::Relaxed);
    }

    /// Offline check on a copy of the real cache: train on older likes, ask
    /// the model to tell the newest likes from tracks that were never liked.
    /// `SCDB=<copy of cache.sqlite3> cargo test eval_on_real_history -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn eval_on_real_history() {
        let Ok(path) = std::env::var("SCDB") else { return };
        let c = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let load = |sql: &str| -> Vec<(ScTrack, i64)> {
            let mut st = c.prepare(sql).unwrap();
            let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))).unwrap();
            rows.filter_map(|r| r.ok()).filter_map(|(j, n)| serde_json::from_str::<ScTrack>(&j).ok().map(|t| (t, n))).collect()
        };
        // newest first
        let likes = load("SELECT t.json, l.position FROM likes l JOIN tracks t ON t.id = l.track_id ORDER BY l.position ASC");
        let unliked = load("SELECT json, id FROM tracks WHERE id NOT IN (SELECT track_id FROM likes)");
        let skips = load("SELECT t.json, w.signal FROM wave_signals w JOIN tracks t ON t.id = w.track_id WHERE w.signal < 0");
        let listened = load("SELECT t.json, w.signal FROM wave_signals w JOIN tracks t ON t.id = w.track_id WHERE w.signal > 0");
        println!("likes {} unliked {} skips {} listened {}", likes.len(), unliked.len(), skips.len(), listened.len());
        let hold = likes.len() / 6;
        let (test_pos, train_likes) = likes.split_at(hold);
        let (mut test_neg, mut train_neg) = (Vec::new(), Vec::new());
        for (i, (t, _)) in unliked.iter().enumerate() {
            if i % 5 == 0 { test_neg.push(t.clone()) } else { train_neg.push(t.clone()) }
        }
        let auc = |m: &dyn Fn(&ScTrack) -> f32| -> f32 {
            let ps: Vec<f32> = test_pos.iter().map(|(t, _)| m(t)).collect();
            let ns: Vec<f32> = test_neg.iter().map(m).collect();
            let mut wins = 0.0;
            for p in &ps { for n in &ns { wins += if p > n { 1.0 } else if p == n { 0.5 } else { 0.0 }; } }
            wins / (ps.len() * ns.len()) as f32
        };
        let build = |neg_w: f32, skip_w: f32| -> Vec<Sample> {
            let mut v = Vec::new();
            for (i, (t, _)) in train_likes.iter().enumerate() {
                v.push(Sample { track: t.clone(), positive: true, weight: 1.0 / (1.0 + i as f32 / 200.0), ctx: None });
            }
            for (t, _) in &listened { v.push(Sample { track: t.clone(), positive: true, weight: 0.6, ctx: None }); }
            for (t, _) in &skips { v.push(Sample { track: t.clone(), positive: false, weight: skip_w, ctx: None }); }
            for t in train_neg.iter().take(400) { v.push(Sample { track: t.clone(), positive: false, weight: neg_w, ctx: None }); }
            v
        };
        let started = std::time::Instant::now();
        let d = Params::default();
        for (name, mask) in [
            ("base only", 0u32),
            ("+title words", F_TITLE),
            ("+artist name", F_ARTIST_NAME),
            ("+age", F_AGE),
            ("+likes count", F_LIKES),
            ("+25 tags", F_TAGS_ALL),
            ("all", 0b11111),
            ("title+age+tags", F_TITLE | F_AGE | F_TAGS_ALL),
        ] {
            FEATURES.store(mask, std::sync::atomic::Ordering::Relaxed);
            let mut samples = build(0.5, 1.0);
            samples.truncate(samples.len() - train_neg.len().min(400));
            for t in train_neg.iter().take(400) { samples.push(Sample { track: t.clone(), positive: false, weight: 0.5, ctx: None }); }
            let m = Model::train_with(&samples, None, d).expect("data");
            println!("{name:20} AUC {:.4}", auc(&|t| m.score(t)));
        }
        FEATURES.store(F_ARTIST_NAME, std::sync::atomic::Ordering::Relaxed);
        println!("popularity only      AUC {:.4}", auc(&|t| (t.playback_count.unwrap_or(0) as f32 + 1.0).ln()));
        println!("total {} ms", started.elapsed().as_millis());
    }
}
