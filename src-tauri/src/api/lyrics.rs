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
}

impl Lyrics {
    fn new(source: &str, lines: Vec<Line>, url: Option<String>) -> Option<Self> {
        let lines: Vec<Line> = lines.into_iter().filter(|l| !(l.text.trim().is_empty() && l.start_ms.is_none())).collect();
        if lines.iter().all(|l| l.text.trim().is_empty()) {
            return None;
        }
        let synced = lines.iter().any(|l| l.start_ms.is_some());
        let word_level = lines.iter().any(|l| !l.words.is_empty());
        Some(Self { source: source.into(), synced, word_level, lines, url, original: false })
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
}

/// Text of the original song for a changed version. Its length differs, so
/// any length goes; the uploader is often not the artist ("ceeri" for CKay),
/// so a title-only search picks the artist the title is most often filed under.
async fn original_text(http: &HttpClient, state: &LyricsState, artist: &str, title: &str) -> AppResult<Option<Lyrics>> {
    let (artist, title) = match title.split_once(" - ") {
        Some((a, t)) => (a.trim().to_owned(), t.trim().to_owned()),
        None => (artist.to_owned(), title.to_owned()),
    };
    let q = Query { artist, title, duration_ms: 0, version_of: None };
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

#[allow(unused_assignments)] // the macro also assigns `best` after the last read
pub async fn find(http: &HttpClient, state: &LyricsState, q: &Query) -> AppResult<Option<Lyrics>> {
    let mut best: Option<Lyrics> = None;
    let mut plain: Option<Lyrics> = None;
    let mut errors = 0;

    macro_rules! consider {
        ($res:expr, $name:literal) => {
            match $res {
                Ok(Some(l)) if l.word_level => return Ok(Some(l)),
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
                    errors += 1;
                    tracing::debug!(provider = $name, error = %e, "lyrics provider failed");
                }
            }
        };
    }

    consider!(lrclib(http, q).await, "lrclib");
    consider!(deezer(http, state, q).await, "deezer");
    consider!(netease(http, q).await, "netease");
    consider!(musixmatch(http, state, q).await, "musixmatch");
    if let Some(l) = best {
        return Ok(Some(l));
    }
    if plain.is_none() {
        // covers / re-uploads: the uploader is not the artist the lyrics are filed under
        consider!(lrclib_by_title(http, q).await, "lrclib-title");
        if let Some(l) = best {
            return Ok(Some(l));
        }
    }
    if plain.is_none() {
        // last: may need the Genius browser window
        consider!(genius_plain(http, q).await, "genius");
    }
    if plain.is_none() {
        // slowed / sped up / nightcore …: the original's words, without timings
        if let Some(base) = q.version_of.as_deref() {
            match original_text(http, state, &q.artist, base).await {
                Ok(Some(l)) => return Ok(Some(l.as_original())),
                Ok(None) => {}
                Err(e) => tracing::debug!(error = %e, "original lyrics failed"),
            }
        }
    }
    if plain.is_none() && errors >= 4 {
        return Err(AppError::Network("ни один источник текстов не ответил".into()));
    }
    Ok(plain)
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

    let mut plain = None;
    for h in hits.iter().filter(|h| {
        let d = (h["duration"].as_f64().unwrap_or(0.0) * 1000.0) as u64;
        duration_ok(q.duration_ms, d)
            && names_match(q, h["artistName"].as_str().unwrap_or(""), h["trackName"].as_str().unwrap_or(""))
    }) {
        if let Some(lrc) = h["syncedLyrics"].as_str().filter(|s| !s.trim().is_empty()) {
            if let Some(l) = Lyrics::new("lrclib", parse_lrc(lrc), None) {
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
    let Some(hit) = crate::geniusweb::search(http, &q.artist, &q.title).await? else {
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
    use super::{distinctive_title, version_base};

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
