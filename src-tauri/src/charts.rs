//! Real charts, matched onto SoundCloud.
//!
//! * «Сегодня»: Yandex Music's live chart (Russia / world, listeners per day).
//! * «Неделя / Месяц / Год»: the app keeps one snapshot of that chart per day
//!   and sums the listeners over the period.
//! * By year: Yandex Music's own year-end lists for Russia ("2023 в треках",
//!   "Топ-100 треков 2025" …) and Billboard's Year-End Hot 100 for the world.
//!
//! Every song is then looked up on SoundCloud strictly (title, artist,
//! length): no confident match means the song is skipped, never a wrong
//! track. Matches are remembered for good, so a year is slow only once.

use std::collections::HashMap;

use futures::{stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    api::{
        genius::{normalize, query_from},
        soundcloud::{ScTrack, SoundCloud},
    },
    db::cache::now,
    error::{AppError, AppResult},
    models::TrackDto,
    net::HttpClient,
    state::AppState,
};

const YA: &str = "https://api.music.yandex.net";
const DAY: i64 = 86_400;

/// A song as a chart lists it (not on SoundCloud yet).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChartSong {
    /// stable id within the source ("ya:123", "bb:2019:1")
    pub key: String,
    pub artist: String,
    pub title: String,
    pub duration_ms: u64,
    /// listeners per day (Yandex live chart only)
    #[serde(default)]
    pub listeners: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChartPage {
    pub title: String,
    pub note: String,
    pub tracks: Vec<TrackDto>,
    /// chart songs not found on SoundCloud
    pub missing: usize,
}

/// Where a year's Russian chart comes from, in order of trust:
/// * 2021+: Yandex Music's own top-100 by plays ("<year> в треках", "Топ-100 треков 2025");
/// * statistics-based lists next: "Что слушали в России в <year> году", "На репите в
///   <year>-м", "Лето-2017: Россия", "Хиты ТикТока <year>";
/// * "Верните мой <year>": the hits of that year (the only source before 2016).
/// The older "<year> в треках" lists are editorial picks, not popularity: not used.
const YA_YEARS: &[(u16, &[(&str, u32)])] = &[
    (2025, &[("yandexmusic", 1559)]),
    (2024, &[("yandexmusic", 1439)]),
    (2023, &[("yandexmusic", 1313)]),
    (2022, &[("yandexmusic", 1150), ("yandexmusic", 1188)]),
    (2021, &[("yandexmusic", 1011), ("yandexmusic", 1187), ("music-blog", 2755)]),
    (2020, &[("music-blog", 2020), ("music-blog", 2453)]),
    (2019, &[("yamusic-research", 1056), ("music-blog", 2019)]),
    (2018, &[("yamusic-research", 1044), ("yamusic-research", 1045), ("music-blog", 2018)]),
    (2017, &[("yamusic-research", 1009), ("music-blog", 1787), ("music-blog", 2017)]),
    (2016, &[("music-blog", 1481), ("music-blog", 1510), ("music-blog", 2016)]),
    (2015, &[("music-blog", 2015)]),
    (2014, &[("music-blog", 2014)]),
    (2013, &[("music-blog", 2013)]),
    (2012, &[("music-blog", 2012)]),
    (2011, &[("music-blog", 2011)]),
    (2010, &[("music-blog", 2010)]),
    (2009, &[("music-blog", 2009)]),
    (2008, &[("music-blog", 2008)]),
    (2007, &[("music-blog", 2007)]),
    (2006, &[("music-blog", 2006)]),
    (2005, &[("music-blog", 2005)]),
    (2004, &[("music-blog", 2004)]),
    (2003, &[("music-blog", 2003)]),
    (2002, &[("music-blog", 2002)]),
    (2001, &[("music-blog", 2001)]),
];

pub fn ya_years() -> Vec<u16> {
    YA_YEARS.iter().map(|(y, _)| *y).collect()
}

