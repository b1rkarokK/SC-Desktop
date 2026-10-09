//! Genius: plain (unsynced) lyrics — last resort in the lyrics chain.
//! Uses the site's own public search (no API key), lyrics come from the page.

use scraper::{node::Node, ElementRef, Html, Selector};
use serde_json::Value;
use url::Url;

use crate::{
    error::{AppError, AppResult},
    net::{HttpClient, Profile},
};

#[derive(Debug, Clone)]
pub struct GeniusHit {
    pub url: String,
    pub title: String,
    pub artist: String,
}

/// Returns a hit only if both artist and title plausibly match — wrong
/// lyrics are worse than none.
pub async fn search(http: &HttpClient, artist: &str, title: &str) -> AppResult<Option<GeniusHit>> {
    let url = search_url(artist, title)?;
    let resp: Value = http.get_json_with(url.as_str(), &[("referer", "https://genius.com/")]).await?;
    Ok(pick(&resp, artist, title))
}

/// `/api/search/multi?q=` for "artist title".
pub fn search_url(artist: &str, title: &str) -> AppResult<Url> {
    let mut url = Url::parse("https://genius.com/api/search/multi")?;
    url.query_pairs_mut().append_pair("per_page", "5").append_pair("q", &format!("{artist} {title}"));
    Ok(url)
}

/// Best song hit of a `/api/search/multi` response, if it plausibly matches.
pub fn pick(resp: &Value, artist: &str, title: &str) -> Option<GeniusHit> {
    let want_title = normalize(title);
    let want_artist = normalize(artist);
    let hits = resp["response"]["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|s| s["hits"].as_array().into_iter().flatten())
        .filter(|h| h["type"] == "song");
    let candidates: Vec<GeniusHit> = hits
        .filter_map(|h| {
            let r = &h["result"];
            Some(GeniusHit {
                url: r["url"].as_str()?.to_owned(),
                title: r["title"].as_str().unwrap_or_default().to_owned(),
                artist: r["primary_artist"]["name"].as_str().unwrap_or_default().to_owned(),
            })
        })
        .collect();
    let best = candidates
        .iter()
        .map(|h| (match_score(&want_artist, &want_title, &normalize(&h.artist), &normalize(&h.title)), h))
        .filter(|(s, _)| *s >= 3)
        .max_by_key(|(s, _)| *s)
        .map(|(_, h)| h.clone());
    // Title in another language/script ("ТЕНТАКЛИ" on SoundCloud, "tentacles"
    // on Genius): trust one of the two top hits if the artist matches exactly.
    let fallback = || {
        candidates
            .iter()
            .take(2)
            .find(|h| !want_artist.is_empty() && normalize(&h.artist) == want_artist)
            .cloned()
    };
    best.or_else(fallback)
}

pub async fn lyrics(http: &HttpClient, page_url: &str) -> AppResult<String> {
    let url = Url::parse(page_url)?;
    let host_ok =
        url.scheme() == "https" && url.host_str().is_some_and(|h| h == "genius.com" || h.ends_with(".genius.com"));
    if !host_ok {
        return Err(AppError::Parse("unexpected lyrics host".into()));
    }
    let html = http.get_text(url.as_str(), Profile::Document, None).await?;
    Ok(extract_lyrics(&html))
}

/// 0..=4: title exact=2 / partial=1, artist exact=2 / partial=1.
pub fn match_score(want_artist: &str, want_title: &str, artist: &str, title: &str) -> u8 {
    fn part(a: &str, b: &str) -> u8 {
        if a.is_empty() || b.is_empty() {
            0
        } else if a == b {
            2
        } else if a.len().min(b.len()) >= 3 && (a.contains(b) || b.contains(a)) {
            1
        } else {
            0
        }
    }
    let t = part(want_title, title);
    if t == 0 {
        return 0;
    }
    t + part(want_artist, artist)
}

/// Lowercase, letters/digits only (Unicode-aware, so Arabic/CJK survive).
/// Cyrillic is transliterated, so "Монокини" and "Monokini" compare equal.
pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase) {
        match translit(c) {
            Some(t) => out.push_str(t),
            None => out.push(c),
        }
    }
    out
}

