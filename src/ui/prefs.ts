// Frontend-owned preferences, persisted in config.json → `ui` (debounced).
import { api } from './api';
import type { Lang } from './i18n';

export type BaseTheme = 'dark' | 'oled' | 'light';
/** a built-in theme, or `custom:<id>` of one the user made */
export type Theme = BaseTheme | `custom:${string}`;

/** A theme of the user's own: four colours, the rest is derived from them. */
export interface CustomTheme {
  id: string;
  name: string;
  /** light or dark: scrollbars, form controls and the few fixed shades follow it */
  base: 'dark' | 'light';
  bg: string;
  panel: string;
  text: string;
  accent: string;
  /** shades normally mixed from the four colours; set here they are used as they are */
  muted?: string;
  border?: string;
  hover?: string;
  /** corner radius of buttons, cards and fields, 0 … 16 px */
  radius?: number;
  /** a background picture is saved for this theme (it lives in the database, see `theme_bg_*`) */
  image?: boolean;
  /** darkening (or lightening, on a light base) laid over the picture, 0 … 0.85 */
  dim?: number;
  /** how opaque the side bar, title bar, text panel and player are over the picture, 40 … 100 % */
  shell?: number;
  /** the user's own CSS, applied on top of everything (no network: see `safeCss`) */
  css?: string;
}
export type FsLayout = 'side' | 'text' | 'cover';
export type FsBackground = 'theme' | 'cover' | 'oled';
export type Highlight = 'word' | 'line' | 'none';
export type VizMode = 'off' | 'bars' | 'mirror' | 'line' | 'wave' | 'ring';
export type VizColor = 'accent' | 'cover' | 'white';

export interface UiPrefs {
  /** mirrored for the core (tray, Discord); the UI reads it from localStorage */
  lang?: Lang;
  /** mini player opacity at rest, 0.1 … 1 */
  miniOpacity?: number;
  /** opacity of the lyric line in the mini player at rest (the capsule has its own) */
  miniTextOpacity?: number;
  /** the accent colour follows the cover of the playing track */
  accentFromCover?: boolean;
  /** the height of track rows in lists */
  density?: 'compact' | 'normal' | 'large';
  /** the lyrics panel shows a translation under each line */
  lyricsTranslate?: boolean;
  /** quieter at night: from the hour to the hour, by this share (the core eases the volume) */
  nightQuiet?: { on: boolean; from: number; to: number; percent: number };
  theme: Theme;
  customThemes?: CustomTheme[];
  themeSystem: boolean;
  /** lyrics are moved to the singing by the sound itself (see lyrics_align.rs), default on */
  lyricsAuto?: boolean;
  /** no lyrics anywhere: listen to the track and find the song by the words heard (hear.rs), default on */
  lyricsHear?: boolean;
  /** liked tracks are kept as files to play without the internet (downloads.rs), default off */
  offlineLikes?: boolean;
  /** the provider blocks SoundCloud: the TLS greeting is cut (net/bypass.rs); "auto" by default */
  scBypass?: 'auto' | 'on' | 'off';
  /** My Wave: only Russian / only foreign songs (by the letters of the title and the artist) */
  waveLang?: 'all' | 'ru' | 'foreign';
  /** My Wave: nothing that was ever played before */
  waveNoHeard?: boolean;
  /** what the user moved the lyrics of a track by, ms (kept per track) */
  lyricsNudge?: Record<string, number>;
  navCollapsed: boolean;
  lyricsOpen: boolean;
  /** width of the right lyrics panel, px (dragged by its left edge) */
  lyricsWidth?: number;
  /** text size of the right lyrics panel, px */
  lyricsSize?: number;
  syncedLyrics: boolean;
  wordHighlight: boolean;
  fs: {
    layout: FsLayout;
    background: FsBackground;
    highlight: Highlight;
    align: 'left' | 'center';
    size: number;
    /** ms added to the playback clock for lyrics (positive = text earlier) */
    offset: number;
    /** music visualizer: bars above the controls or waves behind the text */
    viz: VizMode;
    vizColor: VizColor;
  };
}

export const defaultPrefs: UiPrefs = {
  theme: 'dark',
  themeSystem: false,
  navCollapsed: false,
  lyricsOpen: true,
  syncedLyrics: true,
  wordHighlight: true,
  fs: { layout: 'side', background: 'theme', highlight: 'word', align: 'left', size: 24, offset: 0, viz: 'bars', vizColor: 'accent' },
};

let current: UiPrefs = structuredClone(defaultPrefs);
const listeners = new Set<(p: UiPrefs) => void>();
let saveTimer = 0;

export function prefs(): UiPrefs {
  return current;
}

export function loadPrefs(raw: Record<string, unknown> | undefined): void {
  const r = (raw ?? {}) as Partial<UiPrefs>;
  current = { ...structuredClone(defaultPrefs), ...r, fs: { ...defaultPrefs.fs, ...(r.fs ?? {}) } };
  applyTheme();
}

export function updatePrefs(patch: (p: UiPrefs) => void): void {
  patch(current);
  applyTheme();
  listeners.forEach((cb) => cb(current));
  window.clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => void api.configSetUi(current as unknown as Record<string, unknown>), 400);
}

export function onPrefs(cb: (p: UiPrefs) => void): void {
  listeners.add(cb);
}

