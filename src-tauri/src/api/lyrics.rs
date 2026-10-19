//! Synced lyrics: LRCLIB → Deezer → NetEase → Musixmatch, plain Genius as last resort.
//! (Musixmatch hands out an all-zero token since 2026-10; Deezer, with an
//! anonymous session, took over: line timings for most songs, Russian too,
//! and word timings for some.)
//!
//! Word-level timings beat line-level: if the first synced hit is line-only,
//! the remaining providers are still asked for a word-level version.
//! Every provider is best-effort — a failure just moves on to the next one.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use super::genius::{match_score, normalize};
use crate::{
    error::{AppError, AppResult},
    net::HttpClient,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Word {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Line {
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,
    pub text: String,
    #[serde(default)]
    pub words: Vec<Word>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lyrics {
    /// lrclib | netease | musixmatch | genius
    pub source: String,
    pub synced: bool,
    pub word_level: bool,
    pub lines: Vec<Line>,
    pub url: Option<String>,
    /// the original song's text shown for a changed version (slowed, sped up …)
    #[serde(default)]
    pub original: bool,
    /// how far (ms) the length of the recording these timings were made for is from ours; set only when it is large
    #[serde(default)]
    pub length_off_ms: Option<u64>,
    /// the length of the recording these timings were made for (LRCLIB tells it)
    #[serde(default)]
    pub source_duration_ms: Option<u64>,
    /// timings of the original recalculated for a sped-up / slowed version: ours / the original's length
    #[serde(default)]
    pub scaled_by: Option<f32>,
}

/// Timings of a version whose length differs more than this are likely shifted: warn.
const WARN_OFF_MS: u64 = 3_000;

impl Lyrics {
    fn new(source: &str, lines: Vec<Line>, url: Option<String>) -> Option<Self> {
        let lines: Vec<Line> = lines.into_iter().filter(|l| !(l.text.trim().is_empty() && l.start_ms.is_none())).collect();
        if lines.iter().all(|l| l.text.trim().is_empty()) {
            return None;
        }
        let synced = lines.iter().any(|l| l.start_ms.is_some());
        let word_level = lines.iter().any(|l| !l.words.is_empty());
        Some(Self { source: source.into(), synced, word_level, lines, url, original: false, length_off_ms: None, source_duration_ms: None, scaled_by: None })
    }

    /// The original's timed lyrics for a sped-up / slowed version: every time is multiplied by the ratio of the
    /// lengths (a sped-up track is the original squeezed evenly), when that ratio fits the kind of the version.
    /// Anything else (no length known, a ratio that does not fit, an edit with another intro) goes without timings.
    pub(crate) fn with_tempo(mut self, ours_ms: u64, kind: VersionKind) -> Self {
        let (Some(src), true) = (self.source_duration_ms, self.synced) else { return self.as_original() };
        if ours_ms == 0 || src == 0 {
            return self.as_original();
        }
        let r = ours_ms as f64 / src as f64;
        let fits = match kind {
            VersionKind::Slowed => (1.04..=1.8).contains(&r),
            VersionKind::Sped => (0.5..=0.96).contains(&r),
            VersionKind::Same => (0.97..=1.03).contains(&r),
        };
        if !fits {
            return self.as_original();
        }
        let mv = |t: u64| (t as f64 * r).round() as u64;
        for l in &mut self.lines {
            l.start_ms = l.start_ms.map(mv);
            l.end_ms = l.end_ms.map(mv);
            for w in &mut l.words {
                w.start_ms = mv(w.start_ms);
                w.end_ms = mv(w.end_ms);
            }
        }
        self.original = true;
        self.length_off_ms = None;
        self.scaled_by = ((r - 1.0).abs() > 0.005).then_some(r as f32);
        self
    }

    /// Timings dropped: the text of the original over a changed version.
    fn as_original(mut self) -> Self {
        for l in &mut self.lines {
            l.start_ms = None;
            l.end_ms = None;
            l.words.clear();
        }
        self.synced = false;
        self.word_level = false;
        self.original = true;
        self
    }
}

/// How a changed version differs in tempo from the original, by its title.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VersionKind {
    Slowed,
    Sped,
    /// reverb, 8D, bass boost: the tempo stays
    Same,
}

pub fn version_kind(title: &str) -> VersionKind {
    let t = title.to_lowercase();
    if ["slowed", "slow version", "daycore", "замедл"].iter().any(|m| t.contains(m)) {
        VersionKind::Slowed
    } else if ["sped", "speed up", "speedup", "speed-up", "nightcore", "ускор"].iter().any(|m| t.contains(m)) {
        VersionKind::Sped
    } else {
        VersionKind::Same
    }
}

/// Marks of a changed version of a song: tempo / effect edits keep the words.
const VERSION_MARKS: &[&str] = &[
    "ultra slowed", "super slowed", "perfectly slowed", "slowed down", "slowed", "slow version",
    "sped up", "speed up", "speedup", "spedup", "sped-up", "speed-up", "nightcore", "daycore",
    "reverb", "8d audio", "8d", "bass boosted", "bassboosted", "tiktok version", "tik tok version",
    "замедленная", "замедлено", "замедлен", "ускоренная", "ускорено", "ускорен",
];

fn has_mark(s: &str) -> bool {
    let s = s.to_lowercase();
    VERSION_MARKS.iter().any(|m| {
        s.match_indices(m).any(|(i, _)| {
            let before = s[..i].chars().last().is_none_or(|c| !c.is_alphanumeric());
            let after = s[i + m.len()..].chars().next().is_none_or(|c| !c.is_alphanumeric());
            before && after
        })
    })
}

/// "love nwantiti slowed" / "Song (Slowed + Reverb)" → Some("love nwantiti" / "Song");
/// None when the title has no such mark.
pub fn version_base(title: &str) -> Option<String> {
    if !has_mark(title) {
        return None;
    }
    // bracketed parts with a mark go whole: "(slowed + reverb)", "[sped up]"
    let mut out = String::new();
    let mut depth = 0usize;
    let mut part = String::new();
    for c in title.chars() {
        match c {
            '(' | '[' | '{' => {
                if depth == 0 {
                    out.push_str(&part);
                    part.clear();
                }
                depth += 1;
                part.push(c);
            }
            ')' | ']' | '}' if depth > 0 => {
                depth -= 1;
                part.push(c);
                if depth == 0 {
                    if !has_mark(&part) {
                        out.push_str(&part);
                    }
                    part.clear();
                }
            }
            _ => part.push(c),
        }
    }
    out.push_str(&part);
    // loose marks: "song slowed", "song - sped up + reverb"
    let mut words: Vec<&str> = out.split_whitespace().collect();
    for m in VERSION_MARKS {
        let mw: Vec<&str> = m.split_whitespace().collect();
        let mut i = 0;
        while i + mw.len() <= words.len() {
            let hit = words[i..i + mw.len()]
                .iter()
                .zip(&mw)
                .all(|(a, b)| a.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase() == *b);
            if hit {
                words.drain(i..i + mw.len());
            } else {
                i += 1;
            }
        }
    }
    let joiner = |w: &&str| matches!(w.to_lowercase().as_str(), "+" | "&" | "-" | "–" | "—" | "x" | "and" | "/" | "|" | "," | "и");
    while words.last().is_some_and(joiner) {
        words.pop();
    }
    while words.first().is_some_and(joiner) {
        words.remove(0);
    }
    let base = words.join(" ").trim_matches(|c: char| c == '-' || c == ',' || c == '+' || c.is_whitespace()).to_owned();
    (!base.is_empty()).then_some(base)
}

pub struct Query {
    pub artist: String,
    pub title: String,
    pub duration_ms: u64,
    /// a changed version (slowed …): the original song's title
    pub version_of: Option<String>,
    /// how that version differs in tempo (from the track's own title, the one searched is cleaned of the mark)
    pub kind: VersionKind,
}

/// Text of the original song for a changed version. Its length differs, so
/// any length goes; the uploader is often not the artist ("ceeri" for CKay),
/// so a title-only search picks the artist the title is most often filed under.
async fn original_text(http: &HttpClient, state: &LyricsState, artist: &str, title: &str) -> AppResult<Option<Lyrics>> {
    let (artist, title) = match title.split_once(" - ") {
        Some((a, t)) => (a.trim().to_owned(), t.trim().to_owned()),
        None => (artist.to_owned(), title.to_owned()),
    };
    let q = Query { artist, title, duration_ms: 0, version_of: None, kind: VersionKind::Same };
    if let Ok(Some(l)) = lrclib(http, &q).await {
        return Ok(Some(l));
    }
    if let Ok(Some(l)) = deezer(http, state, &q).await {
        return Ok(Some(l));
    }
    if let Ok(Some(l)) = musixmatch(http, state, &q).await {
        return Ok(Some(l));
    }
    lrclib_most_common(http, &q.title).await
}

async fn lrclib_most_common(http: &HttpClient, title: &str) -> AppResult<Option<Lyrics>> {
    let want = normalize(title);
    if want.chars().count() < 4 {
        return Ok(None);
    }
    let mut url = Url::parse("https://lrclib.net/api/search")?;
    url.query_pairs_mut().append_pair("q", title);
    let hits: Value = http.get_json_with(url.as_str(), &[("lrclib-client", "SC Desk")]).await?;
    let hits: Vec<&Value> =
        hits.as_array().into_iter().flatten().filter(|h| normalize(h["trackName"].as_str().unwrap_or("")) == want).collect();
    let mut by_artist: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for h in &hits {
        *by_artist.entry(normalize(h["artistName"].as_str().unwrap_or(""))).or_default() += 1;
    }
    let Some((top, _)) = by_artist.into_iter().max_by_key(|(_, n)| *n) else { return Ok(None) };
    for h in hits.into_iter().filter(|h| normalize(h["artistName"].as_str().unwrap_or("")) == top) {
        let text = h["plainLyrics"].as_str().filter(|s| !s.trim().is_empty()).map(str::to_owned).or_else(|| {
            h["syncedLyrics"].as_str().map(|s| parse_lrc(s).into_iter().map(|l| l.text).collect::<Vec<_>>().join("\n"))
        });
        if let Some(l) = text.and_then(|t| Lyrics::new("lrclib", plain_lines(&t), None)) {
            return Ok(Some(l));
        }
    }
    Ok(None)
}

/// Musixmatch desktop token, reused between lookups.
#[derive(Default)]
pub struct LyricsState {
    /// "base|app_id|token": the token belongs to the endpoint that gave it
    mxm_token: Mutex<Option<String>>,
    /// Musixmatch hands out an all-zero token (2026-10: for this IP or for
    /// everyone): don't ask again before this
    mxm_off_until: Mutex<Option<std::time::Instant>>,
    /// Deezer anonymous session (JWT), reused until refused
    dz_jwt: Mutex<Option<String>>,
}

pub async fn find(http: &HttpClient, state: &LyricsState, q: &Query) -> AppResult<Option<Lyrics>> {
    find_counted(http, state, q).await.map(|(l, _)| l)
}

/// `find`, plus how many sources failed to answer: "nothing found" while a source was down
/// (LRCLIB overloaded, no Musixmatch token) is not a real answer and must not be kept.
#[allow(unused_assignments)] // the macro also assigns `best` after the last read
pub async fn find_counted(http: &HttpClient, state: &LyricsState, q: &Query) -> AppResult<(Option<Lyrics>, u32)> {
    let mut best: Option<Lyrics> = None;
    let mut plain: Option<Lyrics> = None;
    let mut errors = 0u32;

    macro_rules! consider {
        ($res:expr, $name:literal) => {
            match $res {
                Ok(Some(l)) if l.word_level => return Ok((Some(l), errors)),
                Ok(Some(l)) if l.synced => {
                    if best.is_none() {
                        best = Some(l);
                    }
                }
                Ok(Some(l)) => {
                    if plain.is_none() {
                        plain = Some(l);
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    // Genius often just skips (its check window is not opened in the background): not a failure
                    if !$name.starts_with("genius") {
                        errors += 1;
                    }
                    tracing::debug!(provider = $name, error = %e, "lyrics provider failed");
                }
            }
        };
    }

    // the four at once (the time of the slowest, not the sum of all), judged in the same order as before
    let (r_lrclib, r_deezer, r_netease, r_musixmatch) = futures::join!(lrclib(http, q), deezer(http, state, q), netease(http, q), musixmatch(http, state, q));
    consider!(r_lrclib, "lrclib");
    consider!(r_deezer, "deezer");
    consider!(r_netease, "netease");
    consider!(r_musixmatch, "musixmatch");
    if let Some(l) = best {
        return Ok((Some(l), errors));
    }
    if plain.is_none() {
        // covers / re-uploads: the uploader is not the artist the lyrics are filed under
        consider!(lrclib_by_title(http, q).await, "lrclib-title");
        if let Some(l) = best {
            return Ok((Some(l), errors));
        }
    }
    if plain.is_none() {
        // last: may need the Genius browser window
        consider!(genius_plain(http, q).await, "genius");
    }
    if plain.is_none() && best.is_none() && q.version_of.is_none() {
        // a re-upload under someone else's name ("Gabris - Дымок" is Ицык Цыпер's):
        // find who really recorded it by the title and the exact length, ask again
        for alias in real_artists(http, q).await {
            let q2 = Query { artist: alias.0, title: alias.1, duration_ms: q.duration_ms, version_of: None, kind: q.kind };
            tracing::debug!(artist = %q2.artist, title = %q2.title, "lyrics: trying the real artist");
            let (a, b, c, d) = futures::join!(lrclib(http, &q2), deezer(http, state, &q2), netease(http, &q2), musixmatch(http, state, &q2));
            consider!(a, "lrclib-real");
            consider!(b, "deezer-real");
            consider!(c, "netease-real");
            consider!(d, "musixmatch-real");
            if let Some(l) = best {
                return Ok((Some(l), errors));
            }
            if plain.is_none() {
                consider!(genius_plain(http, &q2).await, "genius-real");
            }
            if plain.is_some() {
                break;
            }
        }
    }
    if plain.is_none() {
        // slowed / sped up / nightcore …: the original's words, without timings
        if let Some(base) = q.version_of.as_deref() {
            match original_text(http, state, &q.artist, base).await {
                Ok(Some(l)) => return Ok((Some(l.with_tempo(q.duration_ms, q.kind)), errors)),
                Ok(None) => {}
                Err(e) => tracing::debug!(error = %e, "original lyrics failed"),
            }
        }
    }
    if plain.is_none() && errors >= 4 {
        return Err(AppError::Network("ни один источник текстов не ответил".into()));
    }
    Ok((plain, errors))
}

/// The same title in brackets-free form: "Дымок (Smoke)" = "Дымок".
fn bare_title(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0u32;
    for c in s.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    normalize(&out)
}

/// Who really recorded a track uploaded under another name: catalog entries with
/// the same title and the same length (±2 s), most trusted first (Deezer ranks
/// popular recordings first, then Yandex Music, then LRCLIB). At most two, and
/// never the uploader itself.
async fn real_artists(http: &HttpClient, q: &Query) -> Vec<(String, String)> {
    if q.duration_ms == 0 {
        return Vec::new();
    }
    let want = bare_title(&q.title);
    if want.is_empty() {
        return Vec::new();
    }
    let close = |secs: f64| (secs * 1000.0) as u64 >= 1 && q.duration_ms.abs_diff((secs * 1000.0) as u64) <= 2_000;

    let dz = async {
        let mut url = Url::parse("https://api.deezer.com/search").ok()?;
        url.query_pairs_mut().append_pair("q", &q.title);
        let v: Value = http.get_json_with(url.as_str(), &[]).await.ok()?;
        Some(
            v["data"]
                .as_array()?
                .iter()
                .filter(|t| close(t["duration"].as_f64().unwrap_or(0.0)))
                .filter_map(|t| Some((t["artist"]["name"].as_str()?.to_owned(), t["title"].as_str()?.to_owned())))
                .collect::<Vec<_>>(),
        )
    };
    let ya = async {
        let mut url = Url::parse("https://api.music.yandex.net/search").ok()?;
        url.query_pairs_mut().append_pair("type", "track").append_pair("page", "0").append_pair("text", &q.title);
        let v: Value = http.get_json_with(url.as_str(), &[]).await.ok()?;
        Some(
            v["result"]["tracks"]["results"]
                .as_array()?
                .iter()
                .filter(|t| close(t["durationMs"].as_f64().unwrap_or(0.0) / 1000.0))
                .filter_map(|t| {
                    let artist = t["artists"].as_array()?.iter().filter_map(|a| a["name"].as_str()).collect::<Vec<_>>().join(", ");
                    Some((artist, t["title"].as_str()?.to_owned()))
                })
                .collect::<Vec<_>>(),
        )
    };
    let lr = async {
        let mut url = Url::parse("https://lrclib.net/api/search").ok()?;
        url.query_pairs_mut().append_pair("q", &q.title);
        let v: Value = http.get_json_with(url.as_str(), &[("lrclib-client", "SC Desk")]).await.ok()?;
        Some(
            v.as_array()?
                .iter()
                .filter(|t| close(t["duration"].as_f64().unwrap_or(0.0)))
                .filter_map(|t| Some((t["artistName"].as_str()?.to_owned(), t["trackName"].as_str()?.to_owned())))
                .collect::<Vec<_>>(),
        )
    };
    let (dz, ya, lr) = futures::join!(dz, ya, lr);
    let uploader = normalize(&q.artist);
    let mut out: Vec<(String, String)> = Vec::new();
    for (artist, title) in [dz, ya, lr].into_iter().flatten().flatten() {
        let a = normalize(&artist);
        if bare_title(&title) == want && a != uploader && !out.iter().any(|(x, _)| normalize(x) == a) {
            out.push((artist, title));
        }
        if out.len() == 2 {
            break;
        }
    }
    out
}

fn duration_ok(want_ms: u64, got_ms: u64) -> bool {
    want_ms == 0 || got_ms == 0 || want_ms.abs_diff(got_ms) <= 6_000
}

fn names_match(q: &Query, artist: &str, title: &str) -> bool {
    match_score(&normalize(&q.artist), &normalize(&q.title), &normalize(artist), &normalize(title)) >= 3
}

// ------------------------------------------------------------------ LRCLIB

async fn lrclib(http: &HttpClient, q: &Query) -> AppResult<Option<Lyrics>> {
    let mut url = Url::parse("https://lrclib.net/api/search")?;
    url.query_pairs_mut().append_pair("track_name", &q.title).append_pair("artist_name", &q.artist);
    let hits: Value = http.get_json_with(url.as_str(), &[("lrclib-client", "SC Desk")]).await?;
    let hits = hits.as_array().cloned().unwrap_or_default();

    let dur = |h: &Value| (h["duration"].as_f64().unwrap_or(0.0) * 1000.0) as u64;
    let mut cands: Vec<&Value> = hits
        .iter()
        .filter(|h| duration_ok(q.duration_ms, dur(h)) && names_match(q, h["artistName"].as_str().unwrap_or(""), h["trackName"].as_str().unwrap_or("")))
        .collect();
    // the version made for the recording closest in length to ours goes first; with no length of ours
    // (the original of a sped-up edit) the one of the usual length, the middle of the timed ones
    let reference = if q.duration_ms > 0 {
        q.duration_ms
    } else {
        let mut lens: Vec<u64> = cands.iter().filter(|h| h["syncedLyrics"].as_str().is_some_and(|x| !x.trim().is_empty())).map(|h| dur(h)).filter(|d| *d > 0).collect();
        lens.sort_unstable();
        lens.get(lens.len() / 2).copied().unwrap_or(0)
    };
    cands.sort_by_key(|h| if reference == 0 || dur(h) == 0 { u64::MAX / 2 } else { reference.abs_diff(dur(h)) });

    let mut plain = None;
    for h in cands {
        if let Some(lrc) = h["syncedLyrics"].as_str().filter(|s| !s.trim().is_empty()) {
            if let Some(mut l) = Lyrics::new("lrclib", parse_lrc(lrc), None) {
                let off = if q.duration_ms > 0 && dur(h) > 0 { q.duration_ms.abs_diff(dur(h)) } else { 0 };
                if off > WARN_OFF_MS {
                    l.length_off_ms = Some(off);
                }
                l.source_duration_ms = (dur(h) > 0).then(|| dur(h));
                return Ok(Some(l));
            }
        }
        if plain.is_none() {
            if let Some(text) = h["plainLyrics"].as_str() {
                plain = Lyrics::new("lrclib", plain_lines(text), None);
            }
        }
    }
    Ok(plain)
}

/// Title long enough that an exact match is the same song, not a namesake:
/// "Дым сигарет с ментолом" yes, "Sea Angel" / "TATE" no.
fn distinctive_title(title: &str) -> bool {
    title.split_whitespace().count() >= 3 && normalize(title).chars().count() >= 12
}

/// Last resort for covers and re-uploads: search by the title alone and take
/// an exact title match. Timings are kept only if the length matches closely
/// (same recording), otherwise the text is shown without karaoke.
async fn lrclib_by_title(http: &HttpClient, q: &Query) -> AppResult<Option<Lyrics>> {
    if !distinctive_title(&q.title) {
        return Ok(None);
    }
    let mut url = Url::parse("https://lrclib.net/api/search")?;
    url.query_pairs_mut().append_pair("q", &q.title);
    let hits: Value = http.get_json_with(url.as_str(), &[("lrclib-client", "SC Desk")]).await?;
    let want = normalize(&q.title);
    let want_artist = normalize(&q.artist);
    let mut hits: Vec<&Value> = hits
        .as_array()
        .into_iter()
        .flatten()
        .filter(|h| normalize(h["trackName"].as_str().unwrap_or("")) == want)
        .collect();
    // the right artist first, then the closest length
    hits.sort_by_key(|h| {
        let artist_ok = normalize(h["artistName"].as_str().unwrap_or("")) == want_artist;
        let d = (h["duration"].as_f64().unwrap_or(0.0) * 1000.0) as u64;
        (!artist_ok, d.abs_diff(q.duration_ms))
    });
    for h in hits {
        let d = (h["duration"].as_f64().unwrap_or(0.0) * 1000.0) as u64;
        let same_recording = q.duration_ms > 0 && d > 0 && q.duration_ms.abs_diff(d) <= 3_000;
        if same_recording {
            if let Some(l) = h["syncedLyrics"].as_str().and_then(|s| Lyrics::new("lrclib", parse_lrc(s), None)) {
                return Ok(Some(l));
            }
        }
        let text = h["plainLyrics"].as_str().map(str::to_owned).or_else(|| {
            // synced only: drop the timestamps
            h["syncedLyrics"].as_str().map(|s| parse_lrc(s).into_iter().map(|l| l.text).collect::<Vec<_>>().join("\n"))
        });
        if let Some(l) = text.and_then(|t| Lyrics::new("lrclib", plain_lines(&t), None)) {
            return Ok(Some(l));
        }
    }
    Ok(None)
}

// ------------------------------------------------------------------ Deezer

const DZ_PIPE: &str = "https://pipe.deezer.com/api";
const DZ_LYRICS: &str = "query L($id: String!) { track(trackId: $id) { lyrics { text \
    synchronizedLines { line milliseconds duration } \
    synchronizedWordByWordLines { start end words { start end word } } } } }";

async fn dz_jwt(http: &HttpClient, state: &LyricsState, fresh: bool) -> AppResult<String> {
    if !fresh {
        if let Some(t) = state.dz_jwt.lock().unwrap_or_else(|p| p.into_inner()).clone() {
            return Ok(t);
        }
    }
    let v: Value = http.get_json_with("https://auth.deezer.com/login/anonymous?jo=p&rto=c", &[]).await?;
    let jwt = v["jwt"].as_str().filter(|j| !j.is_empty()).ok_or_else(|| AppError::Network("deezer: no session".into()))?.to_owned();
    *state.dz_jwt.lock().unwrap_or_else(|p| p.into_inner()) = Some(jwt.clone());
    Ok(jwt)
}

async fn deezer(http: &HttpClient, state: &LyricsState, q: &Query) -> AppResult<Option<Lyrics>> {
    // the track: strict search first, then a free one
    let mut found = None;
    for query in [format!("artist:\"{}\" track:\"{}\"", q.artist, q.title), format!("{} {}", q.artist, q.title)] {
        let mut url = Url::parse("https://api.deezer.com/search")?;
        url.query_pairs_mut().append_pair("q", &query).append_pair("limit", "10");
        let res: Value = http.get_json_with(url.as_str(), &[]).await?;
        found = res["data"].as_array().into_iter().flatten().find(|t| {
            duration_ok(q.duration_ms, t["duration"].as_u64().unwrap_or(0) * 1000)
                && names_match(q, t["artist"]["name"].as_str().unwrap_or(""), t["title"].as_str().unwrap_or(""))
        }).and_then(|t| t["id"].as_u64());
        if found.is_some() {
            break;
        }
    }
    let Some(id) = found else { return Ok(None) };
    let body = serde_json::json!({ "operationName": "L", "variables": { "id": id.to_string() }, "query": DZ_LYRICS });
    let mut res = Value::Null;
    for fresh in [false, true] {
        let jwt = dz_jwt(http, state, fresh).await?;
        let auth = format!("Bearer {jwt}");
        match http.post_json(DZ_PIPE, &[("authorization", auth.as_str())], &body).await {
            Ok(v) if v["errors"].as_array().is_some_and(|e| e.iter().any(|e| e["type"].as_str().unwrap_or("").contains("Auth"))) && !fresh => continue,
            Ok(v) => {
                res = v;
                break;
            }
            Err(AppError::AuthExpired | AppError::Forbidden) if !fresh => continue,
            Err(e) => return Err(e),
        }
    }
    let lyr = &res["data"]["track"]["lyrics"];
    // word by word when Deezer has it
    let words: Vec<Line> = lyr["synchronizedWordByWordLines"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|l| {
            let words: Vec<Word> = l["words"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|w| Word {
                    start_ms: w["start"].as_u64().unwrap_or(0),
                    end_ms: w["end"].as_u64().unwrap_or(0),
                    text: format!("{} ", w["word"].as_str().unwrap_or("").trim_end()),
                })
                .collect();
            Line {
                start_ms: l["start"].as_u64(),
                end_ms: l["end"].as_u64(),
                text: words.iter().map(|w| w.text.as_str()).collect::<String>().trim_end().to_owned(),
                words,
            }
        })
        .collect();
    if let Some(l) = Lyrics::new("deezer", words, None).filter(|l| l.word_level) {
        return Ok(Some(l));
    }
    let lines: Vec<Line> = lyr["synchronizedLines"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|l| {
            let start = l["milliseconds"].as_u64();
            Line {
                start_ms: start,
                end_ms: start.zip(l["duration"].as_u64()).map(|(s, d)| s + d),
                text: l["line"].as_str().unwrap_or("").to_owned(),
                words: Vec::new(),
            }
        })
        .collect();
    if let Some(l) = Lyrics::new("deezer", lines, None).filter(|l| l.synced) {
        return Ok(Some(l));
    }
    Ok(lyr["text"].as_str().and_then(|t| Lyrics::new("deezer", plain_lines(t), None)))
}

// ----------------------------------------------------------------- NetEase

const NETEASE_REFERER: &str = "https://music.163.com/";

/// Songs whose lyrics hold these words, by NetEase's search inside the texts (type 1006): (artist, title, length in ms).
/// It does not limit tokens the way Musixmatch does, so it answers when that one has stopped.
pub async fn netease_by_lyrics(http: &HttpClient, words: &str) -> AppResult<Vec<(String, String, u64)>> {
    let mut url = Url::parse("https://music.163.com/api/search/get")?;
    url.query_pairs_mut().append_pair("s", words).append_pair("type", "1006").append_pair("offset", "0").append_pair("limit", "10");
    let res: Value = http.get_json_with(url.as_str(), &[("referer", NETEASE_REFERER)]).await?;
    Ok(res["result"]["songs"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|s| {
                    let artist = s["artists"].as_array()?.first()?["name"].as_str()?;
                    Some((artist.to_owned(), s["name"].as_str()?.to_owned(), s["duration"].as_u64().unwrap_or(0)))
                })
                .collect()
        })
        .unwrap_or_default())
}

async fn netease(http: &HttpClient, q: &Query) -> AppResult<Option<Lyrics>> {
    let mut url = Url::parse("https://music.163.com/api/search/get/web")?;
    url.query_pairs_mut()
        .append_pair("s", &format!("{} {}", q.artist, q.title))
        .append_pair("type", "1")
        .append_pair("offset", "0")
        .append_pair("limit", "10");
    let res: Value = http.get_json_with(url.as_str(), &[("referer", NETEASE_REFERER)]).await?;
    let songs = res["result"]["songs"].as_array().cloned().unwrap_or_default();
    let Some(song) = songs.iter().find(|s| {
        let artist = s["artists"].as_array().and_then(|a| a.first()).and_then(|a| a["name"].as_str()).unwrap_or("");
        duration_ok(q.duration_ms, s["duration"].as_u64().unwrap_or(0))
            && names_match(q, artist, s["name"].as_str().unwrap_or(""))
    }) else {
        return Ok(None);
    };
    let id = song["id"].as_u64().ok_or_else(|| AppError::Parse("netease id".into()))?;

    let lyr: Value = http
        .get_json_with(
            &format!("https://music.163.com/api/song/lyric/v1?id={id}&lv=-1&kv=-1&tv=-1&yv=-1&rv=-1"),
            &[("referer", NETEASE_REFERER)],
        )
        .await?;
    if let Some(yrc) = lyr["yrc"]["lyric"].as_str().filter(|s| !s.trim().is_empty()) {
        let lines = parse_yrc(yrc);
        if let Some(l) = Lyrics::new("netease", lines, None).filter(|l| l.word_level) {
            return Ok(Some(l));
        }
    }
    let lrc = lyr["lrc"]["lyric"].as_str().unwrap_or("");
    Ok(Lyrics::new("netease", parse_lrc(lrc), None).filter(|l| l.synced))
}

// -------------------------------------------------------------- Musixmatch

const MXM_COOKIE: &str = "AWSELBCORS=0; AWSELB=0";
/// Where a user token can come from, tried in order. Musixmatch limits token
/// requests per address: over the limit the desktop endpoint answers with an
/// all-zero token and the mobile one with "captcha". A token, once given, is
/// kept (also across launches, see `session` / `restore`) instead of asking
/// for a new one on every start.
const MXM_ENDPOINTS: &[(&str, &str)] = &[
    ("https://apic-desktop.musixmatch.com/ws/1.1/", "web-desktop-app-v1.0"),
    ("https://apic-appmobile.musixmatch.com/ws/1.1/", "android-player-v1.0"),
];

#[derive(Clone)]
struct MxmSession {
    base: String,
    app: String,
    token: String,
}

impl LyricsState {
    /// The Musixmatch session to keep between launches.
    pub fn mxm_session(&self) -> Option<String> {
        self.mxm_token.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn mxm_restore(&self, saved: &str) {
        if saved.split('|').count() == 3 {
            *self.mxm_token.lock().unwrap_or_else(|p| p.into_inner()) = Some(saved.to_owned());
        }
    }
}

fn mxm_parse(s: &str) -> Option<MxmSession> {
    let mut it = s.split('|');
    Some(MxmSession { base: it.next()?.into(), app: it.next()?.into(), token: it.next()?.into() })
}

async fn mxm_token(http: &HttpClient, state: &LyricsState) -> AppResult<MxmSession> {
    if let Some(s) = state.mxm_token.lock().unwrap_or_else(|p| p.into_inner()).as_deref().and_then(mxm_parse) {
        return Ok(s);
    }
    if state.mxm_off_until.lock().unwrap_or_else(|p| p.into_inner()).is_some_and(|u| std::time::Instant::now() < u) {
        return Err(AppError::Network("musixmatch: недоступен, повтор позже".into()));
    }
    for (base, app) in MXM_ENDPOINTS {
        let url = format!("{base}token.get?app_id={app}&user_language=en&format=json&t={}", rand::random::<u32>());
        let Ok(res) = http.get_json_with::<Value>(&url, &[("cookie", MXM_COOKIE)]).await else { continue };
        let token = res["message"]["body"]["user_token"].as_str().unwrap_or("").to_owned();
        // an all-zero token looks valid but every lookup with it finds nothing
        if token.is_empty() || token.starts_with("UpgradeOnly") || token.chars().all(|c| c == '0') {
            tracing::debug!(endpoint = base, hint = res["message"]["header"]["hint"].as_str().unwrap_or(""), "musixmatch: no token here");
            continue;
        }
        tracing::info!(endpoint = base, "musixmatch: token received");
        let s = MxmSession { base: (*base).into(), app: (*app).into(), token };
        *state.mxm_token.lock().unwrap_or_else(|p| p.into_inner()) = Some(format!("{}|{}|{}", s.base, s.app, s.token));
        return Ok(s);
    }
    tracing::info!("musixmatch: no working token, trying again in 30 min");
    *state.mxm_off_until.lock().unwrap_or_else(|p| p.into_inner()) =
        Some(std::time::Instant::now() + std::time::Duration::from_secs(30 * 60));
    Err(AppError::Network("musixmatch: токен не выдан".into()))
}

/// Songs whose lyrics contain these words (Musixmatch searches inside the texts): (artist, title, length in ms).
pub async fn search_by_lyrics(http: &HttpClient, state: &LyricsState, words: &str) -> AppResult<Vec<(String, String, u64)>> {
    let MxmSession { base, app, token } = mxm_token(http, state).await?;
    let mut url = Url::parse(&format!("{base}track.search"))?;
    url.query_pairs_mut()
        .append_pair("format", "json")
        .append_pair("app_id", &app)
        .append_pair("q_lyrics", words)
        .append_pair("f_has_lyrics", "1")
        .append_pair("page_size", "12")
        .append_pair("usertoken", &token);
    let res: Value = http.get_json_with(url.as_str(), &[("cookie", MXM_COOKIE)]).await?;
    if res["message"]["header"]["status_code"].as_u64() == Some(401) {
        *state.mxm_token.lock().unwrap_or_else(|p| p.into_inner()) = None;
        return Err(AppError::Network("musixmatch 401".into()));
    }
    Ok(res["message"]["body"]["track_list"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|t| {
                    let t = &t["track"];
                    Some((t["artist_name"].as_str()?.to_owned(), t["track_name"].as_str()?.to_owned(), t["track_length"].as_u64().unwrap_or(0) * 1000))
                })
                .collect()
        })
        .unwrap_or_default())
}

