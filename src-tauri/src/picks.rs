//! «Только в SC Desk»: picks no other player makes, mostly from the app's own
//! history (no network): forgotten likes, a year ago, on repeat; and the
//! underground radar: little-known artists close to the user's likes.

use std::collections::{HashMap, HashSet};

use rand::seq::SliceRandom;

use crate::{
    api::soundcloud::ScTrack,
    db::cache::now,
    error::{AppError, AppResult},
    models::TrackDto,
    state::AppState,
};

const DAY: i64 = 86_400;
/// "underground": artists with fewer followers than this
const RADAR_MAX_FOLLOWERS: u64 = 5_000;
const RADAR_MAX_PLAYS: u64 = 50_000;
const RADAR_TTL: i64 = 12 * 3600;

pub async fn get(state: &AppState, kind: &str) -> AppResult<Vec<TrackDto>> {
    match kind {
        "forgotten" => {
            // oldest likes untouched for 3 months, a different handful each time
            let mut list = state.db.forgotten_likes(now() - 90 * DAY, 300).await?;
            list.shuffle(&mut rand::thread_rng());
            list.truncate(50);
            Ok(list)
        }
        "year-ago" => {
            let center = now() - 365 * DAY;
            let (from, to) = (iso_date(center - 20 * DAY), iso_date(center + 20 * DAY));
            state.db.likes_between(from, to).await
        }
        "repeat" => state.db.on_repeat(now() - 30 * DAY, 50).await,
        "radar" => radar(state).await,
        "anti" => anti(state).await,
        _ => Err(AppError::Config(format!("неизвестная подборка {kind}"))),
    }
}

/// Little-known artists near the user's taste: related tracks of a few recent
/// likes, only artists under `RADAR_MAX_FOLLOWERS`, two tracks per artist at most.
async fn radar(state: &AppState) -> AppResult<Vec<TrackDto>> {
    if let Some((v, at)) = state.db.kv_get::<Vec<TrackDto>>("picks:radar").await? {
        if now() - at < RADAR_TTL && !v.is_empty() {
            return Ok(v);
        }
    }
    let sc = state.sc()?;
    let liked = state.db.likes_ids().await?;
    let disliked = state.db.disliked_track_ids().await?;
    let mut seeds: Vec<u64> = state.db.likes_page(0, 60).await?.iter().map(|t| t.id).collect();
    seeds.shuffle(&mut rand::thread_rng());
    seeds.truncate(12);

    let mut pool: Vec<ScTrack> = Vec::new();
    let mut seen = HashSet::new();
    for seed in seeds {
        let Ok(related) = Box::pin(crate::wave::related_cached(state, &sc, seed)).await else { continue };
        for t in related {
            let small = t.user.as_ref().and_then(|u| u.followers_count).is_some_and(|f| f < RADAR_MAX_FOLLOWERS);
            let quiet = t.playback_count.unwrap_or(0) < RADAR_MAX_PLAYS;
            if small && quiet && t.is_fully_playable() && !liked.contains(&t.id) && !disliked.contains(&t.id) && seen.insert(t.id) {
                pool.push(t);
            }
        }
    }
    pool.shuffle(&mut rand::thread_rng());
    let mut per_artist: HashMap<u64, u8> = HashMap::new();
    let list: Vec<TrackDto> = pool
        .iter()
        .filter(|t| {
            let n = per_artist.entry(t.artist_id()).or_default();
            *n += 1;
            *n <= 2
        })
        .take(50)
        .map(TrackDto::from)
        .collect();
    state.db.kv_put("picks:radar", &list).await?;
    Ok(list)
}

/// The artists whose sound «Антимузыка» is made from: noise, distortion, rap against the grain. Not a list of
/// names to play: the scene around them is what is searched for (see `anti`), the names only show where to start.
const ANTI_SEEDS: &[&str] = &["dekma"];
const ANTI_TTL: i64 = 12 * 3600;
/// from one artist, at most this many (the seeds included)
const ANTI_PER_ARTIST: u8 = 3;

