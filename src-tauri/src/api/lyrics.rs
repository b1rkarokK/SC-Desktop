//! Synced lyrics: LRCLIB → NetEase → Musixmatch, plain Genius as last resort.
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
}

impl Lyrics {
    fn new(source: &str, lines: Vec<Line>, url: Option<String>) -> Option<Self> {
        let lines: Vec<Line> = lines.into_iter().filter(|l| !(l.text.trim().is_empty() && l.start_ms.is_none())).collect();
        if lines.iter().all(|l| l.text.trim().is_empty()) {
            return None;
        }
        let synced = lines.iter().any(|l| l.start_ms.is_some());
        let word_level = lines.iter().any(|l| !l.words.is_empty());
        Some(Self { source: source.into(), synced, word_level, lines, url })
    }
}

pub struct Query {
    pub artist: String,
    pub title: String,
    pub duration_ms: u64,
}

/// Musixmatch desktop token, reused between lookups.
#[derive(Default)]
pub struct LyricsState {
    mxm_token: Mutex<Option<String>>,
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

const MXM_BASE: &str = "https://apic-desktop.musixmatch.com/ws/1.1/";
const MXM_APP: &str = "web-desktop-app-v1.0";
const MXM_COOKIE: &str = "AWSELBCORS=0; AWSELB=0";

async fn mxm_token(http: &HttpClient, state: &LyricsState) -> AppResult<String> {
    if let Some(t) = state.mxm_token.lock().unwrap_or_else(|p| p.into_inner()).clone() {
        return Ok(t);
    }
    let url = format!("{MXM_BASE}token.get?app_id={MXM_APP}&user_language=en&t={}", rand::random::<u32>());
    let res: Value = http.get_json_with(&url, &[("cookie", MXM_COOKIE)]).await?;
    let token = res["message"]["body"]["user_token"].as_str().unwrap_or("").to_owned();
    if token.is_empty() || token.starts_with("UpgradeOnly") {
        return Err(AppError::Network("musixmatch: токен не выдан".into()));
    }
    *state.mxm_token.lock().unwrap_or_else(|p| p.into_inner()) = Some(token.clone());
    Ok(token)
}

async fn musixmatch(http: &HttpClient, state: &LyricsState, q: &Query) -> AppResult<Option<Lyrics>> {
    let token = mxm_token(http, state).await?;
    let mut url = Url::parse(&format!("{MXM_BASE}macro.subtitles.get"))?;
    url.query_pairs_mut()
        .append_pair("format", "json")
        .append_pair("namespace", "lyrics_richsynched")
        .append_pair("subtitle_format", "mxm")
        .append_pair("app_id", MXM_APP)
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
                "{MXM_BASE}track.richsync.get?format=json&subtitle_format=mxm&app_id={MXM_APP}&commontrack_id={id}&usertoken={token}"
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
    use super::distinctive_title;

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