pub(crate) fn ya_song(t: &Value) -> Option<ChartSong> {
    let title = t["title"].as_str()?.to_owned();
    let version = t["version"].as_str().filter(|v| !v.is_empty());
    let artist = t["artists"].as_array()?.iter().filter_map(|a| a["name"].as_str()).collect::<Vec<_>>().join(", ");
    Some(ChartSong {
        key: format!("ya:{}", t["id"].as_str().map(str::to_owned).or_else(|| t["id"].as_u64().map(|n| n.to_string()))?),
        // "(Remix)", "(prod. …)": part of the song's identity on SoundCloud too
        title: match version {
            Some(v) => format!("{title} ({v})"),
            None => title,
        },
        artist,
        duration_ms: t["durationMs"].as_u64().unwrap_or(0),
        listeners: 0,
    })
}

/// Yandex Music's live chart: "russia" | "world".
pub async fn ya_chart(http: &HttpClient, region: &str) -> AppResult<Vec<ChartSong>> {
    let v: Value = http.get_json_with(&format!("{YA}/landing3/chart/{region}"), &[]).await?;
    Ok(v["result"]["chart"]["tracks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let mut s = ya_song(&e["track"])?;
            s.listeners = e["chart"]["listeners"].as_u64().unwrap_or(0);
            Some(s)
        })
        .collect())
}

async fn ya_playlist(http: &HttpClient, owner: &str, kind: u32) -> AppResult<Vec<ChartSong>> {
    let v: Value = http.get_json_with(&format!("{YA}/users/{owner}/playlists/{kind}"), &[]).await?;
    Ok(v["result"]["tracks"].as_array().into_iter().flatten().filter_map(|e| ya_song(&e["track"])).collect())
}

/// Billboard Year-End Hot 100 from Wikipedia's wikitext (top 50).
async fn billboard(http: std::sync::Arc<HttpClient>, year: u16) -> AppResult<Vec<ChartSong>> {
    let page = format!("Billboard_Year-End_Hot_100_singles_of_{year}");
    let url = format!("https://en.wikipedia.org/w/api.php?action=parse&format=json&prop=wikitext&page={page}");
    let v: Value = http.get_json_with(&url, &[]).await?;
    let text = v["parse"]["wikitext"]["*"].as_str().ok_or_else(|| AppError::Parse("wikipedia".into()))?;
    let mut songs = parse_billboard(text, year);
    // Billboard has no lengths; without them a 10-minute loop or a TV performance
    // could pass for the song. Yandex Music knows the real length.
    let pairs: Vec<(String, String)> = songs.iter().map(|s| (s.artist.clone(), s.title.clone())).collect();
    let lengths: Vec<u64> = stream::iter(pairs)
        .map(|(a, t)| {
            let http = http.clone();
            async move { ya_length(&http, &a, &t).await.unwrap_or(0) }
        })
        .buffered(4)
        .collect()
        .await;
    for (s, ms) in songs.iter_mut().zip(lengths) {
        s.duration_ms = ms;
    }
    Ok(songs)
}

/// Length of the original recording, from Yandex Music's catalogue search.
async fn ya_length(http: &HttpClient, artist: &str, title: &str) -> Option<u64> {
    let (a, t) = query_from(artist, title);
    let mut url = url::Url::parse(&format!("{YA}/search")).ok()?;
    url.query_pairs_mut().append_pair("type", "track").append_pair("page", "0").append_pair("text", &format!("{a} {t}"));
    let v: Value = http.get_json_with(url.as_str(), &[]).await.ok()?;
    let (wa, wt) = (normalize(&a), normalize(&t));
    v["result"]["tracks"]["results"].as_array()?.iter().find_map(|r| {
        let rt = normalize(r["title"].as_str()?);
        let ra = r["artists"].as_array()?.iter().filter_map(|x| x["name"].as_str()).map(normalize).collect::<Vec<_>>();
        let version_ok = r["version"].as_str().map_or(true, |v| markers(v).is_empty());
        (version_ok && (rt == wt || rt.starts_with(&wt)) && ra.iter().any(|x| *x == wa || x.contains(&wa) || wa.contains(x.as_str())))
            .then(|| r["durationMs"].as_u64())
            .flatten()
    })
}