/// The names in the credits of a track ("takizava, SHTRIHCOD, dekma", "a x b", "a & b feat. c").
fn credit_names(t: &ScTrack) -> Vec<String> {
    let line = t
        .artist_name()
        .replace(" x ", ",")
        .replace(" X ", ",")
        .replace(" feat. ", ",")
        .replace(" ft. ", ",")
        .replace(" prod. ", ",")
        .replace(" & ", ",")
        .replace(['&', '/', ';'], ",");
    line.split(',').map(|p| p.trim().to_owned()).filter(|p| (3..=30).contains(&p.chars().count())).collect()
}

/// The account with exactly this name (case does not matter): a search by name finds many, only one is the artist.
async fn find_artist(sc: &crate::api::soundcloud::SoundCloud, name: &str) -> Option<crate::api::soundcloud::ScUser> {
    let want = name.to_lowercase();
    sc.search_users(name, 5).await.ok()?.into_iter().find(|u| u.username.to_lowercase() == want)
}

/// The best known tracks of an artist, playable ones.
async fn top_tracks(sc: &crate::api::soundcloud::SoundCloud, user_id: u64, n: usize) -> Vec<ScTrack> {
    let Ok(mut tracks) = sc.user_tracks(user_id).await else { return Vec::new() };
    tracks.retain(|t| t.is_fully_playable());
    tracks.sort_by_key(|t| std::cmp::Reverse(t.playback_count.unwrap_or(0)));
    tracks.truncate(n);
    tracks
}

/// «Антимузыка»: not one artist but a scene. Start from the seed artists; their circle is whoever shares the
/// credits with them (a feature, a joint track); the scene is whoever shares the credits with at least two of
/// that circle. A recommendation list would do it worse: for a scene this small it is full of uploaders who
/// simply upload a lot. A small taste model (the one of the wave, wave_model.rs) is trained on the spot, the
/// seeds' and the circle's tracks as "yes", ordinary music as "no", and puts the scene's tracks in order.
async fn anti(state: &AppState) -> AppResult<Vec<TrackDto>> {
    if let Some((v, at)) = state.db.kv_get::<Vec<TrackDto>>("picks:anti6").await? {
        if now() - at < ANTI_TTL && !v.is_empty() {
            return Ok(v);
        }
    }
    let sc = state.sc()?;

    // the seeds
    let mut sample: Vec<ScTrack> = Vec::new();
    let mut known: HashSet<String> = ANTI_SEEDS.iter().map(|n| n.to_lowercase()).collect();
    for name in ANTI_SEEDS {
        if let Some(u) = find_artist(&sc, name).await {
            sample.extend(top_tracks(&sc, u.id, 60).await);
        }
    }
    if sample.is_empty() {
        return Err(AppError::Other(crate::lang::pick("Не нашлось образца для подборки", "No sample for this pick was found").into()));
    }
    let seed_artists: HashSet<u64> = sample.iter().map(|t| t.artist_id()).collect();

    // the circle: the most frequent names in the credits of the seeds' tracks
    let mut credited: HashMap<String, (String, u32)> = HashMap::new();
    for t in &sample {
        for name in credit_names(t) {
            if !known.contains(&name.to_lowercase()) {
                credited.entry(name.to_lowercase()).or_insert((name, 0)).1 += 1;
            }
        }
    }
    let mut names: Vec<(String, u32)> = credited.into_values().map(|(n, c)| (n, c)).collect();
    names.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    let mut circle: Vec<ScTrack> = Vec::new();
    let mut circle_artists: HashSet<u64> = HashSet::new();
    for (name, _) in names.into_iter().take(20) {
        let Some(u) = find_artist(&sc, &name).await else { continue };
        known.insert(name.to_lowercase());
        circle_artists.insert(u.id);
        circle.extend(top_tracks(&sc, u.id, 30).await);
    }

    // the scene: names in the credits of the circle's tracks, the more circle artists they come up with the higher
    let mut ties: HashMap<String, (String, HashSet<u64>)> = HashMap::new();
    for t in &circle {
        for name in credit_names(t) {
            if !known.contains(&name.to_lowercase()) {
                ties.entry(name.to_lowercase()).or_insert((name, HashSet::new())).1.insert(t.artist_id());
            }
        }
    }
    let mut scene_names: Vec<(String, usize)> = ties.into_values().map(|(n, from)| (n, from.len())).collect();
    scene_names.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    let mut scene: Vec<(ScTrack, usize)> = Vec::new();
    for (name, links) in scene_names.into_iter().take(24) {
        let Some(u) = find_artist(&sc, &name).await else { continue };
        known.insert(name.to_lowercase());
        scene.extend(top_tracks(&sc, u.id, 15).await.into_iter().map(|t| (t, links)));
    }

    // the model: the seeds and the circle are "yes", ordinary music is "no"
    use crate::wave_model::{Model, Sample};
    let mut samples: Vec<Sample> = sample.iter().map(|t| Sample { track: t.clone(), positive: true, weight: 1.0, ctx: None }).collect();
    samples.extend(circle.iter().map(|t| Sample { track: t.clone(), positive: true, weight: 0.6, ctx: None }));
    let own: HashSet<u64> = samples.iter().map(|s| s.track.id).collect();
    for t in state.db.sample_unliked(1500).await? {
        if !own.contains(&t.id) {
            samples.push(Sample { track: t, positive: false, weight: 0.5, ctx: None });
        }
    }
    let model = Model::train(&samples, None);

    let disliked = state.db.disliked_track_ids().await?;
    // everything that can be offered: the seeds' best few, the circle, the scene
    let mut pool: Vec<(f32, ScTrack)> = Vec::new();
    let score = |t: &ScTrack, ties: f32| model.as_ref().map_or(0.5, |m| m.score(t)) * (1.0 + 0.25 * ties);
    for t in sample.iter().take(ANTI_PER_ARTIST as usize) {
        pool.push((10.0, t.clone())); // the seeds lead
    }
    for t in &circle {
        if !own.contains(&t.id) || !sample.iter().any(|x| x.id == t.id) {
            pool.push((score(t, 2.0), t.clone()));
        }
    }
    for (t, links) in &scene {
        pool.push((score(t, (*links).min(4) as f32), t.clone()));
    }
    // a page of a genre shows the liked ones too: only what the user turned down is left out
    pool.retain(|(_, t)| !disliked.contains(&t.id));
    pool.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (sc_, t) in pool.iter().take(30) {
        tracing::info!(score = *sc_, artist = %t.artist_name(), title = %t.title, "anti candidate");
    }
    let tracks = crate::home::unique_songs(pool.into_iter().map(|(_, t)| t).collect());
    let mut per_artist: HashMap<u64, u8> = HashMap::new();
    let list: Vec<TrackDto> = tracks
        .iter()
        .filter(|t| {
            let n = per_artist.entry(t.artist_id()).or_default();
            *n += 1;
            *n <= if seed_artists.contains(&t.artist_id()) || circle_artists.contains(&t.artist_id()) { ANTI_PER_ARTIST } else { 2 }
        })
        .take(60)
        .map(TrackDto::from)
        .collect();
    state.db.kv_put("picks:anti6", &list).await?;
    Ok(list)
}