fn translit(c: char) -> Option<&'static str> {
    Some(match c {
        'а' => "a",
        'б' => "b",
        'в' => "v",
        'г' => "g",
        'д' => "d",
        'е' | 'ё' | 'э' => "e",
        'ж' => "zh",
        'з' => "z",
        'и' | 'і' => "i",
        'й' | 'ы' => "y",
        'к' => "k",
        'л' => "l",
        'м' => "m",
        'н' => "n",
        'о' => "o",
        'п' => "p",
        'р' => "r",
        'с' => "s",
        'т' => "t",
        'у' => "u",
        'ф' => "f",
        'х' => "h",
        'ц' => "c",
        'ч' => "ch",
        'ш' => "sh",
        'щ' => "sch",
        'ъ' | 'ь' => "",
        'ю' => "yu",
        'я' => "ya",
        _ => return None,
    })
}

/// Builds a Genius query out of a SoundCloud artist + title.
/// SoundCloud titles are often "Artist - Title (Official Video) [Free DL]".
pub fn query_from(artist: &str, title: &str) -> (String, String) {
    let (mut artist, mut title) = (artist.to_owned(), title.to_owned());
    for sep in [" - ", " – ", " — "] {
        if let Some((a, t)) = title.split_once(sep) {
            artist = a.to_owned();
            title = t.to_owned();
            break;
        }
    }
    (first_artist(&clean(&strip_brackets(&artist))), clean(&strip_feat(&strip_brackets(&title))))
}

/// Drops emoji / decorations and tag-like junk that break lyric search:
/// "Song 💜", "Song | free dl", "song rocket.Ri", "#tag", "out now".
fn clean(s: &str) -> String {
    let s = s.split(['|', '/']).next().unwrap_or(s);
    let words: Vec<String> = s
        .split_whitespace()
        .filter(|w| !w.starts_with('#') && !w.starts_with('@'))
        // "rocket.Ri", "site.com": a dot glued between letters = a tag, not lyrics title
        .filter(|w| {
            let chars: Vec<char> = w.chars().collect();
            !chars.windows(3).any(|t| t[0].is_alphanumeric() && t[1] == '.' && t[2].is_alphanumeric())
        })
        .map(|w| w.chars().filter(|c| c.is_alphanumeric() || matches!(c, '\'' | '’' | '&' | '-' | '.' | ',' | '!' | '?')).collect::<String>())
        .filter(|w| !w.is_empty())
        .collect();
    let mut out = words.join(" ");
    for junk in ["free download", "free dl", "out now", "official video", "official audio", "lyrics", "lyric video"] {
        if let Some(i) = out.to_lowercase().find(junk) {
            if out.is_char_boundary(i) {
                out.truncate(i);
            }
        }
    }
    out.trim().trim_matches(|c: char| c == '-' || c == ',').trim().to_owned()
}

