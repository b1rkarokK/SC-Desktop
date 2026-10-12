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
  theme: Theme;
  customThemes?: CustomTheme[];
  themeSystem: boolean;
  navCollapsed: boolean;
  lyricsOpen: boolean;
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
  return `${scope}{--bg:${t.bg};--panel:${t.panel};--text:${t.text};--accent:${t.accent};--on-accent:${readableOn(t.accent)};--strong:${t.base === 'light' ? '#000000' : '#ffffff'};--muted:color-mix(in srgb,${t.text} 62%,${t.bg});--faint:color-mix(in srgb,${t.text} 34%,${t.bg});--border:color-mix(in srgb,${t.text} 16%,${t.bg});--hover:color-mix(in srgb,${t.text} 9%,${t.panel});--tint:color-mix(in srgb,${t.text} 7%,transparent)}`;
}

function applyTheme(): void {
  document.documentElement.dataset.theme = effectiveTheme();
  let el = document.getElementById('custom-theme');
  const c = activeCustom();
  if (!c) {
    el?.remove();
    return;
  }
  if (!el) {
    el = document.createElement('style');
    el.id = 'custom-theme';
    document.head.append(el);
  }
  // after the built-in rules, so it wins over `[data-theme]` of equal weight
  el.textContent = customCss(c, ':root[data-theme]');
}

/** Try colours without saving them (the theme editor); `null` goes back to the saved look. */
export function previewTheme(t: CustomTheme | null): void {
  if (!t) return applyTheme();
  document.documentElement.dataset.theme = t.base;
  let el = document.getElementById('custom-theme');
  if (!el) {
    el = document.createElement('style');
    el.id = 'custom-theme';
    document.head.append(el);
  }
  el.textContent = customCss(t, ':root[data-theme]');
}