async fn musixmatch(http: &HttpClient, state: &LyricsState, q: &Query) -> AppResult<Option<Lyrics>> {
    let MxmSession { base, app, token } = mxm_token(http, state).await?;
    let mut url = Url::parse(&format!("{base}macro.subtitles.get"))?;
    url.query_pairs_mut()
        .append_pair("format", "json")
        .append_pair("namespace", "lyrics_richsynched")
        .append_pair("subtitle_format", "mxm")
        .append_pair("app_id", &app)
        .append_pair("q_artist", &q.artist)
        .append_pair("q_track", &q.title)
        .append_pair("q_duration", &(q.duration_ms / 1000).to_string())
        .append_pair("usertoken", &token);
    let res: Value = http.get_json_with(url.as_str(), &[("cookie", MXM_COOKIE)]).await?;
    if res["message"]["header"]["status_code"].as_u64() == Some(401) {
        // token expired / captcha: forget it, next lookup fetches a new one
        *state.mxm_token.lock().unwrap_or_else(|p| p.into_inner()) = None;
        return Err(AppError::Network("musixmatch 401".into()));
    }
    let calls = &res["message"]["body"]["macro_calls"];
    let track = &calls["matcher.track.get"]["message"]["body"]["track"];
    if track.is_null()
        || !names_match(q, track["artist_name"].as_str().unwrap_or(""), track["track_name"].as_str().unwrap_or(""))
    {
        return Ok(None);
    }

    if track["has_richsync"].as_u64() == Some(1) {
        if let Some(id) = track["commontrack_id"].as_u64() {
            let url = format!(
                "{base}track.richsync.get?format=json&subtitle_format=mxm&app_id={app}&commontrack_id={id}&usertoken={token}"
            );
            if let Ok(rs) = http.get_json_with::<Value>(&url, &[("cookie", MXM_COOKIE)]).await {
                if let Some(body) = rs["message"]["body"]["richsync"]["richsync_body"].as_str() {
                    if let Some(l) = Lyrics::new("musixmatch", parse_richsync(body), None).filter(|l| l.word_level) {
                        return Ok(Some(l));
                    }
                }
            }
        }
    }

    let body = calls["track.subtitles.get"]["message"]["body"]["subtitle_list"][0]["subtitle"]["subtitle_body"]
        .as_str()
        .unwrap_or("");
    let lines: Vec<Line> = serde_json::from_str::<Vec<Value>>(body)
        .unwrap_or_default()
        .iter()
        .map(|v| Line {
            start_ms: v["time"]["total"].as_f64().map(|s| (s * 1000.0) as u64),
            end_ms: None,
            text: v["text"].as_str().unwrap_or("").to_owned(),
            words: Vec::new(),
        })
        .collect();
    Ok(Lyrics::new("musixmatch", fill_ends(lines), None).filter(|l| l.synced))
}

