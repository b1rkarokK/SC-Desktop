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
const EPOCHS: usize = 16;
const LEARNING_RATE: f32 = 0.12;
const L2: f32 = 2e-4;
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

pub struct Sample {
    pub track: ScTrack,
    pub positive: bool,
    pub weight: f32,
    /// when it happened, if known (likes carry no time)
    pub ctx: Option<u8>,
}

pub struct Model {
    w: Vec<f32>,
    v: Vec<f32>,
    bias: f32,
    /// how many training samples had each feature
    seen: Vec<u16>,
    ctx: Option<u8>,
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

fn features(t: &ScTrack, ctx: Option<u8>) -> Vec<usize> {
    let mut f = Vec::with_capacity(32);
    let genre = t.genre.as_deref().map(|g| g.trim().to_lowercase()).filter(|g| !g.is_empty());
    if let Some(g) = &genre {
        f.push(bucket(&format!("g:{g}")));
    }
    for tag in crate::wave::parse_tags(t.tag_list.as_deref().unwrap_or("")).into_iter().take(10) {
        f.push(bucket(&format!("t:{tag}")));
    }
    f.push(bucket(&format!("a:{}", t.artist_id())));
    let title = t.title.to_lowercase();
    for word in title.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
        if TITLE_WORDS.contains(&word) {
            f.push(bucket(&format!("w:{word}")));
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
    f.push(bucket(&format!("d:{}", (t.duration / 60_000).min(10))));
    if let Some(n) = t.playback_count {
        // underground vs. everyone knows it, on a log scale
        f.push(bucket(&format!("p:{}", ((n as f64 + 1.0).ln() / 1.7) as u32)));
        if let Some(l) = t.likes_count {
            let ratio = (l as f64 / (n as f64 + 1.0) * 100.0) as u32; // % of listeners who liked
            f.push(bucket(&format!("r:{}", ratio.min(12))));
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

impl Model {
    /// `None` while there is too little history to say anything.
    /// `now`: the context the wave is built in.
    pub fn train(samples: &[Sample], now: Option<u8>) -> Option<Model> {
        let pos = samples.iter().filter(|s| s.positive).count();
        if pos < MIN_POSITIVE || samples.len() - pos < MIN_NEGATIVE {
            return None;
        }
        let feats: Vec<Vec<usize>> = samples.iter().map(|s| features(&s.track, s.ctx)).collect();
        // positives and negatives weigh the same in total, whatever their numbers
        let (wp, wn): (f32, f32) =
            samples.iter().fold((0.0, 0.0), |(p, n), s| if s.positive { (p + s.weight, n) } else { (p, n + s.weight) });
        let (kp, kn) = (if wp > 0.0 { 0.5 / wp } else { 0.0 }, if wn > 0.0 { 0.5 / wn } else { 0.0 });
        let scale = samples.len() as f32;

        let mut rng = StdRng::seed_from_u64(7);
        let mut m = Model {
            w: vec![0.0; DIM],
            v: (0..DIM * K).map(|_| rng.gen_range(-0.05..0.05)).collect(),
            bias: 0.0,
            seen: vec![0; DIM],
            ctx: now,
        };
        for x in &feats {
            for &j in x {
                m.seen[j] = m.seen[j].saturating_add(1);
            }
        }
        let mut order: Vec<usize> = (0..samples.len()).collect();
        for epoch in 0..EPOCHS {
            order.shuffle(&mut rng);
            let lr = LEARNING_RATE / (1.0 + epoch as f32 * 0.25);
            for &i in &order {
                let s = &samples[i];
                let x = &feats[i];
                let y = if s.positive { 1.0 } else { 0.0 };
                let k = s.weight * if s.positive { kp } else { kn } * scale;
                let (raw, sums) = m.forward(x);
                let err = (sigmoid(raw) - y) * k;
                let n = x.len().max(1) as f32;
                let (lin_norm, pair_norm) = (1.0 / n.sqrt(), 1.0 / n);
                for &j in x {
                    m.w[j] -= lr * (err * lin_norm + L2 * m.w[j]);
                    for f in 0..K {
                        let vjf = m.v[j * K + f];
                        let grad = err * pair_norm * (sums[f] - vjf);
                        m.v[j * K + f] -= lr * (grad + L2 * vjf);
                    }
                }
                m.bias -= lr * err;
            }
        }
        // a diverged fit must never reach the wave: better no model than NaN scores
        if !m.bias.is_finite() || m.w.iter().chain(&m.v).any(|x| !x.is_finite()) {
            tracing::warn!("wave model diverged, ignored");
            return None;
        }
        Some(m)
    }

    /// Score before the sigmoid, and the per-factor sums the update needs.
    fn forward(&self, x: &[usize]) -> (f32, [f32; K]) {
        let n = x.len().max(1) as f32;
        let linear: f32 = x.iter().map(|&j| self.w[j]).sum::<f32>() / n.sqrt();
        let mut sums = [0.0f32; K];
        let mut squares = 0.0f32;
        for &j in x {
            for f in 0..K {
                let v = self.v[j * K + f];
                sums[f] += v;
                squares += v * v;
            }
        }
        let pair = 0.5 * (sums.iter().map(|s| s * s).sum::<f32>() - squares) / n;
        (linear + pair + self.bias, sums)
    }

    /// 0 … 1: how much this track looks like what the user keeps (now).
    pub fn score(&self, t: &ScTrack) -> f32 {
        sigmoid(self.forward(&features(t, self.ctx)).0)
    }

    /// 0 … 1: how little of this track the model has seen (unknown artist,
    /// rare tags): the exploration bonus.
    pub fn novelty(&self, t: &ScTrack) -> f32 {
        let x = features(t, self.ctx);
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
}
