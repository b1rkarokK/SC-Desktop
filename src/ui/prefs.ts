// Frontend-owned preferences, persisted in config.json → `ui` (debounced).
import { api } from './api';

export type Theme = 'dark' | 'oled' | 'light';
export type FsLayout = 'side' | 'text' | 'cover';
export type FsBackground = 'theme' | 'cover' | 'oled';
export type Highlight = 'word' | 'line' | 'none';

export interface UiPrefs {
  theme: Theme;
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
  };
}

export const defaultPrefs: UiPrefs = {
  theme: 'dark',
  themeSystem: false,
  navCollapsed: false,
  lyricsOpen: true,
  syncedLyrics: true,
  wordHighlight: true,
  fs: { layout: 'side', background: 'theme', highlight: 'word', align: 'left', size: 24, offset: 0 },
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

export function effectiveTheme(): Theme {
  if (current.themeSystem) return systemDark.matches ? (current.theme === 'oled' ? 'oled' : 'dark') : 'light';
  return current.theme;
}

function applyTheme(): void {
  document.documentElement.dataset.theme = effectiveTheme();
}