// ------------------------------------------------------------------ Genius

async fn genius_plain(http: &HttpClient, q: &Query) -> AppResult<Option<Lyrics>> {
    // through a browser page if Genius blocks plain requests (Cloudflare check)
    let hit = match crate::geniusweb::search(http, &q.artist, &q.title).await? {
        Some(hit) => Some(hit),
        // the uploader is often not the artist: a long, distinctive title alone
        // is the same song ("А девочке твоей всего 15 лет"), a short one is not
        None if distinctive_title(&q.title) => crate::geniusweb::search_by_title(http, &q.title).await?,
        None => None,
    };
    let Some(hit) = hit else {
        return Ok(None);
    };
    let text = crate::geniusweb::lyrics(http, &hit.url).await?;
    Ok(Lyrics::new("genius", plain_lines(&text), Some(hit.url)))
}

// ----------------------------------------------------------------- parsers

fn plain_lines(text: &str) -> Vec<Line> {
    text.lines().map(|t| Line { start_ms: None, end_ms: None, text: t.trim().to_owned(), words: Vec::new() }).collect()
}

/// `mm:ss.xx`, `mm:ss.xxx`, `mm:ss`, `mm:ss:xx` → ms
fn parse_ts(s: &str) -> Option<u64> {
    let s = s.trim();
    let (min, rest) = s.split_once(':')?;
    let min: u64 = min.parse().ok()?;
    let (sec, frac) = match rest.split_once(['.', ':']) {
        Some((a, b)) => (a, b),
        None => (rest, ""),
    };
    let sec: u64 = sec.parse().ok()?;
    let frac_ms = if frac.is_empty() {
        0
    } else {
        let digits: String = frac.chars().take(3).collect();
        let v: u64 = digits.parse().ok()?;
        match digits.len() {
            1 => v * 100,
            2 => v * 10,
            _ => v,
        }
    };
    Some(min * 60_000 + sec * 1000 + frac_ms)
}