/// Rows look like `| 1 || "[[Old Town Road]]" || [[Lil Nas X]] featuring [[Billy Ray Cyrus]]`
/// (or with the rank on its own line); links may be `[[Page|Shown]]`.
fn parse_billboard(text: &str, year: u16) -> Vec<ChartSong> {
    fn unlink(s: &str) -> String {
        let mut out = String::new();
        let mut rest = s;
        while let Some(i) = rest.find("[[") {
            out.push_str(&rest[..i]);
            let after = &rest[i + 2..];
            let Some(j) = after.find("]]") else { break };
            let inner = &after[..j];
            out.push_str(inner.rsplit('|').next().unwrap_or(inner));
            rest = &after[j + 2..];
        }
        out.push_str(rest);
        out.replace("''", "").replace('"', "").trim().to_owned()
    }
    let mut out = Vec::new();
    // join each table row into one line: rows are separated by "|-"
    for row in text.split("|-") {
        let cells: Vec<String> = row
            .lines()
            .flat_map(|l| l.trim().trim_start_matches(['|', '!']).split("||").map(str::to_owned).collect::<Vec<_>>())
            .map(|c| c.trim().to_owned())
            .filter(|c| !c.is_empty())
            .collect();
        if cells.len() < 3 {
            continue;
        }
        let Ok(rank) = cells[0].trim_start_matches("style=").parse::<u32>() else { continue };
        let title = unlink(&cells[1]);
        let artist = unlink(&cells[2])
            .replace(" featuring ", ", ")
            .replace(" and ", ", ")
            .replace(" & ", ", ");
        if title.is_empty() || artist.is_empty() || rank > 100 {
            continue;
        }
        out.push(ChartSong { key: format!("bb:{year}:{rank}"), artist, title, duration_ms: 0, listeners: 0 });
    }
    out.sort_by_key(|s| s.key.rsplit(':').next().and_then(|r| r.parse::<u32>().ok()).unwrap_or(999));
    out.truncate(50);
    out
}

// ------------------------------------------------------------ matching

/// Words that make a different recording of a song.
const VERSION_MARKERS: &[&str] = &[
    "remix", "rmx", "edit", "slowed", "sped up", "speed up", "speedup", "nightcore", "8d", "acoustic", "акустик", "cover",
    "кавер", "live", "концерт", "instrumental", "инструментал", "минус", "karaoke", "караоке", "demo", "демо", "mashup",
    "bootleg", "flip", "rework", "reverb", "bass boost", "bassboost", "phonk", "mix)", "version", "версия", "remake",
    "ремикс", "x2", "hardstyle", "jersey", "drill", "piano", "lofi", "lo-fi", "extended", "vip", "snippet", "сниппет",
    "вокал", "acapella", "a cappella", "acappella", "bbc", "session", "unplugged", "tribute", "blend", "late show",
    "tiny desk", "sped", "slow",
];

fn markers(s: &str) -> Vec<&'static str> {
    let l = s.to_lowercase();
    VERSION_MARKERS
        .iter()
        .copied()
        .filter(|m| {
            // long markers count even glued to other words ("CoverRemix")
            if m.chars().count() >= 5 && *m != "version" && *m != "piano" {
                return l.contains(m);
            }
            l.match_indices(m).any(|(i, _)| {
                // short ones only as whole words: "edit" must not hit "credit", "live" not "olive"
                let before = l[..i].chars().next_back().map_or(true, |c| !c.is_alphanumeric());
                let after = l[i + m.len()..].chars().next().map_or(true, |c| !c.is_alphanumeric());
                before && (after || m.ends_with(')'))
            })
        })
        .collect()
}

/// Bracket words that don't change the recording: "(Official Audio)", "[prod. X]", "(feat. Y)".
const HARMLESS: &[&str] = &[
    "official", "audio", "video", "music", "lyric", "lyrics", "visualizer", "clip", "клип", "премьера", "premiere", "prod",
    "produced", "by", "feat", "ft", "featuring", "with", "explicit", "clean", "hq", "hd", "4k", "remaster", "remastered",
    "single", "original", "mv", "new", "2001", "2002", "2003", "2004", "2005", "2006", "2007", "2008", "2009", "2010", "2011",
    "2012", "2013", "2014", "2015", "2016", "2017", "2018", "2019", "2020", "2021", "2022", "2023", "2024", "2025", "2026",
    "out", "now", "free", "download", "dl", "x", "и", "the", "a", "of", "from", "ost", "саундтрек", "soundtrack",
];

