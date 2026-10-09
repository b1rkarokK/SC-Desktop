//! Unofficial SoundCloud api-v2 client (the same API soundcloud.com uses).
//!
//! Auth = the user's own `client_id` + `oauth_token` copied from DevTools.
//! Requests go out with the browser identity from `net::client`, so from
//! SoundCloud's side they look like the web player on the user's home IP.

use std::sync::Arc;

use futures::{StreamExt, TryStreamExt};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::{
    error::{AppError, AppResult},
    net::{HttpClient, Profile},
    secrets::Credentials,
};

pub const API_HOST: &str = "api-v2.soundcloud.com";
const API_BASE: &str = "https://api-v2.soundcloud.com";
pub const PAGE_SIZE: u32 = 50;
/// Upper bound for likes sync: 400 pages × 50.
const MAX_LIKE_PAGES: usize = 400;
const HLS_PARALLEL_SEGMENTS: usize = 6;

// ---------------------------------------------------------------- models

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScUser {
    pub id: u64,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub permalink_url: Option<String>,
    #[serde(default)]
    pub followers_count: Option<u64>,
    #[serde(default)]
    pub track_count: Option<u64>,
    #[serde(default)]
    pub city: Option<String>,
    #[serde(default)]
    pub full_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScPlaylist {
    pub id: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artwork_url: Option<String>,
    #[serde(default)]
    pub permalink_url: Option<String>,
    #[serde(default)]
    pub user: Option<ScUser>,
    #[serde(default)]
    pub track_count: Option<u64>,
    #[serde(default)]
    pub is_album: Option<bool>,
    /// "album" | "ep" | "single" | "compilation" | ""
    #[serde(default)]
    pub set_type: Option<String>,
    #[serde(default)]
    pub release_date: Option<String>,
    #[serde(default)]
    pub duration: Option<u64>,
    /// first few are full, the rest only carry `id`
    #[serde(default)]
    pub tracks: Vec<Value>,
}

impl ScPlaylist {
    pub fn is_album(&self) -> bool {
        self.is_album == Some(true) || matches!(self.set_type.as_deref(), Some("album" | "ep" | "single" | "compilation"))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PublisherMetadata {
    #[serde(default)]
    pub artist: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscodingFormat {
    pub protocol: String,
    pub mime_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcoding {
    pub url: String,
    #[serde(default)]
    pub preset: String,
    #[serde(default)]
    pub snipped: bool,
    pub format: TranscodingFormat,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Media {
    #[serde(default)]
    pub transcodings: Vec<Transcoding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScTrack {
    pub id: u64,
    #[serde(default)]
    pub title: String,
    /// milliseconds
    #[serde(default)]
    pub duration: u64,
    #[serde(default)]
    pub genre: Option<String>,
    #[serde(default)]
    pub tag_list: Option<String>,
    #[serde(default)]
    pub artwork_url: Option<String>,
    #[serde(default)]
    pub permalink_url: Option<String>,
    #[serde(default)]
    pub user_id: Option<u64>,
    #[serde(default)]
    pub user: Option<ScUser>,
    #[serde(default)]
    pub publisher_metadata: Option<PublisherMetadata>,
    #[serde(default)]
    pub media: Option<Media>,
    #[serde(default)]
    pub track_authorization: Option<String>,
    /// ALLOW / MONETIZE / SNIP (30s preview only) / BLOCK
    #[serde(default)]
    pub policy: Option<String>,
    #[serde(default)]
    pub streamable: Option<bool>,
    #[serde(default)]
    pub playback_count: Option<u64>,
    #[serde(default)]
    pub likes_count: Option<u64>,
    #[serde(default)]
    pub release_date: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
}

impl ScTrack {
    pub fn artist_id(&self) -> u64 {
        self.user.as_ref().map(|u| u.id).or(self.user_id).unwrap_or(0)
    }

    pub fn uploader(&self) -> &str {
        self.user.as_ref().map(|u| u.username.as_str()).unwrap_or("")
    }

    /// Credited artist if the label filled it, otherwise the uploader.
    pub fn artist_name(&self) -> String {
        self.publisher_metadata
            .as_ref()
            .and_then(|p| p.artist.as_deref())
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .unwrap_or_else(|| self.uploader())
            .to_owned()
    }

    pub fn artwork(&self) -> Option<&str> {
        self.artwork_url
            .as_deref()
            .or_else(|| self.user.as_ref().and_then(|u| u.avatar_url.as_deref()))
    }

    /// Full-length playback is possible (no preview-only / geo-blocked tracks).
    pub fn is_fully_playable(&self) -> bool {
        !matches!(self.policy.as_deref(), Some("BLOCK" | "SNIP")) && self.streamable != Some(false)
    }
}

#[derive(Debug, Deserialize)]
pub struct Collection<T> {
    #[serde(default = "Vec::new")]
    pub collection: Vec<T>,
    #[serde(default)]
    pub next_href: Option<String>,
}

pub enum UserSection {
    Tracks(Vec<ScTrack>),
    Playlists(Vec<ScPlaylist>),
}

#[derive(Debug, Deserialize)]
struct LikeItem {
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    track: Option<ScTrack>,
}

#[derive(Debug, Deserialize)]
struct ResolvedStream {
    url: String,
}

// ---------------------------------------------------------------- client

pub struct SoundCloud {
    http: Arc<HttpClient>,
    client_id: String,
    auth: String,
}

impl SoundCloud {
    pub fn new(http: Arc<HttpClient>, creds: &Credentials) -> Self {
        Self {
            http,
            client_id: creds.client_id.clone(),
            auth: format!("OAuth {}", creds.oauth_token),
        }
    }

    fn api_url(&self, path: &str, query: &[(&str, String)]) -> AppResult<Url> {
        let mut url = Url::parse(&format!("{API_BASE}{path}"))?;
        {
            let mut q = url.query_pairs_mut();
            for (k, v) in query {
                q.append_pair(k, v);
            }
            q.append_pair("client_id", &self.client_id);
            q.append_pair("app_locale", "en");
        }
        Ok(url)
    }

    /// `next_href` / media URLs come from the server; only ever send the token to api-v2.
    fn api_href(&self, href: &str) -> AppResult<Url> {
        let mut url = Url::parse(href)?;
        if url.scheme() != "https" || url.host_str() != Some(API_HOST) {
            return Err(AppError::Parse(format!("unexpected API host: {:?}", url.host_str())));
        }
        if !url.query_pairs().any(|(k, _)| k == "client_id") {
            url.query_pairs_mut().append_pair("client_id", &self.client_id);
        }
        Ok(url)
    }

    async fn get<T: DeserializeOwned>(&self, url: Url) -> AppResult<T> {
        self.http.get_json(url.as_str(), Profile::ScApi, Some(&self.auth)).await
    }

    pub async fn me(&self) -> AppResult<ScUser> {
        self.get(self.api_url("/me", &[])?).await
    }

    pub async fn track(&self, id: u64) -> AppResult<ScTrack> {
        self.get(self.api_url(&format!("/tracks/{id}"), &[])?).await
    }

    /// All liked tracks, newest first, paging by 50 via `next_href`.
    pub async fn likes_all(
        &self,
        user_id: u64,
        mut on_progress: impl FnMut(usize) + Send,
    ) -> AppResult<Vec<(ScTrack, String)>> {
        let mut url = self.api_url(
            &format!("/users/{user_id}/track_likes"),
            &[("limit", PAGE_SIZE.to_string()), ("linked_partitioning", "1".into())],
        )?;
        let mut out = Vec::new();
        for _ in 0..MAX_LIKE_PAGES {
            let page: Collection<LikeItem> = self.get(url).await?;
            out.extend(page.collection.into_iter().filter_map(|i| i.track.map(|t| (t, i.created_at))));
            on_progress(out.len());
            match page.next_href.filter(|h| !h.is_empty()) {
                Some(next) => url = self.api_href(&next)?,
                None => break,
            }
        }
        Ok(out)
    }

    /// Ids of the newest liked tracks (one request) — for change detection.
    pub async fn likes_first_ids(&self, user_id: u64) -> AppResult<Vec<u64>> {
        let url = self.api_url(&format!("/users/{user_id}/track_likes"), &[("limit", "20".into())])?;
        let page: Collection<LikeItem> = self.get(url).await?;
        Ok(page.collection.into_iter().filter_map(|i| i.track.map(|t| t.id)).collect())
    }

    /// Like (PUT) / unlike (DELETE) — the same calls the web player makes.
    /// `cookies`: the soundcloud.com browser session (incl. the anti-bot
    /// `datadome` cookie) — write requests without it are answered with 403.
    /// Like / unlike request: (method, url). Sent by `bridge` from a browser page.
    pub fn like_request(&self, user_id: u64, track_id: u64, liked: bool) -> AppResult<(&'static str, Url)> {
        let url = self.api_url(&format!("/users/{user_id}/track_likes/{track_id}"), &[])?;
        Ok((if liked { "PUT" } else { "DELETE" }, url))
    }

    pub fn follow_request(&self, user_id: u64, follow: bool) -> AppResult<(&'static str, Url)> {
        let url = self.api_url(&format!("/me/followings/{user_id}"), &[])?;
        Ok((if follow { "POST" } else { "DELETE" }, url))
    }

    pub fn auth_header(&self) -> &str {
        &self.auth
    }

    /// Follows `next_href` up to `max_pages` pages of 50 (or the given limit).
    async fn paged<T: DeserializeOwned>(&self, mut url: Url, max_pages: usize) -> AppResult<Vec<T>> {
        let mut out = Vec::new();
        for _ in 0..max_pages {
            let page: Collection<T> = self.get(url).await?;
            out.extend(page.collection);
            match page.next_href.filter(|h| !h.is_empty()) {
                Some(next) => url = self.api_href(&next)?,
                None => break,
            }
        }
        Ok(out)
    }

    /// Liked playlists and albums (newest first).
    pub async fn playlist_likes(&self, user_id: u64) -> AppResult<Vec<ScPlaylist>> {
        #[derive(Deserialize)]
        struct Item {
            #[serde(default)]
            playlist: Option<ScPlaylist>,
        }
        let url = self.api_url(
            &format!("/users/{user_id}/playlist_likes"),
            &[("limit", PAGE_SIZE.to_string()), ("linked_partitioning", "1".into())],
        )?;
        let items: Vec<Item> = self.paged(url, 40).await?;
        Ok(items.into_iter().filter_map(|i| i.playlist).collect())
    }

    /// Playlists and albums the user created (with OAuth also the private ones).
    pub async fn own_playlists(&self, user_id: u64) -> AppResult<Vec<ScPlaylist>> {
        let url = self.api_url(
            &format!("/users/{user_id}/playlists"),
            &[("limit", PAGE_SIZE.to_string()), ("linked_partitioning", "1".into())],
        )?;
        self.paged(url, 20).await
    }

    /// Artists the user follows.
    pub async fn followings(&self, user_id: u64) -> AppResult<Vec<ScUser>> {
        let url = self.api_url(
            &format!("/users/{user_id}/followings"),
            &[("limit", "200".into()), ("linked_partitioning", "1".into())],
        )?;
        self.paged(url, 20).await
    }

    pub async fn user(&self, id: u64) -> AppResult<ScUser> {
        self.get(self.api_url(&format!("/users/{id}"), &[])?).await
    }

    /// Artist page tabs: popular | tracks | albums | playlists | reposts
    pub async fn user_section(&self, id: u64, section: &str) -> AppResult<UserSection> {
        let q = [("limit", PAGE_SIZE.to_string()), ("linked_partitioning", "1".into())];
        Ok(match section {
            "popular" => UserSection::Tracks(self.get::<Collection<ScTrack>>(self.api_url(&format!("/users/{id}/toptracks"), &q)?).await?.collection),
            "tracks" => UserSection::Tracks(self.paged(self.api_url(&format!("/users/{id}/tracks"), &q)?, 4).await?),
            "albums" => UserSection::Playlists(self.paged(self.api_url(&format!("/users/{id}/albums"), &q)?, 2).await?),
            "playlists" => UserSection::Playlists(
                self.paged(self.api_url(&format!("/users/{id}/playlists_without_albums"), &q)?, 2).await?,
            ),
            "reposts" => {
                let items: Vec<Value> = self.paged(self.api_url(&format!("/stream/users/{id}/reposts"), &q)?, 2).await?;
                UserSection::Tracks(
                    items.into_iter().filter_map(|i| serde_json::from_value::<ScTrack>(i["track"].clone()).ok()).collect(),
                )
            }
            other => return Err(AppError::Other(format!("unknown section {other}"))),
        })
    }

    pub async fn playlist(&self, id: u64) -> AppResult<(ScPlaylist, Vec<ScTrack>)> {
        let pl: ScPlaylist = self.get(self.api_url(&format!("/playlists/{id}"), &[("representation", "full".into())])?).await?;
        let ids: Vec<u64> = pl.tracks.iter().filter_map(|t| t["id"].as_u64()).collect();
        let tracks = self.tracks_by_ids(&ids).await?;
        Ok((pl, tracks))
    }

    /// Hydrates partial tracks, keeping the given order (max 50 ids per request).
    pub async fn tracks_by_ids(&self, ids: &[u64]) -> AppResult<Vec<ScTrack>> {
        let mut found = std::collections::HashMap::new();
        for chunk in ids.chunks(50) {
            let joined = chunk.iter().map(u64::to_string).collect::<Vec<_>>().join(",");
            let batch: Vec<ScTrack> = self.get(self.api_url("/tracks", &[("ids", joined)])?).await?;
            found.extend(batch.into_iter().map(|t| (t.id, t)));
        }
        Ok(ids.iter().filter_map(|id| found.remove(id)).collect())
    }

    pub async fn search_users(&self, query: &str, limit: u32) -> AppResult<Vec<ScUser>> {
        let url = self.api_url("/search/users", &[("q", query.to_owned()), ("limit", limit.to_string())])?;
        Ok(self.get::<Collection<ScUser>>(url).await?.collection)
    }

    pub async fn search_playlists(&self, query: &str, limit: u32) -> AppResult<Vec<ScPlaylist>> {
        let url = self.api_url("/search/playlists", &[("q", query.to_owned()), ("limit", limit.to_string())])?;
        Ok(self.get::<Collection<ScPlaylist>>(url).await?.collection)
    }

    pub async fn related(&self, track_id: u64) -> AppResult<Vec<ScTrack>> {
        let url = self.api_url(&format!("/tracks/{track_id}/related"), &[("limit", PAGE_SIZE.to_string())])?;
        Ok(self.get::<Collection<ScTrack>>(url).await?.collection)
    }

    pub async fn user_tracks(&self, user_id: u64) -> AppResult<Vec<ScTrack>> {
        let url = self.api_url(
            &format!("/users/{user_id}/tracks"),
            &[("limit", PAGE_SIZE.to_string()), ("linked_partitioning", "1".into())],
        )?;
        Ok(self.get::<Collection<ScTrack>>(url).await?.collection)
    }

    pub async fn search_tracks(&self, query: &str, offset: u32) -> AppResult<(Vec<ScTrack>, Option<u32>)> {
        let url = self.api_url(
            "/search/tracks",
            &[
                ("q", query.to_owned()),
                ("limit", PAGE_SIZE.to_string()),
                ("offset", offset.to_string()),
                ("linked_partitioning", "1".into()),
            ],
        )?;
        let page: Collection<ScTrack> = self.get(url).await?;
        let next = page
            .next_href
            .filter(|h| !h.is_empty() && !page.collection.is_empty())
            .map(|_| offset + PAGE_SIZE);
        Ok((page.collection, next))
    }

    /// Resolves the best transcoding and downloads the whole track into memory
    /// (≈ 4–10 MB for a typical track; makes seeking trivial and robust).
    pub async fn download_audio(&self, track: &ScTrack) -> AppResult<Vec<u8>> {
        if track.policy.as_deref() == Some("BLOCK") {
            return Err(AppError::UnsupportedStream("заблокирован в вашем регионе".into()));
        }
        let tc = pick_transcoding(track)?;
        if tc.snipped {
            tracing::warn!(track = track.id, "only a 30s preview is available");
        }
        let mut url = self.api_href(&tc.url)?;
        if let Some(auth) = &track.track_authorization {
            url.query_pairs_mut().append_pair("track_authorization", auth);
        }
        let resolved: ResolvedStream = self.get(url).await?;
        tracing::debug!(track = track.id, protocol = %tc.format.protocol, mime = %tc.format.mime_type, "stream resolved");

        match tc.format.protocol.as_str() {
            "progressive" => self.http.get_bytes(&resolved.url, Profile::Media, None).await,
            "hls" => self.fetch_hls(&resolved.url).await,
            other => Err(AppError::UnsupportedStream(format!("протокол {other}"))),
        }
    }

    /// Whether the native engine can get this track's stream (cheap: resolves
    /// the stream URL, downloads nothing).
    pub async fn native_stream_ok(&self, track: &ScTrack) -> AppResult<bool> {
        let Ok(tc) = pick_transcoding(track) else { return Ok(false) };
        let mut url = self.api_href(&tc.url)?;
        if let Some(auth) = &track.track_authorization {
            url.query_pairs_mut().append_pair("track_authorization", auth);
        }
        match self.get::<ResolvedStream>(url).await {
            Ok(_) => Ok(true),
            Err(AppError::NotFound) => Ok(false),
            Err(e) => Err(e),
        }
    }

    async fn fetch_hls(&self, playlist_url: &str) -> AppResult<Vec<u8>> {
        let mut base = Url::parse(playlist_url)?;
        let mut text = self.http.get_text(base.as_str(), Profile::Media, None).await?;

        // Master playlist → follow the first variant (one level).
        if text.contains("#EXT-X-STREAM-INF") {
            let variant = text
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty() && !l.starts_with('#'))
                .ok_or_else(|| AppError::Parse("empty HLS master playlist".into()))?;
            base = base.join(variant)?;
            text = self.http.get_text(base.as_str(), Profile::Media, None).await?;
        }

        let playlist = parse_media_playlist(&base, &text)?;
        let mut out = Vec::new();
        if let Some(init) = &playlist.init {
            out.extend(self.http.get_bytes(init.as_str(), Profile::Media, None).await?);
        }

        let http = self.http.clone();
        let chunks: Vec<Vec<u8>> = futures::stream::iter(playlist.segments.into_iter().map(|seg| {
            let http = http.clone();
            async move { http.get_bytes(seg.as_str(), Profile::Media, None).await }
        }))
        .buffered(HLS_PARALLEL_SEGMENTS)
        .try_collect()
        .await?;

        out.reserve(chunks.iter().map(Vec::len).sum());
        for c in chunks {
            out.extend_from_slice(&c);
        }
        Ok(out)
    }
}

/// The public web client_id is embedded in soundcloud.com's JS bundles;
/// read it from there so the user never has to dig in DevTools.
pub async fn discover_client_id(http: &HttpClient) -> AppResult<String> {
    let html = http.get_text("https://soundcloud.com/", Profile::Document, None).await?;
    let mut scripts = Vec::new();
    let marker = "src=\"https://a-v2.sndcdn.com/assets/";
    let mut rest = html.as_str();
    while let Some(i) = rest.find(marker) {
        let start = i + "src=\"".len();
        let Some(len) = rest[start..].find('"') else { break };
        scripts.push(rest[start..start + len].to_owned());
        rest = &rest[start + len..];
    }
    // the id lives in one of the last bundles
    for src in scripts.iter().rev() {
        let js = http.get_text(src, Profile::Media, None).await?;
        if let Some(id) = find_client_id(&js) {
            return Ok(id);
        }
    }
    Err(AppError::Parse("client_id не найден на soundcloud.com — введите его вручную".into()))
}

fn find_client_id(js: &str) -> Option<String> {
    for marker in ["client_id:\"", "client_id=\"", "?client_id="] {
        let mut rest = js;
        while let Some(i) = rest.find(marker) {
            let tail = &rest[i + marker.len()..];
            let id: String = tail.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
            if (20..=40).contains(&id.len()) {
                return Some(id);
            }
            rest = tail;
        }
    }
    None
}

/// The track has DRM streams (Widevine / PlayReady HLS), playable only by
/// SoundCloud's own player.
pub fn has_drm_stream(track: &ScTrack) -> bool {
    track.policy.as_deref() != Some("BLOCK")
        && track.media.as_ref().is_some_and(|m| m.transcodings.iter().any(|t| t.format.protocol.contains("encrypted")))
}

/// Preference: full track over preview; progressive MP3 → HLS MP3 → HLS AAC (fMP4).
/// Encrypted HLS (Go+ DRM) and Opus are skipped: rodio/symphonia cannot play them.
fn pick_transcoding(track: &ScTrack) -> AppResult<&Transcoding> {
    let media = track
        .media
        .as_ref()
        .ok_or_else(|| AppError::UnsupportedStream("нет медиа-данных".into()))?;

    let rank = |t: &Transcoding| -> Option<u8> {
        let mime = t.format.mime_type.as_str();
        match t.format.protocol.as_str() {
            "progressive" if mime.starts_with("audio/mpeg") => Some(0),
            "hls" if mime.starts_with("audio/mpeg") => Some(1),
            "hls" if mime.starts_with("audio/mp4") || mime.contains("mp4a") => Some(2),
            _ => None,
        }
    };

    media
        .transcodings
        .iter()
        .filter_map(|t| rank(t).map(|r| ((t.snipped as u8, r), t)))
        .min_by_key(|(k, _)| *k)
        .map(|(_, t)| t)
        .ok_or_else(|| AppError::UnsupportedStream("только зашифрованные (DRM) или неподдерживаемые форматы".into()))
}

#[derive(Debug)]
struct MediaPlaylist {
    init: Option<Url>,
    segments: Vec<Url>,
}

fn parse_media_playlist(base: &Url, text: &str) -> AppResult<MediaPlaylist> {
    let mut init = None;
    let mut segments = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(attrs) = line.strip_prefix("#EXT-X-KEY:") {
            if !attrs.contains("METHOD=NONE") {
                return Err(AppError::UnsupportedStream("зашифрованный HLS (DRM)".into()));
            }
        } else if let Some(attrs) = line.strip_prefix("#EXT-X-MAP:") {
            if let Some(uri) = quoted_attr(attrs, "URI") {
                init = Some(base.join(uri)?);
            }
        } else if !line.starts_with('#') {
            segments.push(base.join(line)?);
        }
    }
    if segments.is_empty() {
        return Err(AppError::Parse("HLS playlist has no segments".into()));
    }
    Ok(MediaPlaylist { init, segments })
}

fn quoted_attr<'a>(attrs: &'a str, key: &str) -> Option<&'a str> {
    let start = attrs.find(&format!("{key}=\""))? + key.len() + 2;
    let len = attrs[start..].find('"')?;
    Some(&attrs[start..start + len])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fmp4_playlist() {
        let base = Url::parse("https://cf-hls-media.sndcdn.com/playlist/abc/playlist.m3u8?x=1").unwrap();
        let text = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-MAP:URI=\"init.mp4?a=1\"\n#EXTINF:10.0,\nseg0.m4s\n#EXTINF:10.0,\nhttps://other.sndcdn.com/seg1.m4s\n#EXT-X-ENDLIST\n";
        let p = parse_media_playlist(&base, text).unwrap();
        assert_eq!(p.init.unwrap().as_str(), "https://cf-hls-media.sndcdn.com/playlist/abc/init.mp4?a=1");
        assert_eq!(p.segments.len(), 2);
        assert_eq!(p.segments[1].host_str(), Some("other.sndcdn.com"));
    }

    #[test]
    fn finds_client_id_in_bundle() {
        let js = r#"x={env:"production",client_id:"aBcDeFgHiJkLmNoPqRsTuVwXyZ012345",y:1}"#;
        assert_eq!(find_client_id(js).as_deref(), Some("aBcDeFgHiJkLmNoPqRsTuVwXyZ012345"));
        assert_eq!(find_client_id(r#"client_id:"short""#), None);
    }

    #[test]
    fn rejects_encrypted_hls() {
        let base = Url::parse("https://x.sndcdn.com/p.m3u8").unwrap();
        let text = "#EXTM3U\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://k\"\nseg.ts\n";
        assert!(matches!(parse_media_playlist(&base, text), Err(AppError::UnsupportedStream(_))));
    }

    #[test]
    fn prefers_full_progressive() {
        let json = r#"{"id":1,"title":"t","duration":1000,"media":{"transcodings":[
            {"url":"https://api-v2.soundcloud.com/a","snipped":false,"format":{"protocol":"hls","mime_type":"audio/mp4; codecs=\"mp4a.40.2\""}},
            {"url":"https://api-v2.soundcloud.com/b","snipped":true,"format":{"protocol":"progressive","mime_type":"audio/mpeg"}},
            {"url":"https://api-v2.soundcloud.com/c","snipped":false,"format":{"protocol":"ctr-encrypted-hls","mime_type":"audio/mpeg"}}
        ]}}"#;
        let t: ScTrack = serde_json::from_str(json).unwrap();
        assert!(pick_transcoding(&t).unwrap().url.ends_with("/a"));
    }
}