/// Standard LRC plus "enhanced" word tags: `[00:12.30]<00:12.30>Hello <00:12.80>world`
pub fn parse_lrc(text: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut stamps = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(end) = stripped.find(']') else { break };
            match parse_ts(&stripped[..end]) {
                Some(ms) => stamps.push(ms),
                None => {
                    stamps.clear(); // metadata tag like [ar:...]
                    rest = "";
                    break;
                }
            }
            rest = &stripped[end + 1..];
        }
        if stamps.is_empty() {
            continue;
        }
        let (text, words) = parse_enhanced(rest);
        for ms in stamps {
            lines.push(Line { start_ms: Some(ms), end_ms: None, text: text.clone(), words: words.clone() });
        }
    }
    lines.sort_by_key(|l| l.start_ms);
    fill_ends(lines)
}

fn parse_enhanced(s: &str) -> (String, Vec<Word>) {
    if !s.contains('<') {
        return (s.trim().to_owned(), Vec::new());
    }
    let mut words: Vec<Word> = Vec::new();
    let mut text = String::new();
    for part in s.split('<').filter(|p| !p.is_empty()) {
        let Some((ts, w)) = part.split_once('>') else {
            text.push_str(part);
            continue;
        };
        let Some(ms) = parse_ts(ts) else { continue };
        if let Some(prev) = words.last_mut() {
            prev.end_ms = ms;
        }
        text.push_str(w);
        if !w.trim().is_empty() {
            words.push(Word { start_ms: ms, end_ms: ms, text: w.to_owned() });
        }
    }
    (text.trim().to_owned(), words)
}