/// Words inside brackets of `title` (lowercase, letters/digits only).
fn bracket_words(title: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0u32;
    let mut cur = String::new();
    for c in title.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth > 0 => cur.push(c),
            _ => {}
        }
        if depth == 0 && !cur.is_empty() {
            out.extend(cur.split(|ch: char| !ch.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase));
            cur.clear();
        }
    }
    out
}

/// The upload is the very recording the chart lists: no version markers it
/// lacks, and nothing in brackets beyond harmless words, the song's own
/// bracket words and the artists' names.
pub fn same_recording(want_title: &str, want_artist: &str, cand_title: &str) -> bool {
    let want = markers(want_title);
    if markers(cand_title).iter().any(|m| !want.contains(m)) {
        return false;
    }
    let own: Vec<String> = bracket_words(want_title)
        .into_iter()
        .chain(want_artist.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase))
        .collect();
    bracket_words(cand_title).iter().all(|w| HARMLESS.contains(&w.as_str()) || own.contains(w) || w.chars().count() <= 1)
}

/// How sure we are that `t` is `song` (0 = no).
///
/// * An official upload (the artist uploaded it or is credited) needs the
///   title and, when known, the length to agree.
/// * Anyone else's upload must be the exact same recording: the same title
///   with nothing extra in brackets and a length within 4 s. Covers, parodies,
///   8-bit, guitar, live and backing-track uploads never pass.
fn match_score(song: &ChartSong, t: &ScTrack) -> i32 {
    let (want_artist, want_title) = query_from(&song.artist, &song.title);
    if !same_recording(&song.title, &song.artist, &t.title) {
        return 0;
    }
    let wt = normalize(&want_title);
    let wa = normalize(&want_artist);
    if wt.is_empty() || wa.is_empty() {
        return 0;
    }
    let (title_artist, sc_title) = query_from(t.uploader(), &t.title);
    let st = normalize(&sc_title);
    let exact_title = st == wt;
    // exact title only: "Better" must not become "Better off Alone"
    if !exact_title {
        return 0;
    }

    let channel = |name: &str| {
        // "RihannaVEVO", "Eminem Official", "MiyaGi - Topic"
        let n = normalize(name);
        n == wa || ["vevo", "official", "music", "topic", "tv", "channel"].iter().any(|s| n == format!("{wa}{s}"))
    };
    let official = channel(&t.artist_name()) || channel(t.uploader());
    let named = normalize(&title_artist) == wa;
    if !official && !named {
        return 0;
    }

    let length = if song.duration_ms == 0 {
        None
    } else {
        let real = t.full_duration.unwrap_or(t.duration).max(t.duration);
        Some(real.abs_diff(song.duration_ms))
    };
    let score_len = match length {
        None => 0,
        Some(0..=4_000) => 3,
        Some(4_001..=12_000) if official => 1,
        Some(12_001..=25_000) if official => -1,
        // someone else's upload must be the same length; nobody's may be a loop or a snippet
        Some(_) => return 0,
    };
    if !official && (length.is_none() || !exact_title) {
        return 0;
    }
    i32::from(exact_title) * 2 + if official { 6 } else { 2 } + score_len + i32::from(t.is_fully_playable())
}

async fn find_on_soundcloud(sc: &SoundCloud, song: &ChartSong) -> Option<ScTrack> {
    let (artist, title) = query_from(&song.artist, &song.title);
    let (found, _) = sc.search_tracks(&format!("{artist} {title}"), 0).await.ok()?;
    found
        .into_iter()
        .filter(|t| !t.is_unplayable())
        .map(|t| (match_score(song, &t), t))
        .filter(|(s, _)| *s >= 6)
        .max_by_key(|(s, t)| (*s, t.playback_count.unwrap_or(0)))
        .map(|(_, t)| t)
}

