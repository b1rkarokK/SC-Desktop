// Typed bridge to the Rust commands (src-tauri/src/commands.rs).
import { convertFileSrc, invoke } from '@tauri-apps/api/core';

export interface Track {
  id: number;
  title: string;
  artist: string;
  user_id: number;
  duration_ms: number;
  artwork_url: string | null;
  permalink_url: string | null;
  genre: string | null;
}

export interface User {
  id: number;
  username: string;
  avatar_url: string | null;
  followers_count: number | null;
  track_count: number | null;
  city: string | null;
  permalink_url: string | null;
}

export interface Playlist {
  id: number;
  title: string;
  user_id: number;
  artist: string;
  artwork_url: string | null;
  track_count: number;
  is_album: boolean;
  year: string | null;
  permalink_url: string | null;
}

export type RepeatMode = 'off' | 'all' | 'one';
export type QueueSource = 'list' | 'likes' | 'wave';
export type NetMode = 'direct' | 'proxy';
export type Mood = 'normal' | 'fresh' | 'familiar' | 'calm' | 'energetic';

export interface PlayerSnapshot {
  track: Track | null;
  playing: boolean;
  loading: boolean;
  position_ms: number;
  volume: number;
  shuffle: boolean;
  repeat: RepeatMode;
  source: QueueSource;
  queue_len: number;
  queue_pos: number;
}

export interface AuthStatus {
  has_credentials: boolean;
  username: string | null;
  user_id: number | null;
  avatar_url: string | null;
}

export interface EqConfig {
  enabled: boolean;
  preamp: number;
  gains: number[];
  preset: string;
}

export interface DiscordConfig {
  enabled: boolean;
  app_id: string;
  progress: boolean;
  button: boolean;
  hide_on_pause: boolean;
}

export interface AppConfig {
  net_mode: NetMode;
  proxy: string | null;
  chrome_version: number;
  volume: number;
  eq: EqConfig;
  discord: DiscordConfig;
  start_minimized: boolean;
  ui: Record<string, unknown>;
}

export interface Word {
  start_ms: number;
  end_ms: number;
  text: string;
}

export interface LyricLine {
  start_ms: number | null;
  end_ms: number | null;
  text: string;
  words: Word[];
}

export interface Lyrics {
  track_id: number;
  found: boolean;
  source: string | null;
  synced: boolean;
  word_level: boolean;
  lines: LyricLine[];
  url: string | null;
  cached: boolean;
}

export interface SearchPage {
  tracks: Track[];
  next_offset: number | null;
}

export interface SearchAll {
  users: User[];
  tracks: Track[];
  playlists: Playlist[];
}

export interface TrackPage {
  track: Track;
  tags: string[];
  year: string | null;
  plays: number | null;
  likes: number | null;
  related: Track[];
}

export interface ArtistSection {
  tracks: Track[];
  playlists: Playlist[];
}

export interface PlaylistPage {
  playlist: Playlist;
  tracks: Track[];
}

export interface LibraryCounts {
  tracks: number;
  playlists: number;
  albums: number;
  artists: number;
}

export interface DislikedArtist {
  user_id: number;
  name: string;
}

export interface WaveInfo {
  mood: Mood;
  reason: string | null;
  disliked_tracks: number;
  disliked_artists: DislikedArtist[];
}

export interface NetCheck {
  ok: boolean;
  status: number | null;
  ms: number;
  message: string;
}

export interface SystemPrefs {
  autostart: boolean;
  start_minimized: boolean;
}

export interface AppErrorPayload {
  kind: string;
  message: string;
}

export function isAppError(e: unknown): e is AppErrorPayload {
  return typeof e === 'object' && e !== null && 'kind' in e && 'message' in e;
}

export function errorMessage(e: unknown): string {
  if (isAppError(e)) return e.message;
  if (e instanceof Error) return e.message;
  return String(e);
}