fn strip_brackets(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0u32;
    for c in s.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_feat(s: &str) -> String {
    let lower = s.to_lowercase();
    // "+" glues a guest on: "не знакомы+fortuna812"
    let cut = [" feat. ", " feat ", " ft. ", " ft ", " prod. ", " prod by ", "+"]
        .iter()
        .filter_map(|m| lower.find(m))
        .min();
    match cut {
        // `find` on the lowercased copy: only cut on an ASCII-safe boundary
        Some(i) if s.is_char_boundary(i) => s[..i].trim().to_owned(),
        _ => s.trim().to_owned(),
    }
}

fn first_artist(s: &str) -> String {
    let s = strip_feat(s);
    let lower = s.to_lowercase();
    let cut = [", ", " & ", " x ", " и ", " vs ", "+"].iter().filter_map(|m| lower.find(m)).min();
    match cut {
        Some(i) if s.is_char_boundary(i) => s[..i].trim().to_owned(),
        _ => s,
    }
}

/// Genius renders lyrics into one or more `div[data-lyrics-container]`; `<br>`
/// are line breaks, nodes with `data-exclude-from-selection` are UI chrome.
pub fn extract_lyrics(html: &str) -> String {
    let doc = Html::parse_document(html);
    let containers = Selector::parse(r#"div[data-lyrics-container="true"]"#).expect("static selector");
    let mut out = String::new();
    for c in doc.select(&containers) {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        walk(c, &mut out);
    }
    tidy(&out)
}

fn walk(el: ElementRef<'_>, out: &mut String) {
    for child in el.children() {
        match child.value() {
            Node::Text(t) => out.push_str(t),
            Node::Element(e) => {
                if e.name() == "br" {
                    out.push('\n');
                    continue;
                }
                let chrome = e.attr("data-exclude-from-selection").is_some()
                    || e.attr("class").is_some_and(|c| c.contains("LyricsHeader") || c.contains("SongBioPreview"));
                if chrome {
                    continue;
                }
                if let Some(child_el) = ElementRef::wrap(child) {
                    walk(child_el, out);
                }
            }
            _ => {}
        }
    }
}

pub fn tidy(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank_run = 0;
    for line in s.lines().map(str::trim_end) {
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run == 1 && !out.is_empty() {
                out.push('\n');
            }
        } else {
            blank_run = 0;
            out.push_str(line.trim_start());
            out.push('\n');
        }
    }
    let out = out.trim();
    // Genius puts a header line first: "[Текст песни «TATE»]", "[Song Lyrics]"
    match out.split_once('\n') {
        Some((first, rest)) if is_header(first) => rest.trim().to_owned(),
        _ => out.to_owned(),
    }
}

fn is_header(line: &str) -> bool {
    let l = line.trim().to_lowercase();
    l.starts_with('[') && l.ends_with(']') && (l.contains("текст песни") || l.ends_with(" lyrics]"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_lyrics_with_breaks_and_skips_chrome() {
        let html = r##"<html><body>
          <div data-lyrics-container="true"><div data-exclude-from-selection="true">12 Contributors</div>[Verse 1]<br/>Первая строка<br><a href="#"><span>Second line</span></a><br>שורה</div>
          <div data-lyrics-container="true">[Chorus]<br>Last</div>
        </body></html>"##;
        let text = extract_lyrics(html);
        assert_eq!(text, "[Verse 1]\nПервая строка\nSecond line\nשורה\n[Chorus]\nLast");
    }

    #[test]
    fn builds_query_from_soundcloud_title() {
        let (a, t) = query_from("SomeLabel", "Artist One & Two - Song Name (feat. X) [Free DL]");
        assert_eq!(a, "Artist One");
        assert_eq!(t, "Song Name");
    }

    #[test]
    fn drops_genius_header_line() {
        assert_eq!(tidy("[Текст песни «TATE»]\n[Куплет]\nстрока"), "[Куплет]\nстрока");
        assert_eq!(tidy("[Verse 1]\nline"), "[Verse 1]\nline");
    }

    #[test]
    fn translit_and_plus() {
        assert_eq!(normalize("Монокини"), normalize("Monokini"));
        assert_eq!(normalize("Дым сигарет с ментолом."), normalize("Дым сигарет с ментолом"));
        assert_eq!(query_from("тревога", "не знакомы+fortuna812 (prod:JESUSTALKWME)").1, "не знакомы");
        assert_eq!(query_from("Mona", "Monokini - Дотянуться до солнца"), ("Monokini".into(), "Дотянуться до солнца".into()));
    }

    #[test]
    fn cleans_junk_from_titles() {
        assert_eq!(query_from("x", "Тентакли 💜").1, "Тентакли");
        assert_eq!(query_from("x", "Song rocket.Ri").1, "Song");
        assert_eq!(query_from("x", "Song | free dl").1, "Song");
        assert_eq!(query_from("x", "Song #phonk out now").1, "Song");
    }

    #[test]
    fn scoring_requires_title() {
        assert_eq!(match_score("artist", "song", "artist", "other"), 0);
        assert_eq!(match_score("artist", "song", "artist", "song"), 4);
        assert!(match_score("label", "songname", "realartist", "songname") < 3);
    }
}