/// Import: SoundCloud tracks for songs (cached like the charts' lookups),
/// `progress(done)` as they are looked up; also returns the songs not found.
pub(crate) async fn find_all(state: &AppState, songs: &[ChartSong], progress: impl Fn(usize)) -> AppResult<(Vec<TrackDto>, Vec<ChartSong>)> {
    let sc = state.sc()?;
    let mut ids: HashMap<String, u64> = HashMap::new();
    let mut todo = Vec::new();
    for s in songs {
        match state.db.kv_get::<u64>(&format!("match6:{}", s.key)).await? {
            Some((id, _)) if id > 0 => {
                ids.insert(s.key.clone(), id);
            }
            _ => todo.push(s.clone()),
        }
    }
    let mut done = songs.len() - todo.len();
    progress(done);
    let mut lookups = stream::iter(todo)
        .map(|s| {
            let sc = &sc;
            async move { (s.key.clone(), find_on_soundcloud(sc, &s).await) }
        })
        .buffer_unordered(4);
    let mut fresh = Vec::new();
    while let Some((key, t)) = lookups.next().await {
        let id = t.as_ref().map_or(0, |t| t.id);
        state.db.kv_put(&format!("match6:{key}"), &id).await?;
        if let Some(t) = t {
            ids.insert(key, t.id);
            fresh.push(t);
        }
        done += 1;
        progress(done);
    }
    state.db.upsert_tracks(fresh).await?;
    let order: Vec<u64> = songs.iter().filter_map(|s| ids.get(&s.key).copied()).collect();
    let cached = state.db.tracks_by_ids(order.clone()).await?;
    let by_id: HashMap<u64, TrackDto> = cached.iter().map(|t| (t.id, TrackDto::from(t))).collect();
    let mut seen = std::collections::HashSet::new();
    let tracks = order.iter().filter(|id| seen.insert(**id)).filter_map(|id| by_id.get(id).cloned()).collect();
    let missing = songs.iter().filter(|s| !ids.contains_key(&s.key)).cloned().collect();
    Ok((tracks, missing))
}

/// SoundCloud tracks for chart songs, in chart order. Lookups are remembered
/// in kv "match6:<key>" (0 = not on SoundCloud, retried after a week).
async fn on_soundcloud(state: &AppState, songs: &[ChartSong]) -> AppResult<(Vec<TrackDto>, usize)> {
    const RETRY_MISS: i64 = 7 * DAY;
    let sc = state.sc()?;
    let mut ids: HashMap<String, u64> = HashMap::new();
    let mut todo = Vec::new();
    for s in songs {
        match state.db.kv_get::<u64>(&format!("match6:{}", s.key)).await? {
            Some((id, _)) if id > 0 => {
                ids.insert(s.key.clone(), id);
            }
            Some((_, at)) if now() - at < RETRY_MISS => {}
            _ => todo.push(s.clone()),
        }
    }
    // a few lookups at a time: fast enough, gentle on SoundCloud
    let found: Vec<(String, Option<ScTrack>)> = stream::iter(todo)
        .map(|s| {
            let sc = &sc;
            async move { (s.key.clone(), find_on_soundcloud(sc, &s).await) }
        })
        .buffer_unordered(4)
        .collect()
        .await;
    let mut fresh = Vec::new();
    for (key, t) in found {
        let id = t.as_ref().map_or(0, |t| t.id);
        state.db.kv_put(&format!("match6:{key}"), &id).await?;
        if let Some(t) = t {
            ids.insert(key, t.id);
            fresh.push(t);
        }
    }
    state.db.upsert_tracks(fresh).await?;

    let order: Vec<u64> = songs.iter().filter_map(|s| ids.get(&s.key).copied()).collect();
    let cached = state.db.tracks_by_ids(order.clone()).await?;
    let by_id: HashMap<u64, TrackDto> = cached.iter().map(|t| (t.id, TrackDto::from(t))).collect();
    let mut seen = std::collections::HashSet::new();
    let tracks: Vec<TrackDto> = order.iter().filter(|id| seen.insert(**id)).filter_map(|id| by_id.get(id).cloned()).collect();
    let missing = songs.len().saturating_sub(tracks.len());
    Ok((tracks, missing))
}