/// Unix seconds → "YYYY-MM-DD" (UTC), comparable with SoundCloud's ISO dates as text.
fn iso_date(unix: i64) -> String {
    // days → civil date (Howard Hinnant's algorithm)
    let z = unix.div_euclid(DAY) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::iso_date;

    #[test]
    fn dates() {
        assert_eq!(iso_date(0), "1970-01-01");
        assert_eq!(iso_date(1_791_590_400), "2026-10-10");
        assert_eq!(iso_date(951_782_400), "2000-02-29");
    }
}

#[cfg(test)]
mod anti_tests {
    use super::*;

    fn track(artist: &str) -> ScTrack {
        serde_json::from_value(serde_json::json!({
            "id": 1, "title": "t", "duration": 180000, "user_id": 5,
            "publisher_metadata": { "artist": artist },
        }))
        .unwrap()
    }

    #[test]
    fn credits_are_split_at_commas_ands_and_features() {
        assert_eq!(credit_names(&track("takizava, SHTRIHCOD, dekma, UMSY, WARYKID")), vec!["takizava", "SHTRIHCOD", "dekma", "UMSY", "WARYKID"]);
        assert_eq!(credit_names(&track("a1b x c2d feat. e3f")), vec!["a1b", "c2d", "e3f"]);
        assert_eq!(credit_names(&track("ВУЛЬФ & UMSY")), vec!["ВУЛЬФ", "UMSY"]);
    }

    #[test]
    fn names_too_short_or_too_long_are_not_names() {
        assert!(credit_names(&track("ab, x")).is_empty());
        assert!(credit_names(&track(&"n".repeat(31))).is_empty());
    }
}