export const api = {
  authStatus: () => invoke<AuthStatus>('auth_status'),
  authVerify: () => invoke<AuthStatus>('auth_verify'),
  authSave: (clientId: string, oauthToken: string) => invoke<AuthStatus>('auth_save', { clientId, oauthToken }),
  authLogin: () => invoke<void>('auth_login'),
  authClear: () => invoke<void>('auth_clear'),

  likesSync: () => invoke<number>('likes_sync'),
  likesCount: () => invoke<number>('likes_count'),
  likesPage: (offset: number, limit: number) => invoke<Track[]>('likes_page', { offset, limit }),
  likesIds: () => invoke<number[]>('likes_ids'),
  likeSet: (track: Track, liked: boolean) => invoke<void>('like_set', { track, liked }),
  libraryPlaylists: (force: boolean) => invoke<Playlist[]>('library_playlists', { force }),
  libraryArtists: (force: boolean) => invoke<User[]>('library_artists', { force }),
  libraryCounts: () => invoke<LibraryCounts>('library_counts'),
  followSet: (user: User, follow: boolean) => invoke<void>('follow_set', { user, follow }),

  searchTracks: (query: string, offset: number) => invoke<SearchPage>('search_tracks', { query, offset }),
  searchAll: (query: string) => invoke<SearchAll>('search_all', { query }),
  searchUsers: (query: string) => invoke<User[]>('search_users', { query }),
  searchPlaylists: (query: string) => invoke<Playlist[]>('search_playlists', { query }),

  trackPage: (id: number) => invoke<TrackPage>('track_page', { id }),
  artistGet: (id: number) => invoke<User>('artist_get', { id }),
  artistSection: (id: number, section: string) => invoke<ArtistSection>('artist_section', { id, section }),
  playlistPage: (id: number) => invoke<PlaylistPage>('playlist_page', { id }),

  playLikes: (index: number) => invoke<void>('player_play_likes', { index }),
  playTracks: (tracks: Track[], index: number) => invoke<void>('player_play_tracks', { tracks, index }),
  enqueue: (track: Track, next: boolean) => invoke<void>('player_enqueue', { track, next }),
  toggle: () => invoke<void>('player_toggle'),
  next: () => invoke<void>('player_next'),
  prev: () => invoke<void>('player_prev'),
  seek: (positionMs: number) => invoke<void>('player_seek', { positionMs: Math.max(0, Math.round(positionMs)) }),
  setVolume: (volume: number, persist: boolean) => invoke<void>('player_set_volume', { volume, persist }),
  setShuffle: (enabled: boolean) => invoke<void>('player_set_shuffle', { enabled }),
  setRepeat: (mode: RepeatMode) => invoke<void>('player_set_repeat', { mode }),
  snapshot: () => invoke<PlayerSnapshot>('player_snapshot'),
  position: () => invoke<number>('player_position'),
  upcoming: (limit: number) => invoke<Track[]>('player_upcoming', { limit }),

  waveStart: () => invoke<number>('wave_start'),
  waveStartFrom: (trackId: number | null, artistId: number | null) =>
    invoke<number>('wave_start_from', { trackId, artistId }),
  waveSetMood: (mood: Mood) => invoke<void>('wave_set_mood', { mood }),
  waveInfo: (trackId: number | null) => invoke<WaveInfo>('wave_info', { trackId }),
  dislikeSet: (trackId: number, disliked: boolean) => invoke<void>('dislike_set', { trackId, disliked }),
  dislikedIds: () => invoke<number[]>('disliked_ids'),
  waveDislikeArtist: (userId: number, name: string) => invoke<void>('wave_dislike_artist', { userId, name }),
  waveUndislikeArtist: (userId: number) => invoke<void>('wave_undislike_artist', { userId }),
  waveClearDislikes: () => invoke<void>('wave_clear_dislikes'),

  lyrics: (track: Track, force: boolean) => invoke<Lyrics>('lyrics_get', { track, force }),
  openExternal: (url: string) => invoke<void>('open_external', { url }),

  configGet: () => invoke<AppConfig>('config_get'),
  configSetNetwork: (netMode: NetMode, proxy: string | null, chromeVersion: number) =>
    invoke<AppConfig>('config_set_network', { netMode, proxy, chromeVersion }),
  netCheck: (netMode: NetMode, proxy: string | null) => invoke<NetCheck>('net_check', { netMode, proxy }),
  configSetUi: (ui: Record<string, unknown>) => invoke<void>('config_set_ui', { ui }),
  eqSet: (eq: EqConfig, persist: boolean) => invoke<void>('eq_set', { eq, persist }),
  discordSet: (discord: DiscordConfig) => invoke<void>('discord_set', { discord }),
  systemGet: () => invoke<SystemPrefs>('system_get'),
  systemSet: (autostart: boolean, startMinimized: boolean) =>
    invoke<void>('system_set', { autostart, startMinimized }),
};

export type CoverSize = 't67x67' | 't300x300' | 't500x500';

/** Artwork goes through the Rust `cover://` cache (SHA-256 keys, LRU 200 MB). */
export function coverUrl(artwork: string | null | undefined, size: CoverSize): string | null {
  if (!artwork) return null;
  const sized = artwork.replace(/-(large|small|badge|tiny|mini|crop|original|t\d+x\d+)\.(jpg|png|jpeg)$/, `-${size}.$2`);
  return convertFileSrc(sized, 'cover');
}

export function fmtTime(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = String(total % 60).padStart(2, '0');
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`;
}

const nf = new Intl.NumberFormat('ru-RU');

export function fmtCount(n: number | null | undefined): string {
  if (n === null || n === undefined) return '';
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1).replace('.', ',')} млн`;
  if (n >= 10_000) return `${(n / 1000).toFixed(1).replace('.', ',')} тыс.`;
  return nf.format(n);
}

/** 1 трек, 2 трека, 5 треков */
export function plural(n: number, one: string, few: string, many: string): string {
  const m10 = n % 10;
  const m100 = n % 100;
  if (m10 === 1 && m100 !== 11) return one;
  if (m10 >= 2 && m10 <= 4 && (m100 < 12 || m100 > 14)) return few;
  return many;
}
