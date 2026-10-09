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
