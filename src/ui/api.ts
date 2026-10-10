// Typed bridge to the Rust commands (src-tauri/src/commands.rs).
import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { T, tr } from './i18n';

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
  own: boolean;
}

export interface PickedFile {
  path: string;
  name: string;
  size: number;
  /** file name without extension */
  title: string;
}

export interface UploadForm {
  path: string;
  title: string;
  genre: string | null;
  tags: string | null;
  private: boolean;
  artwork: string | null;
}

export interface MyTrack {
  track: Track;
  private: boolean;
  plays: number;
  created_at: string | null;
}

export interface HistoryItem {
  track: Track;
  played_at: number;
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
  /** Smart Shuffle: recommendations mixed into the list */
  smart: boolean;
  /** the current track is such a recommendation */
  recommended: boolean;
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

export interface FxConfig {
  /** playback rate: < 1 slowed, > 1 sped up */
  speed: number;
  /** 0 … 1 */
  reverb: number;
  normalize: boolean;
  /** seconds, 0 = off */
  crossfade: number;
}

export interface QueueView {
  /** place of the current track in the play order */
  current: number | null;
  items: { at: number; track: Track; recommended: boolean }[];
  upcoming: number;
  source: QueueSource;
}

export interface Stats {
  plays: number;
  ms: number;
  tracks: number;
  artists: number;
  top_tracks: { track: Track; plays: number }[];
  top_artists: { user_id: number; name: string; plays: number; ms: number; artwork_url: string | null }[];
  hours: number[];
  genres: [string, number][];
  streak: number;
}

export interface SleepState {
  remaining_s: number | null;
  end_of_track: boolean;
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
  fx?: FxConfig;
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
  /** the original song's text over a changed version (slowed …) */
  original?: boolean;
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
  no_liked: boolean;
  reason: string | null;
  /** "по плейлисту «…»" when the wave is built around something */
  context: string | null;
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
  fast_protected: boolean;
  notify_new: boolean;
}

export interface AppErrorPayload {
  kind: string;
  message: string;
}

export function isAppError(e: unknown): e is AppErrorPayload {
  return typeof e === 'object' && e !== null && 'kind' in e && 'message' in e;
}

export function errorMessage(e: unknown): string {
  // messages come from the core in Russian
  if (isAppError(e)) return tr(e.message);
  if (e instanceof Error) return tr(e.message);
  return tr(String(e));
}

export interface PendingLike {
  track: Track;
  liked: boolean;
}

export interface HomeCard {
  kind: 'mix' | 'playlist';
  id: number;
  urn: string | null;
  title: string;
  subtitle: string;
  artwork_url: string | null;
  track_count: number;
}

export interface HomeSection {
  title: string;
  cards: HomeCard[];
}

export interface MixPage {
  title: string;
  description: string;
  artwork_url: string | null;
  permalink_url: string | null;
  tracks: Track[];
}

export interface FeedItem {
  track: Track;
  reposted_by: string[];
  at: string;
}

export interface FeedPage {
  items: FeedItem[];
  next: string | null;
}

export interface ChartPage {
  title: string;
  note: string;
  tracks: Track[];
  missing: number;
}

export interface PendingFollow {
  user: User;
  follow: boolean;
}

export interface Download {
  track: Track;
  path: string;
  at: number;
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
  likesPending: () => invoke<PendingLike[]>('likes_pending'),
  likesPendingFlush: () => invoke<number>('likes_pending_flush'),
  captchaWaiting: () => invoke<boolean>('captcha_waiting'),
  downloads: () => invoke<Download[]>('downloads_list'),
  download: (track: Track) => invoke<Download>('download_track', { track }),
  downloadRemove: (trackId: number) => invoke<void>('download_remove', { trackId }),
  homeSections: (force: boolean) => invoke<HomeSection[]>('home_sections', { force }),
  categoryTracks: (key: string, queries: string[]) => invoke<Track[]>('category_tracks', { key, queries }),
  categoryCovers: (tiles: { key: string; mix: string | null; queries: string[] | null; pick: string | null; chart: string | null }[]) =>
    invoke<Record<string, string[]>>('category_covers', { tiles }),
  chartPage: (kind: string) => invoke<ChartPage>('chart_page', { kind }),
  chartYears: () => invoke<number[]>('chart_years'),
  picks: (kind: string) => invoke<Track[]>('picks', { kind }),
  mixPage: (urn: string) => invoke<MixPage>('mix_page', { urn }),
  feedPage: (next: string | null) => invoke<FeedPage>('feed_page', { next }),
  downloadsDir: () => invoke<string>('downloads_dir'),
  downloadsPickDir: () => invoke<string>('downloads_pick_dir'),
  followsPending: () => invoke<PendingFollow[]>('follows_pending'),
  downloadsOpen: (path?: string) => invoke<void>('downloads_open', { path: path ?? null }),
  libraryPlaylists: (force: boolean) => invoke<Playlist[]>('library_playlists', { force }),
  playlistAddTrack: (playlistId: number, trackId: number) => invoke<void>('playlist_add_track', { playlistId, trackId }),
  playlistRemoveTrack: (playlistId: number, trackId: number) => invoke<void>('playlist_remove_track', { playlistId, trackId }),
  playlistCreate: (title: string, trackId: number | null) => invoke<void>('playlist_create', { title, trackId }),
  uploadPick: (image: boolean) => invoke<PickedFile | null>('upload_pick', { image }),
  uploadTrack: (form: UploadForm) => invoke<Track>('upload_track', { form }),
  uploadDelete: (trackId: number) => invoke<void>('upload_delete', { trackId }),
  myTracks: () => invoke<MyTrack[]>('my_tracks'),
  libraryArtists: (force: boolean) => invoke<User[]>('library_artists', { force }),
  libraryCounts: () => invoke<LibraryCounts>('library_counts'),
  followSet: (user: User, follow: boolean) => invoke<void>('follow_set', { user, follow }),
  history: (offset: number, limit: number) => invoke<HistoryItem[]>('history_page', { offset, limit }),
  historyClear: () => invoke<void>('history_clear'),
  historyPlaylists: (limit: number) => invoke<{ playlist: Playlist; played_at: number }[]>('history_playlists', { limit }),
  historyPlaylistAdd: (playlist: Playlist) => invoke<void>('history_playlist_add', { playlist }),

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
  setSmartShuffle: (enabled: boolean) => invoke<void>('player_set_smart_shuffle', { enabled }),
  setRepeat: (mode: RepeatMode) => invoke<void>('player_set_repeat', { mode }),
  snapshot: () => invoke<PlayerSnapshot>('player_snapshot'),
  position: () => invoke<number>('player_position'),
  upcoming: (limit: number) => invoke<Track[]>('player_upcoming', { limit }),