// ------------------------------------------------------------ daily snapshots

fn day_key(region: &str, unix: i64) -> String {
    format!("yachart:{region}:{}", unix.div_euclid(DAY))
}

/// Today's Yandex chart, stored once a day (the history behind week / month / year).
pub async fn snapshot(state: &AppState, region: &str) -> AppResult<Vec<ChartSong>> {
    let key = day_key(region, now());
    if let Some((songs, _)) = state.db.kv_get::<Vec<ChartSong>>(&key).await? {
        if !songs.is_empty() {
            return Ok(songs);
        }
    }
    let songs = ya_chart(&state.http(), region).await?;
    state.db.kv_put(&key, &songs).await?;
    Ok(songs)
}

/// Sum of daily listeners over the last `days` stored snapshots; returns the
/// ranking and how many days of history it is based on.
async fn period(state: &AppState, region: &str, days: i64) -> AppResult<(Vec<ChartSong>, usize)> {
    snapshot(state, region).await?;
    let today = now();
    let mut total: HashMap<String, (ChartSong, u64)> = HashMap::new();
    let mut have = 0;
    for d in 0..days {
        let Some((songs, _)) = state.db.kv_get::<Vec<ChartSong>>(&day_key(region, today - d * DAY)).await? else { continue };
        have += 1;
        for s in songs {
            let e = total.entry(s.key.clone()).or_insert_with(|| (s.clone(), 0));
            e.1 += s.listeners;
        }
    }
    let mut list: Vec<(ChartSong, u64)> = total.into_values().collect();
    list.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    Ok((list.into_iter().take(100).map(|(s, _)| s).collect(), have))
}

/// Runs once a day in the background so week / month / year have history even
/// when the charts page isn't opened.
pub fn start_daily(app: &tauri::AppHandle) {
    use tauri::Manager;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        loop {
            let state = app.state::<AppState>();
            for region in ["russia", "world"] {
                if let Err(e) = snapshot(&state, region).await {
                    tracing::debug!(error = %e, region, "chart snapshot failed");
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(3 * 3600)).await;
        }
    });
}

// ------------------------------------------------------------ pages

fn plural_days(n: usize) -> String {
    let w = match (n % 10, n % 100) {
        (1, 11) => "дней",
        (1, _) => "день",
        (2..=4, 12..=14) => "дней",
        (2..=4, _) => "дня",
        _ => "дней",
    };
    format!("{n} {w}")
}

