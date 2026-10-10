//! DTOs shared with the frontend (snake_case JSON, mirrored in src/ui/api.ts).

use serde::{Deserialize, Serialize};

use crate::api::{
    lyrics::Line,
    soundcloud::{ScPlaylist, ScTrack, ScUser},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrackDto {
    pub id: u64,
    pub title: String,
    pub artist: String,
    pub user_id: u64,
    pub duration_ms: u64,
    pub artwork_url: Option<String>,
    pub permalink_url: Option<String>,
    pub genre: Option<String>,
}

impl From<&ScTrack> for TrackDto {
    fn from(t: &ScTrack) -> Self {
        Self {
            id: t.id,
            title: t.title.clone(),
            artist: t.artist_name(),
            user_id: t.artist_id(),
            // previews: show the song's real length, the player finds a full upload
            duration_ms: t.full_duration.filter(|_| t.is_preview()).unwrap_or(t.duration).max(t.duration),
            artwork_url: t.artwork().map(str::to_owned),
            permalink_url: t.permalink_url.clone(),
            genre: t.genre.clone().filter(|g| !g.trim().is_empty()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserDto {
    pub id: u64,
    pub username: String,
    pub avatar_url: Option<String>,
    pub followers_count: Option<u64>,
    pub track_count: Option<u64>,
    pub city: Option<String>,
    pub permalink_url: Option<String>,
}

impl From<&ScUser> for UserDto {
    fn from(u: &ScUser) -> Self {
        Self {
            id: u.id,
            username: u.username.clone(),
            avatar_url: u.avatar_url.clone(),
            followers_count: u.followers_count,
            track_count: u.track_count,
            city: u.city.clone().filter(|c| !c.trim().is_empty()),
            permalink_url: u.permalink_url.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistDto {
    pub id: u64,
    pub title: String,
    pub user_id: u64,
    pub artist: String,
    pub artwork_url: Option<String>,
    pub track_count: u64,
    pub is_album: bool,
    pub year: Option<String>,
    pub permalink_url: Option<String>,
    /// created by the signed-in user
    #[serde(default)]
    pub own: bool,
}

impl From<&ScPlaylist> for PlaylistDto {
    fn from(p: &ScPlaylist) -> Self {
        let first_art = p.tracks.iter().find_map(|t| t["artwork_url"].as_str().map(str::to_owned));
        Self {
            id: p.id,
            title: p.title.clone(),
            user_id: p.user.as_ref().map(|u| u.id).unwrap_or(0),
            artist: p.user.as_ref().map(|u| u.username.clone()).unwrap_or_default(),
            artwork_url: p.artwork_url.clone().or(first_art),
            track_count: p.track_count.unwrap_or(p.tracks.len() as u64),
            is_album: p.is_album(),
            year: p.release_date.as_deref().and_then(|d| d.get(..4)).map(str::to_owned),
            permalink_url: p.permalink_url.clone(),
            own: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchPage {
    pub tracks: Vec<TrackDto>,
    pub next_offset: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchAll {
    pub users: Vec<UserDto>,
    pub tracks: Vec<TrackDto>,
    pub playlists: Vec<PlaylistDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrackPage {
    pub track: TrackDto,
    pub tags: Vec<String>,
    pub year: Option<String>,
    pub plays: Option<u64>,
    pub likes: Option<u64>,
    pub related: Vec<TrackDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtistSection {
    pub tracks: Vec<TrackDto>,
    pub playlists: Vec<PlaylistDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlaylistPage {
    pub playlist: PlaylistDto,
    pub tracks: Vec<TrackDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LibraryCounts {
    pub tracks: u64,
    pub playlists: u64,
    pub albums: u64,
    pub artists: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthStatus {
    pub has_credentials: bool,
    pub username: Option<String>,
    pub user_id: Option<u64>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LyricsDto {
    pub track_id: u64,
    pub found: bool,
    pub source: Option<String>,
    pub synced: bool,
    pub word_level: bool,
    pub lines: Vec<Line>,
    pub url: Option<String>,
    #[serde(default)]
    pub cached: bool,
    /// the original song's text shown for a changed version (slowed …)
    #[serde(default)]
    pub original: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DislikedArtist {
    pub user_id: u64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WaveInfo {
    pub mood: String,
    pub no_liked: bool,
    pub reason: Option<String>,
    /// "по плейлисту «…»" when the wave is built around something
    pub context: Option<String>,
    pub disliked_tracks: u64,
    pub disliked_artists: Vec<DislikedArtist>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetCheck {
    pub ok: bool,
    pub status: Option<u16>,
    pub ms: u64,
    pub message: String,
}