  waveStart: () => invoke<number>('wave_start'),
  waveStartFrom: (trackId: number | null, artistId: number | null) =>
    invoke<number>('wave_start_from', { trackId, artistId }),
  waveStartPlaylist: (playlistId: number) => invoke<number>('wave_start_playlist', { playlistId }),
  waveSetMood: (mood: Mood) => invoke<void>('wave_set_mood', { mood }),
  waveInfo: (trackId: number | null) => invoke<WaveInfo>('wave_info', { trackId }),
  waveSetNoLiked: (enabled: boolean) => invoke<void>('wave_set_no_liked', { enabled }),
  dislikeSet: (trackId: number, disliked: boolean) => invoke<void>('dislike_set', { trackId, disliked }),
  dislikedIds: () => invoke<number[]>('disliked_ids'),
  dislikedTracks: () => invoke<Track[]>('disliked_tracks'),
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
  fxSet: (fx: FxConfig, persist: boolean) => invoke<void>('fx_set', { fx, persist }),
  sleepSet: (minutes: number | null, endOfTrack: boolean) => invoke<SleepState>('sleep_set', { minutes, endOfTrack }),
  sleepGet: () => invoke<SleepState>('sleep_get'),
  importPreview: (input: string) => invoke<[string, number]>('import_preview', { input }),
  importRun: (input: string, title: string, isPrivate: boolean) =>
    invoke<{ title: string; found: number; total: number; missing: string[] }>('import_run', { input, title, private: isPrivate }),
  statsGet: (period: 'week' | 'month' | 'year' | 'all') => invoke<Stats>('stats_get', { period }),
  queueGet: (limit: number) => invoke<QueueView>('queue_get', { limit }),
  queueMove: (from: number, to: number) => invoke<void>('queue_move', { from, to }),
  queueRemove: (at: number) => invoke<void>('queue_remove', { at }),
  queueClear: () => invoke<void>('queue_clear'),
  queuePlay: (at: number) => invoke<void>('queue_play', { at }),
  playlistSetTracks: (playlistId: number, trackIds: number[]) => invoke<void>('playlist_set_tracks', { playlistId, trackIds }),
  eqSet: (eq: EqConfig, persist: boolean) => invoke<void>('eq_set', { eq, persist }),
  discordSet: (discord: DiscordConfig) => invoke<void>('discord_set', { discord }),
  systemGet: () => invoke<SystemPrefs>('system_get'),
  newsSet: (enabled: boolean) => invoke<void>('news_set', { enabled }),
  systemSet: (autostart: boolean, startMinimized: boolean, fastProtected: boolean) =>
    invoke<void>('system_set', { autostart, startMinimized, fastProtected }),
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
  if (n >= 1_000_000) return T('{0} млн', (n / 1_000_000).toFixed(1).replace('.', ','));
  if (n >= 10_000) return T('{0} тыс.', (n / 1000).toFixed(1).replace('.', ','));
  return nf.format(n);
}

/** 1 трек, 2 трека, 5 треков (or the English pair) */
export { plural } from './i18n';