const systemDark = window.matchMedia('(prefers-color-scheme: dark)');
systemDark.addEventListener('change', () => applyTheme());

export function customThemes(): CustomTheme[] {
  return current.customThemes ?? [];
}

/** The custom theme in use right now (none while "Как в системе" is on). */
function activeCustom(): CustomTheme | null {
  if (current.themeSystem || !current.theme.startsWith('custom:')) return null;
  const id = current.theme.slice(7);
  return customThemes().find((t) => t.id === id) ?? null;
}

export function effectiveTheme(): BaseTheme {
  if (current.themeSystem) return systemDark.matches ? (current.theme === 'oled' ? 'oled' : 'dark') : 'light';
  const c = activeCustom();
  if (c) return c.base;
  return current.theme.startsWith('custom:') ? 'dark' : (current.theme as BaseTheme);
}

/** Black or white text for a background colour (#rrggbb). */
function readableOn(hex: string): string {
  const n = parseInt(hex.slice(1), 16);
  const lum = (0.299 * ((n >> 16) & 255) + 0.587 * ((n >> 8) & 255) + 0.114 * (n & 255)) / 255;
  return lum > 0.6 ? '#111111' : '#ffffff';
}

/** CSS of a custom theme; the shades between the four colours are mixed from them. */
export function customCss(t: CustomTheme, scope = ':root'): string {
  const muted = t.muted ?? `color-mix(in srgb,${t.text} 62%,${t.bg})`;
  const border = t.border ?? `color-mix(in srgb,${t.text} 16%,${t.bg})`;
  const hover = t.hover ?? `color-mix(in srgb,${t.text} 9%,${t.panel})`;
  const radius = t.radius !== undefined ? `--radius:${Math.min(16, Math.max(0, Math.round(t.radius)))}px;` : '';
  const shell = `--shell-alpha:${Math.min(100, Math.max(40, Math.round(t.shell ?? 80)))}%;`;
  return `${scope}{--bg:${t.bg};--panel:${t.panel};--text:${t.text};--accent:${t.accent};--on-accent:${readableOn(t.accent)};--strong:${t.base === 'light' ? '#000000' : '#ffffff'};--muted:${muted};--faint:color-mix(in srgb,${t.text} 34%,${t.bg});--border:${border};--hover:${hover};--tint:color-mix(in srgb,${t.text} 7%,transparent);${radius}${shell}}`;
}

/** The user's CSS without what could load something from the network (`@import`, `url(http…)`) and capped in size:
 *  a theme is colours and shapes, it must not call anywhere or weigh on the computer. */
export function safeCss(css: string): string {
  return css
    .slice(0, 30_000)
    .replace(/@import[^;]*;?/gi, '/* @import is not allowed */')
    .replace(/url\(\s*(['"]?)\s*(?!data:)[^)]*\)/gi, 'none');
}

function styleEl(id: string): HTMLStyleElement {
  let el = document.getElementById(id) as HTMLStyleElement | null;
  if (!el) {
    el = document.createElement('style');
    el.id = id;
    document.head.append(el);
  }
  return el;
}

/** The picture behind the window: one fixed layer, painted once (no blur or filter at run time; any blur is
 *  baked into the picture when it is chosen), the shell panels let it through. */
function setBackdrop(t: CustomTheme | null, image: string | null): void {
  const root = document.documentElement;
  let el = document.getElementById('theme-bg');
  if (!t || !image) {
    el?.remove();
    delete root.dataset.bgimg;
    return;
  }
  if (!el) {
    el = document.createElement('div');
    el.id = 'theme-bg';
    document.body.prepend(el);
  }
  const dim = Math.min(0.85, Math.max(0, t.dim ?? 0.35));
  const veil = t.base === 'light' ? '255,255,255' : '0,0,0';
  el.style.backgroundImage = `linear-gradient(rgba(${veil},${dim}),rgba(${veil},${dim})),url("${image}")`;
  root.dataset.bgimg = '1';
}

let backdropSeq = 0;

/** Shows what `t` looks like with an optional picture override (`undefined`: the saved one, `null`: none). */
async function paintCustom(t: CustomTheme, image: string | null | undefined): Promise<void> {
  document.documentElement.dataset.theme = t.base;
  styleEl('custom-theme').textContent = customCss(t, ':root[data-theme]');
  styleEl('custom-theme-css').textContent = t.css ? safeCss(t.css) : '';
  const my = ++backdropSeq;
  let pic: string | null = null;
  if (image !== undefined) pic = image;
  else if (t.image) pic = await api.themeBgGet(t.id).catch(() => null);
  if (my === backdropSeq) setBackdrop(t, pic);
}

function applyTheme(): void {
  document.documentElement.dataset.theme = effectiveTheme();
  const c = activeCustom();
  if (!c) {
    document.getElementById('custom-theme')?.remove();
    document.getElementById('custom-theme-css')?.remove();
    backdropSeq++;
    setBackdrop(null, null);
    return;
  }
  void paintCustom(c, undefined);
}

/** Try a theme without saving it (the theme editor); `null` goes back to the saved look.
 *  `image`: the picture being tried (`undefined`: the saved one, `null`: none). */
export function previewTheme(t: CustomTheme | null, image?: string | null): void {
  if (!t) return applyTheme();
  void paintCustom(t, image);
}
