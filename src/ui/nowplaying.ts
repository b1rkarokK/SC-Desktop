// «Что играет» over a game (hotkey): the song, or the current and the next line of its lyrics, for a few seconds.
// A window of its own (nowplaying.rs) that takes no clicks and no focus; this is its page.
import { listen } from '@tauri-apps/api/event';
import { api, coverUrl, type LyricLine, type PlayerSnapshot } from './api';
import { h } from './dom';
import { T } from './i18n';
import { loadPrefs } from './prefs';

export async function bootNowPlaying(): Promise<void> {
  document.documentElement.classList.add('np');
  document.body.classList.add('np');
  try {
    loadPrefs((await api.configGet()).ui);
  } catch {
    /* the default look */
  }
  const box = h('div', { class: 'np-box' });
  document.body.append(box);
  let tick = 0;

  const song = (s: PlayerSnapshot) => {
    const img = h('img', { class: 'np-cover', alt: '' });
    const src = coverUrl(s.track?.artwork_url, 't67x67');
    if (src) img.src = src;
    return [
      img,
      h('div', { class: 'np-text' }, h('div', { class: 'np-title', dir: 'auto', text: s.track?.title ?? T('Ничего не играет') }), h('div', { class: 'np-artist', dir: 'auto', text: s.track?.artist ?? '' })),
    ];
  };

  const play = async (lyrics: boolean, ms: number) => {
    window.clearInterval(tick);
    const s = await api.snapshot();
    box.classList.remove('is-in');
    if (!s.track) {
      box.replaceChildren(h('div', { class: 'np-text' }, h('div', { class: 'np-title', text: T('Ничего не играет') })));
    } else if (!lyrics) {
      box.replaceChildren(...song(s));
    } else {
      let lines: LyricLine[] = [];
      try {
        const l = await api.lyrics(s.track, false);
        if (l.found && l.synced) lines = l.lines.filter((x) => x.start_ms !== null);
      } catch {
        /* no lyrics: the song is shown */
      }
      if (!lines.length) {
        box.replaceChildren(...song(s));
      } else {
        const cur = h('div', { class: 'np-line', dir: 'auto' });
        const next = h('div', { class: 'np-next', dir: 'auto' });
        box.replaceChildren(h('div', { class: 'np-text np-wide' }, cur, next));
        const base = await api.position(); // read once: the clock runs from there
        const at = performance.now();
        const paint = () => {
          const now = base + (s.playing ? (performance.now() - at) * (s.speed || 1) : 0);
          let i = -1;
          for (let k = 0; k < lines.length; k++) if ((lines[k]!.start_ms ?? Infinity) <= now) i = k;
          const c = i >= 0 ? lines[i]!.text : '';
          cur.textContent = c.trim() || '♪';
          next.textContent = lines[i + 1]?.text ?? '';
        };
        paint();
        tick = window.setInterval(paint, 150);
        window.setTimeout(() => window.clearInterval(tick), ms + 600);
      }
    }
    void box.offsetWidth; // restart the fade
    box.classList.add('is-in');
    window.setTimeout(() => box.classList.remove('is-in'), Math.max(500, ms - 400));
  };

  void listen<{ lyrics: boolean; ms: number }>('np:show', (e) => void play(e.payload.lyrics, e.payload.ms));
  // the first press made this window: what to show comes in the address
  const q = new URLSearchParams(location.search);
  void play(q.get('mode') === 'lyrics', Number(q.get('ms')) || 3500);
}
