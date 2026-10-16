// «Акцент из обложки» (Настройки → Тема): the accent colour of the app follows the cover of the playing track.
// The most colourful part of the cover (grey and black do not count) is taken, brought to a lightness that is seen
// on the theme's background, and eased in over a moment. Off: the theme's own accent comes back.
import { coverUrl } from './api';
import { onPrefs, prefs } from './prefs';
import { store } from './store';

let lastKey = '';

/** Average colour of the colourful pixels, or null for a grey cover. */
function colourful(img: HTMLImageElement): [number, number, number] | null {
  const c = document.createElement('canvas');
  c.width = c.height = 24;
  const ctx = c.getContext('2d', { willReadFrequently: true });
  if (!ctx) return null;
  ctx.drawImage(img, 0, 0, 24, 24);
  const d = ctx.getImageData(0, 0, 24, 24).data;
  let r = 0, g = 0, b = 0, w = 0;
  for (let i = 0; i < d.length; i += 4) {
    const [R, G, B] = [d[i]!, d[i + 1]!, d[i + 2]!];
    const max = Math.max(R, G, B);
    const min = Math.min(R, G, B);
    const sat = max === 0 ? 0 : (max - min) / max;
    const weight = sat * sat * (max / 255); // saturated and bright pixels lead
    r += R * weight;
    g += G * weight;
    b += B * weight;
    w += weight;
  }
  return w < 1.5 ? null : [r / w, g / w, b / w];
}

function toHsl([r, g, b]: [number, number, number]): [number, number, number] {
  r /= 255;
  g /= 255;
  b /= 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  if (max === min) return [0, 0, l];
  const d = max - min;
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  let h = 0;
  if (max === r) h = (g - b) / d + (g < b ? 6 : 0);
  else if (max === g) h = (b - r) / d + 2;
  else h = (r - g) / d + 4;
  return [h * 60, s, l];
}

function hslToHex(h: number, s: number, l: number): string {
  const a = s * Math.min(l, 1 - l);
  const f = (n: number) => {
    const k = (n + h / 30) % 12;
    const v = l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1));
    return Math.round(v * 255).toString(16).padStart(2, '0');
  };
  return `#${f(0)}${f(8)}${f(4)}`;
}

function readableOn(hex: string): string {
  const n = parseInt(hex.slice(1), 16);
  const lum = (0.299 * ((n >> 16) & 255) + 0.587 * ((n >> 8) & 255) + 0.114 * (n & 255)) / 255;
  return lum > 0.6 ? '#111111' : '#ffffff';
}

function clear(): void {
  const st = document.documentElement.style;
  st.removeProperty('--accent');
  st.removeProperty('--on-accent');
  lastKey = '';
}

function paint(): void {
  if (!prefs().accentFromCover) return clear();
  const t = store.snapshot?.track;
  const src = coverUrl(t?.artwork_url, 't67x67');
  const key = src ?? '';
  if (key === lastKey) return;
  lastKey = key;
  if (!src) return clear();
  const img = new Image();
  img.crossOrigin = 'anonymous';
  img.onload = () => {
    if (key !== lastKey || !prefs().accentFromCover) return;
    try {
      const rgb = colourful(img);
      if (!rgb) return clear(); // a grey cover: the theme's own accent
      let [h, s, l] = toHsl(rgb);
      const light = document.documentElement.dataset.theme === 'light';
      s = Math.min(0.85, Math.max(0.5, s));
      // seen on the background: lighter on dark themes, deeper on the light one
      l = light ? Math.min(0.46, Math.max(0.32, l)) : Math.min(0.66, Math.max(0.5, l));
      const hex = hslToHex(h, s, l);
      const st = document.documentElement.style;
      st.setProperty('--accent', hex);
      st.setProperty('--on-accent', readableOn(hex));
    } catch {
      clear(); // a canvas the cover taints: leave the theme's accent
    }
  };
  img.onerror = () => clear();
  img.src = src;
}

export function initCoverAccent(): void {
  store.on('player', paint);
  onPrefs(() => {
    lastKey = '';
    paint();
  });
  paint();
}