/// NetEase YRC: `[16210,3460](16210,670,0)Some (16880,410,0)words`
pub fn parse_yrc(text: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    for raw in text.lines() {
        let raw = raw.trim();
        let Some(rest) = raw.strip_prefix('[') else { continue };
        let Some((head, body)) = rest.split_once(']') else { continue };
        let mut it = head.split(',');
        let (Some(start), Some(dur)) = (it.next().and_then(|v| v.parse::<u64>().ok()), it.next().and_then(|v| v.parse::<u64>().ok()))
        else {
            continue;
        };
        let mut words = Vec::new();
        let mut text = String::new();
        for part in body.split('(').filter(|p| !p.is_empty()) {
            let Some((nums, w)) = part.split_once(')') else { continue };
            let mut n = nums.split(',');
            let (Some(ws), Some(wd)) = (n.next().and_then(|v| v.parse::<u64>().ok()), n.next().and_then(|v| v.parse::<u64>().ok()))
            else {
                text.push_str(part);
                continue;
            };
            text.push_str(w);
            if !w.trim().is_empty() {
                words.push(Word { start_ms: ws, end_ms: ws + wd, text: w.to_owned() });
            }
        }
        lines.push(Line { start_ms: Some(start), end_ms: Some(start + dur), text: text.trim().to_owned(), words });
    }
    lines
}

