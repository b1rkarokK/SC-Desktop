//! «Главная» like soundcloud.com/discover: SoundCloud's own home shelves
//! (mixes made for the user, stations, trending by genre, curated playlists)
//! and «Лента»: posts and reposts of the people the user follows.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    api::soundcloud::{ScPlaylist, ScTrack, SoundCloud},
    error::AppResult,
    models::TrackDto,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HomeCard {
    /// "mix" (SoundCloud system playlist, opened by `urn`) | "playlist" (by `id`)
    pub kind: String,
    pub id: u64,
    pub urn: Option<String>,
    pub title: String,
    pub subtitle: String,
    pub artwork_url: Option<String>,
    pub track_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HomeSection {
    pub title: String,
    pub cards: Vec<HomeCard>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MixPage {
    pub title: String,
    pub description: String,
    pub artwork_url: Option<String>,
    pub permalink_url: Option<String>,
    pub tracks: Vec<TrackDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeedItem {
    pub track: TrackDto,
    /// who reposted it (None = the artist posted it)
    pub reposted_by: Vec<String>,
    /// RFC 3339
    pub at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeedPage {
    pub items: Vec<FeedItem>,
    pub next: Option<String>,
}

/// Shelf title in Russian by its urn ("soundcloud:selections:your-moods" …).
fn section_title(urn: &str, fallback: &str) -> String {
    let kind = urn.split(':').nth(2).unwrap_or("");
    match kind {
        "personalized-tracks" => "Больше того, что вам нравится",
        "recently-played" => "Недавно слушали",
        "your-moods" => "Ваши миксы",
        "made-for-you" => "Сделано для вас",
        "personalised-curated-global" | "curated" => "Подборки SoundCloud",
        "personalized-albums" => "Альбомы для вас",
        "artist-stations" => "Станции артистов",
        "trending-by-genre-playlists" | "trending-by-genre" => "В тренде по жанрам",
        "buzzing" => "Стоит послушать",
        k if k.starts_with("liked-by") => "Любимое у других",
        _ => return fallback.to_owned(),
    }
    .into()
}

/// "Your Mix 1" → "Ваш микс 1", "Related tracks: X" → "Похоже на «X»", "X's Picks" → "Выбор X".
fn card_title(title: &str) -> String {
    if let Some(n) = title.strip_prefix("Your Mix ") {
        return format!("Ваш микс {n}");
    }
    if let Some(x) = title.strip_prefix("Related tracks: ") {
        return format!("Похоже на «{x}»");
    }
    if let Some(x) = title.strip_suffix("'s Picks") {
        return format!("Выбор {x}");
    }
    title.to_owned()
}

fn artwork(v: &Value) -> Option<String> {
    v["calculated_artwork_url"]
        .as_str()
        .or_else(|| v["artwork_url"].as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn card(v: &Value) -> Option<HomeCard> {
    let kind = v["kind"].as_str()?;
    let title = card_title(v["title"].as_str().or_else(|| v["short_title"].as_str())?);
    let tracks = v["tracks"].as_array().map_or(0, Vec::len) as u64;
    let track_count = v["track_count"].as_u64().unwrap_or(tracks);
    match kind {
        "system-playlist" => {
            let subtitle = v["short_description"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| v["description"].as_str())
                .unwrap_or("")
                .chars()
                .take(80)
                .collect();
            Some(HomeCard {
                kind: "mix".into(),
                id: 0,
                urn: Some(v["urn"].as_str()?.to_owned()),
                title,
                subtitle,
                artwork_url: artwork(v),
                track_count,
            })
        }
        "playlist" => Some(HomeCard {
            kind: "playlist".into(),
            id: v["id"].as_u64()?,
            urn: None,
            title,
            subtitle: v["user"]["username"].as_str().unwrap_or("").to_owned(),
            artwork_url: artwork(v),
            track_count,
        }),
        _ => None,
    }
}

/// `/mixed-selections` → shelves. "Недавно слушали" is skipped: the app has «История».
pub fn sections(resp: &Value) -> Vec<HomeSection> {
    resp["collection"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| !s["urn"].as_str().unwrap_or("").contains(":recently-played"))
        .filter_map(|s| {
            let cards: Vec<HomeCard> = s["items"]["collection"].as_array()?.iter().filter_map(card).collect();
            (!cards.is_empty()).then(|| HomeSection {
                title: section_title(s["urn"].as_str().unwrap_or(""), s["title"].as_str().unwrap_or("")),
                cards,
            })
        })
        .collect()
}

pub async fn mix(sc: &SoundCloud, urn: &str) -> AppResult<(MixPage, Vec<ScTrack>)> {
    let v = sc.system_playlist(urn).await?;
    let ids: Vec<u64> = v["tracks"].as_array().into_iter().flatten().filter_map(|t| t["id"].as_u64()).collect();
    let tracks = unique_songs(sc.tracks_by_ids(&ids).await?);
    let page = MixPage {
        title: card_title(v["title"].as_str().unwrap_or("")),
        description: v["description"].as_str().unwrap_or("").to_owned(),
        artwork_url: artwork(&v),
        permalink_url: v["permalink_url"].as_str().map(str::to_owned),
        tracks: tracks.iter().map(TrackDto::from).collect(),
    };
    Ok((page, tracks))
}

/// The most liked real playlists (10+ tracks) across the searches.
pub async fn top_playlists(sc: &SoundCloud, queries: &[String], n: usize) -> AppResult<Vec<ScPlaylist>> {
    let mut all: Vec<ScPlaylist> = Vec::new();
    for q in queries.iter().take(4) {
        for p in sc.search_playlists(q, 30).await? {
            if p.track_count.unwrap_or(0) >= 10 && !all.iter().any(|x| x.id == p.id) {
                all.push(p);
            }
        }
    }
    all.sort_by_key(|p| std::cmp::Reverse(p.likes_count.unwrap_or(0)));
    all.truncate(n);
    Ok(all)
}

/// A theme category as tracks: what the top playlists for it agree on.
/// Tracks found in more playlists rank higher, then by plays; one upload per song.
pub async fn category_tracks(sc: &SoundCloud, queries: &[String]) -> AppResult<Vec<ScTrack>> {
    const PER_PLAYLIST: usize = 80;
    let playlists = top_playlists(sc, queries, 5).await?;
    let mut count: std::collections::HashMap<u64, u32> = std::collections::HashMap::new();
    let mut order: Vec<u64> = Vec::new();
    for p in &playlists {
        let Ok(ids) = sc.playlist_track_ids(p.id).await else { continue };
        for id in ids.into_iter().take(PER_PLAYLIST) {
            let n = count.entry(id).or_insert(0);
            if *n == 0 {
                order.push(id);
            }
            *n += 1;
        }
    }
    // most agreed-on first, keep playlist order among equals (stable sort)
    order.sort_by_key(|id| std::cmp::Reverse(count[id]));
    order.truncate(150);
    let mut tracks = sc.tracks_by_ids(&order).await?;
    tracks.sort_by_key(|t| (std::cmp::Reverse(count.get(&t.id).copied().unwrap_or(0)), std::cmp::Reverse(t.playback_count.unwrap_or(0))));
    let mut tracks = unique_songs(tracks);
    tracks.truncate(100);
    Ok(tracks)
}

/// Charts and mixes often hold the same song uploaded several times
/// ("Бременские музыканты" by the artist, a fan and a label): keep the first.
/// Region-blocked tracks are dropped. 30-second Go+ previews stay: the player
/// plays a full upload of the same song instead (see `full_version`).
pub fn unique_songs(tracks: Vec<ScTrack>) -> Vec<ScTrack> {
    let mut keys: Vec<crate::dedupe::TitleKey> = Vec::new();
    let mut out = Vec::with_capacity(tracks.len());
    for t in tracks.into_iter().filter(|t| !t.is_unplayable()) {
        let k = crate::dedupe::key(&t);
        if keys.iter().any(|seen| crate::dedupe::same_song(seen, &k)) {
            continue;
        }
        keys.push(k);
        out.push(t);
    }
    out
}

/// A stand-in for a Go+ preview: the upload, its audio, and where the
/// preview starts inside it (seconds): an upload with a longer intro is the
/// same recording shifted, and the lyrics have to shift with it.
pub struct FullVersion {
    pub track: ScTrack,
    pub audio: Vec<u8>,
    pub shift_secs: f32,
}

/// For a 30-second Go+ preview: someone else's full upload of the very same
/// recording. Candidates must pass the title rules (same song, no remix /
/// cover / live markers), then the audio itself is compared: the preview's
/// acoustic fingerprint has to be found inside the candidate almost bit for
/// bit (see fingerprint.rs). The top candidates are checked in parallel and
/// the whole search gives up after `LIMIT`.
pub async fn full_version(sc: &SoundCloud, preview: &ScTrack) -> AppResult<Option<FullVersion>> {
    const CHECKS: usize = 4;
    const LIMIT: std::time::Duration = std::time::Duration::from_secs(30);
    /// a preview sits within the first minutes of an upload
    const HEAD_SECS: u32 = 240;
    let want_ms = preview.full_duration.unwrap_or(0);
    let key = crate::dedupe::key(preview);
    let queries = [format!("{} {}", preview.artist_name(), preview.title), preview.title.clone()];
    let (a, b) = futures::join!(sc.search_tracks(&queries[0], 0), sc.search_tracks(&queries[1], 0));
    let mut seen = std::collections::HashSet::new();
    let mut cands: Vec<ScTrack> = Vec::new();
    for t in [a, b].into_iter().filter_map(Result::ok).flat_map(|(found, _)| found) {
        if t.id != preview.id
            && seen.insert(t.id)
            && t.is_fully_playable()
            && crate::charts::same_recording(&preview.title, &preview.artist_name(), &t.title)
            && crate::dedupe::same_song(&key, &crate::dedupe::key(&t))
            // another edit of the song may be a bit longer or shorter: the audio check decides
            && (want_ms == 0 || t.duration.abs_diff(want_ms) <= 90_000)
        {
            cands.push(t);
        }
    }
    if cands.is_empty() {
        return Ok(None);
    }
    // most played first, and the closest length among equals
    cands.sort_by_key(|t| (std::cmp::Reverse(t.playback_count.unwrap_or(0) / 1000), t.duration.abs_diff(want_ms)));
    cands.truncate(CHECKS);

    let search = async {
        let reference = match Box::pin(sc.download_audio(preview)).await {
            Ok(bytes) => tauri::async_runtime::spawn_blocking(move || crate::fingerprint::compute(&bytes)).await.ok().flatten(),
            Err(_) => None,
        };
        let Some(reference) = reference else {
            // no preview audio to compare with: only an upload of exactly the same length
            let Some(t) = cands.into_iter().find(|t| want_ms > 0 && t.duration.abs_diff(want_ms) <= 3_000) else { return None };
            let audio = Box::pin(sc.download_audio(&t)).await.ok()?;
            return Some(FullVersion { track: t, audio, shift_secs: 0.0 });
        };
        let reference = std::sync::Arc::new(reference);
        let checks = cands.into_iter().map(|t| {
            let reference = reference.clone();
            async move {
                let audio = Box::pin(sc.download_audio(&t)).await.ok()?;
                let (ber, shift, audio) = tauri::async_runtime::spawn_blocking(move || {
                    let (ber, shift) = crate::fingerprint::compute_head(&audio, HEAD_SECS)
                        .map_or((1.0, 0.0), |fp| crate::fingerprint::best_match(&reference, &fp));
                    (ber, shift, audio)
                })
                .await
                .ok()?;
                tracing::info!(preview = preview.id, candidate = t.id, ber, shift, "Go+ preview: audio check");
                (ber <= crate::fingerprint::SAME_MAX_BER).then_some((ber, FullVersion { track: t, audio, shift_secs: shift.max(0.0) }))
            }
        });
        // the first upload that passes wins: no waiting for slower downloads
        let mut checks: futures::stream::FuturesUnordered<_> = checks.collect();
        while let Some(done) = futures::StreamExt::next(&mut checks).await {
            if let Some((_, v)) = done {
                return Some(v);
            }
        }
        None
    };
    Ok(tokio::time::timeout(LIMIT, search).await.unwrap_or_else(|_| {
        tracing::info!(preview = preview.id, "Go+ preview: audio check timed out");
        None
    }))
}

/// One page of `/stream`: tracks only (posts and reposts), the same track
/// reposted by several people is shown once with all of them.
pub fn feed_items(resp: &Value) -> (Vec<FeedItem>, Vec<ScTrack>) {
    let mut items: Vec<FeedItem> = Vec::new();
    let mut tracks = Vec::new();
    for it in resp["collection"].as_array().into_iter().flatten() {
        let kind = it["type"].as_str().unwrap_or("");
        if !kind.starts_with("track") {
            continue;
        }
        let Ok(t) = serde_json::from_value::<ScTrack>(it["track"].clone()) else { continue };
        if t.is_unplayable() {
            continue;
        }
        let reposter = (kind == "track-repost").then(|| it["user"]["username"].as_str().unwrap_or("").to_owned());
        if let Some(existing) = items.iter_mut().find(|i| i.track.id == t.id) {
            if let Some(r) = reposter.filter(|r| !r.is_empty() && !existing.reposted_by.contains(r)) {
                existing.reposted_by.push(r);
            }
            continue;
        }
        items.push(FeedItem {
            track: TrackDto::from(&t),
            reposted_by: reposter.filter(|r| !r.is_empty()).into_iter().collect(),
            at: it["created_at"].as_str().unwrap_or("").to_owned(),
        });
        tracks.push(t);
    }
    (items, tracks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn russian_titles() {
        assert_eq!(section_title("soundcloud:selections:your-moods", "Mixed for X"), "Ваши миксы");
        assert_eq!(section_title("soundcloud:selections:liked-by-1", "Liked By"), "Любимое у других");
        assert_eq!(section_title("soundcloud:selections:new-thing", "New thing"), "New thing");
        assert_eq!(card_title("Your Mix 2"), "Ваш микс 2");
        assert_eq!(card_title("Related tracks: PESTO"), "Похоже на «PESTO»");
        assert_eq!(card_title("madk1d's Picks"), "Выбор madk1d");
    }

    #[test]
    fn skips_recently_played_and_unknown_kinds() {
        let resp = serde_json::json!({ "collection": [
            { "urn": "soundcloud:selections:recently-played:1", "items": { "collection": [ { "kind": "system-playlist", "urn": "a", "title": "x" } ] } },
            { "urn": "soundcloud:selections:made-for-you", "title": "Made for you", "items": { "collection": [
                { "kind": "system-playlist", "urn": "soundcloud:system-playlists:daily", "title": "Daily Drops", "tracks": [{ "id": 1 }, { "id": 2 }] },
                { "kind": "user", "id": 5 }
            ] } }
        ] });
        let s = sections(&resp);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].title, "Сделано для вас");
        assert_eq!(s[0].cards.len(), 1);
        assert_eq!(s[0].cards[0].track_count, 2);
    }
}