/// kind: "ru:day" | "world:day" | "ru:week" | "ru:month" | "ru:year" | "ru:<year>" | "world:<year>"
pub async fn page(state: &AppState, kind: &str) -> AppResult<ChartPage> {
    let (region, what) = kind.split_once(':').ok_or_else(|| AppError::Config(format!("чарт {kind}")))?;
    let ya_region = if region == "ru" { "russia" } else { "world" };
    let where_ = if region == "ru" { "в России" } else { "в мире" };
    let (title, note, songs) = match what {
        "day" => (
            format!("Сегодня {where_}"),
            "Чарт Яндекс Музыки: самое популярное за последние сутки.".to_owned(),
            snapshot(state, ya_region).await?,
        ),
        "week" | "month" | "year" => {
            let days = match what {
                "week" => 7,
                "month" => 30,
                _ => 365,
            };
            let (songs, have) = period(state, ya_region, days).await?;
            let name = match what {
                "week" => "неделю",
                "month" => "месяц",
                _ => "год",
            };
            let note = if (have as i64) < days {
                format!(
                    "Сумма слушателей чарта Яндекс Музыки по дням. Программа собирает чарт каждый день, пока накоплено {} из {}.",
                    plural_days(have),
                    plural_days(days as usize)
                )
            } else {
                "Сумма слушателей чарта Яндекс Музыки по дням.".to_owned()
            };
            (format!("Хиты {where_} за {name}"), note, songs)
        }
        year => {
            let year: u16 = year.parse().map_err(|_| AppError::Config(format!("чарт {kind}")))?;
            let cache = format!("chartsrc4:{kind}");
            let songs = match state.db.kv_get::<Vec<ChartSong>>(&cache).await? {
                Some((s, _)) if !s.is_empty() => s,
                _ => {
                    let s = if region == "ru" {
                        let (_, sources) = YA_YEARS.iter().find(|(y, _)| *y == year).ok_or_else(|| AppError::Config(format!("нет чарта за {year}")))?;
                        // several lists merged: the most trusted first, each song once
                        let mut merged: Vec<ChartSong> = Vec::new();
                        let mut seen = std::collections::HashSet::new();
                        for (owner, pl) in sources.iter() {
                            for song in ya_playlist(&state.http(), owner, *pl).await? {
                                let (a, t) = query_from(&song.artist, &song.title);
                                if seen.insert((normalize(&a), normalize(&t))) {
                                    merged.push(song);
                                }
                            }
                        }
                        merged
                    } else {
                        billboard(state.http(), year).await?
                    };
                    state.db.kv_put(&cache, &s).await?;
                    s
                }
            };
            let note = if region == "ru" {
                if year >= 2021 {
                    format!("Топ-100 треков {year} года по прослушиваниям в Яндекс Музыке.")
                } else {
                    format!("Хиты {year} года в России по Яндекс Музыке: что слушали и что было на слуху.")
                }
            } else {
                format!("Годовой чарт Billboard Hot 100 за {year} год: главные хиты мира.")
            };
            (format!("{year} {where_}"), note, songs)
        }
    };
    let (tracks, missing) = on_soundcloud(state, &songs).await?;
    Ok(ChartPage { title, note, tracks, missing })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_billboard_rows() {
        let text = r#"{| class="wikitable"
! No. !! Title !! Artist(s)
|-
| 1 || "[[Old Town Road]]" || [[Lil Nas X]] featuring [[Billy Ray Cyrus]]
|-
| 2
| "[[Sunflower (Post Malone and Swae Lee song)|Sunflower (Spider-Man: Into the Spider-Verse)]]"
| [[Post Malone]] and [[Swae Lee]]
|}"#;
        let s = parse_billboard(text, 2019);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].title, "Old Town Road");
        assert_eq!(s[0].artist, "Lil Nas X, Billy Ray Cyrus");
        assert_eq!(s[1].title, "Sunflower (Spider-Man: Into the Spider-Verse)");
        assert_eq!(s[1].artist, "Post Malone, Swae Lee");
    }

    #[test]
    fn version_markers() {
        assert_eq!(markers("Птичка"), Vec::<&str>::new());
        assert!(markers("Bad Guy (Krunk! Remix)").contains(&"remix"));
        assert!(markers("Я в моменте (8D Music)").contains(&"8d"));
        assert!(markers("Сияй (Slowed down)").contains(&"slowed"));
        assert!(markers("Without Me (Acoustic)").contains(&"acoustic"));
        assert_eq!(markers("Credits"), Vec::<&str>::new());
        assert_eq!(markers("Olive"), Vec::<&str>::new());
        assert!(markers("7 Rings (Folded Dragons CoverRemix)").contains(&"cover"));
        assert!(markers("Шадэ (AVA Blend)").contains(&"blend"));
    }

    #[test]
    fn only_the_same_recording() {
        assert!(same_recording("Hot N Cold", "Katy Perry", "Katy Perry - Hot N Cold (Official Audio)"));
        assert!(same_recording("Птичка", "HammAli & Navai", "HammAli & Navai - Птичка (prod. by X)"));
        assert!(!same_recording("Poker Face", "Lady Gaga", "Lady Gaga - Poker Face [Guitar]"));
        assert!(!same_recording("Work", "Rihanna", "Rihanna - Work (Paródia Vaka Loka)"));
        assert!(!same_recording("If I Were A Boy", "Beyonce", "If I Were A Boy (Live)"));
        assert!(!same_recording("Truth Hurts", "Lizzo", "[8 BIT] Lizzo - Truth Hurts"));
    }

    #[test]
    fn days_words() {
        assert_eq!(plural_days(1), "1 день");
        assert_eq!(plural_days(3), "3 дня");
        assert_eq!(plural_days(11), "11 дней");
        assert_eq!(plural_days(30), "30 дней");
    }
}