/// Musixmatch richsync: `[{"ts":12.3,"te":15.1,"l":[{"c":"Hello","o":0.0},{"c":" ","o":0.4}],"x":"Hello world"}]`
pub fn parse_richsync(body: &str) -> Vec<Line> {
    let items: Vec<Value> = serde_json::from_str(body).unwrap_or_default();
    items
        .iter()
        .filter_map(|it| {
            let ts = it["ts"].as_f64()?;
            let te = it["te"].as_f64().unwrap_or(ts);
            let parts = it["l"].as_array().cloned().unwrap_or_default();
            let mut words: Vec<Word> = Vec::new();
            for p in &parts {
                let c = p["c"].as_str().unwrap_or("");
                let start = ((ts + p["o"].as_f64().unwrap_or(0.0)) * 1000.0) as u64;
                if let Some(prev) = words.last_mut() {
                    prev.end_ms = start;
                }
                if !c.trim().is_empty() {
                    words.push(Word { start_ms: start, end_ms: start, text: c.to_owned() });
                } else if let Some(prev) = words.last_mut() {
                    prev.text.push_str(c);
                }
            }
            if let Some(last) = words.last_mut() {
                last.end_ms = (te * 1000.0) as u64;
            }
            Some(Line {
                start_ms: Some((ts * 1000.0) as u64),
                end_ms: Some((te * 1000.0) as u64),
                text: it["x"].as_str().unwrap_or("").to_owned(),
                words,
            })
        })
        .collect()
}

fn fill_ends(mut lines: Vec<Line>) -> Vec<Line> {
    for i in 0..lines.len() {
        let next = lines.get(i + 1).and_then(|l| l.start_ms);
        let line = &mut lines[i];
        if line.end_ms.is_none() {
            line.end_ms = next.or(line.start_ms.map(|s| s + 5_000));
        }
        let end = line.end_ms;
        if let (Some(last), Some(end)) = (line.words.last_mut(), end) {
            if last.end_ms <= last.start_ms {
                last.end_ms = end;
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::{bare_title, distinctive_title, version_base};

    #[test]
    fn bare_titles() {
        assert_eq!(bare_title("Дымок (Smoke)"), bare_title("Дымок"));
        assert_eq!(bare_title("Life [prod. X]"), bare_title("Life"));
    }

    /// Real network: `cargo test real_artist_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn real_artist_live() {
        let http = crate::net::HttpClient::new(&crate::config::AppConfig::default()).unwrap();
        let state = super::LyricsState::default();
        let _ = tracing_subscriber::fmt().with_env_filter("sc_desk=debug").with_test_writer().try_init();
        let (artist, title, ms) = match std::env::var("LY_Q") {
            Ok(v) => { let p: Vec<&str> = v.split('|').collect(); (p[0].to_owned(), p[1].to_owned(), p[2].parse().unwrap()) }
            Err(_) => ("Gabris".to_owned(), "Дымок".to_owned(), 139_000),
        };
        let q = super::Query { kind: super::version_kind(&title), artist, title, duration_ms: ms, version_of: None };
        println!("real artists: {:?}", super::real_artists(&http, &q).await);
        let found = super::find(&http, &state, &q).await.unwrap();
        let l = found.expect("lyrics");
        println!("source {} synced {} first: {:?}", l.source, l.synced, l.lines.iter().find(|x| !x.text.trim().is_empty()).map(|x| &x.text));
    }

    fn timed(src: u64) -> Lyrics {
        let line = |start: u64, end: u64, text: &str| Line { start_ms: Some(start), end_ms: Some(end), text: text.into(), words: vec![Word { start_ms: start, end_ms: end, text: text.into() }] };
        let mut l = Lyrics::new("lrclib", vec![line(10_000, 14_000, "a"), line(14_000, 20_000, "b")], None).unwrap();
        l.source_duration_ms = Some(src);
        l
    }

    #[test]
    fn a_sped_up_version_gets_the_originals_timings_squeezed_by_the_length_ratio() {
        // ours is 150 s, the original 200 s: everything happens at 0.75 of the time
        let l = timed(200_000).with_tempo(150_000, version_kind("Song (Sped Up)"));
        assert!(l.synced && l.original);
        assert_eq!(l.lines[0].start_ms, Some(7_500));
        assert_eq!(l.lines[1].end_ms, Some(15_000));
        assert_eq!(l.lines[0].words[0].end_ms, 10_500);
        assert!((l.scaled_by.unwrap() - 0.75).abs() < 1e-6);
    }

    #[test]
    fn a_slowed_version_is_stretched() {
        let l = timed(100_000).with_tempo(125_000, version_kind("song - slowed + reverb"));
        assert_eq!(l.lines[0].start_ms, Some(12_500));
    }

    #[test]
    fn a_ratio_that_does_not_fit_the_kind_leaves_only_the_text() {
        // "slowed" but shorter than the original: an edit with a cut, not a slowdown
        assert!(!timed(200_000).with_tempo(150_000, version_kind("song slowed")).synced);
        // no length known
        let mut l = timed(200_000);
        l.source_duration_ms = None;
        assert!(!l.with_tempo(150_000, VersionKind::Sped).synced);
        // "reverb" at another length is another edit
        assert!(!timed(200_000).with_tempo(150_000, version_kind("song reverb")).synced);
    }

    #[test]
    fn changed_versions_lose_their_marks() {
        assert_eq!(version_base("love nwantiti slowed").as_deref(), Some("love nwantiti"));
        assert_eq!(version_base("Song (Slowed + Reverb)").as_deref(), Some("Song"));
        assert_eq!(version_base("Song - sped up").as_deref(), Some("Song"));
        assert_eq!(version_base("Artist - Song [Nightcore]").as_deref(), Some("Artist - Song"));
        assert_eq!(version_base("Кукла колдуна замедленная").as_deref(), Some("Кукла колдуна"));
        assert_eq!(version_base("Song (Remix)"), None);
        assert_eq!(version_base("Slowdive"), None);
        assert_eq!(version_base("Reverberation"), None);
    }

    #[test]
    fn only_long_titles_match_by_title_alone() {
        assert!(distinctive_title("Дым сигарет с ментолом"));
        assert!(distinctive_title("Дотянуться до солнца"));
        assert!(!distinctive_title("Sea Angel"));
        assert!(!distinctive_title("TATE"));
    }

    use super::*;

    #[test]
    fn parses_plain_and_enhanced_lrc() {
        let lrc = "[ar:Someone]\n[00:01.50]First line\n[00:04.00]<00:04.00>Hello <00:04.60>world\n[00:07.25]";
        let l = parse_lrc(lrc);
        assert_eq!(l.len(), 3);
        assert_eq!(l[0].start_ms, Some(1500));
        assert_eq!(l[0].end_ms, Some(4000));
        assert_eq!(l[1].text, "Hello world");
        assert_eq!(l[1].words.len(), 2);
        assert_eq!(l[1].words[0].end_ms, 4600);
        assert_eq!(l[1].words[1].end_ms, 7250);
    }

    #[test]
    fn parses_yrc() {
        let l = parse_yrc("{\"t\":0}\n[16210,3460](16210,670,0)Some (16880,410,0)words");
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].text, "Some words");
        assert_eq!(l[0].words[1].start_ms, 16880);
        assert_eq!(l[0].end_ms, Some(19670));
    }

    #[test]
    fn parses_richsync() {
        let body = r#"[{"ts":1.0,"te":3.0,"l":[{"c":"Hi","o":0},{"c":" ","o":0.5},{"c":"there","o":1.0}],"x":"Hi there"}]"#;
        let l = parse_richsync(body);
        assert_eq!(l[0].words.len(), 2);
        assert_eq!(l[0].words[0].text, "Hi ");
        assert_eq!(l[0].words[1].start_ms, 2000);
        assert_eq!(l[0].words[1].end_ms, 3000);
    }

    #[test]
    fn timestamps() {
        assert_eq!(parse_ts("01:02.5"), Some(62_500));
        assert_eq!(parse_ts("00:10.123"), Some(10_123));
        assert_eq!(parse_ts("ar:x"), None);
    }
}
