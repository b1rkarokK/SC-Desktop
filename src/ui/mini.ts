// Mini player (C2), its own small window (?mini=1): a capsule with the cover,
// the karaoke line and ▶; on hover it grows downward with the seek bar and
// every control of the big player; ≡ opens the queue below.
// The window follows the page's height (mini_resize), so the transparent
// rest of the screen never catches clicks.
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { api, coverUrl, errorMessage, fmtTime, type Lyrics, type PlayerSnapshot, type RepeatMode, type Track } from './api';
import { clock } from './clock';
import { h, toast } from './dom';
import { T } from './i18n';
import { I, icon, iconButton, setIcon } from './icons';
import { lyricsData } from './lyrics_data';
import { loadPrefs } from './prefs';
import { QueuePopover } from './queue_popover';
import { Slider } from './slider';
import { store } from './store';

const POLL_MS = 250;
const COLLAPSE_AFTER_MS = 700;

export async function bootMini(): Promise<void> {
  document.documentElement.classList.add('mini');
  document.body.classList.add('mini');
  const cfg = await api.configGet();
  loadPrefs(cfg.ui);
  new MiniPlayer(document.body);
}

class MiniPlayer {
  private box = h('div', { class: 'mini-box' });
  private cover = h('img', { class: 'mini-cover', alt: '', 'data-tauri-drag-region': true });
  private line = h('div', { class: 'mini-line', dir: 'auto', 'data-tauri-drag-region': true });
  private sub = h('div', { class: 'mini-sub', dir: 'auto', 'data-tauri-drag-region': true });
  private playBtn = iconButton(I.play, T('Воспроизвести'), 'mini-play');
  private panel = h('div', { class: 'mini-panel' });
  private time = h('span', { class: 'mini-time', text: '0:00' });
  private duration = h('span', { class: 'mini-time', text: '0:00' });
  private seek: Slider;
  private volume: Slider;
  private shuffleBtn = iconButton(I.shuffle, T('Перемешать'));
  private repeatBtn = iconButton(I.repeat, T('Повтор'));
  private queueBtn = iconButton(I.queue, T('Очередь'));
  private queue = new QueuePopover();
  private expanded = false;
  private collapseTimer = 0;
  private poll = 0;
  private karaoke = 0;
  private lyrics: Lyrics | null = null;
  private lastLine = '';

  constructor(host: HTMLElement) {
    const prev = iconButton(I.prev, T('Предыдущий'));
    const next = iconButton(I.next, T('Следующий'));
    const restore = iconButton(I.exitFullscreen, T('Открыть большое окно'));
    const close = iconButton(I.close, T('Закрыть мини-плеер'));
    prev.addEventListener('click', () => void api.prev());
    next.addEventListener('click', () => void api.next());
    this.playBtn.addEventListener('click', () => void api.toggle());
    restore.addEventListener('click', () => void invoke('mini_close', { restore: true }));
    close.addEventListener('click', () => void invoke('mini_close', { restore: false }));
    this.shuffleBtn.addEventListener('click', () => {
      const s = store.snapshot;
      if (!s?.shuffle) void api.setShuffle(true);
      else if (!s.smart && s.source !== 'wave') void api.setSmartShuffle(true);
      else void api.setShuffle(false);
    });
    this.repeatBtn.addEventListener('click', () => {
      const m: RepeatMode = store.snapshot?.repeat ?? 'off';
      void api.setRepeat(m === 'off' ? 'all' : m === 'all' ? 'one' : 'off');
    });
    this.queueBtn.addEventListener('click', () => this.queue.toggle());
    this.queue.init((open) => {
      this.queueBtn.classList.toggle('is-on', open);
      this.fit();
    });

    this.seek = new Slider(
      T('Позиция'),
      (v) => (this.time.textContent = fmtTime(v * (store.snapshot?.track?.duration_ms ?? 0))),
      (v) => {
        const d = store.snapshot?.track?.duration_ms ?? 0;
        if (d > 0) {
          clock.reset(v * d);
          void api.seek(v * d).catch((e) => toast(errorMessage(e), 'error'));
        }
      },
    );
    this.volume = new Slider(
      T('Громкость'),
      (v) => void api.setVolume(v, false),
      (v) => void api.setVolume(v, true),
    );

    const top = h(
      'div',
      { class: 'mini-top', 'data-tauri-drag-region': true },
      this.cover,
      h('div', { class: 'mini-text', 'data-tauri-drag-region': true }, this.line, this.sub),
      h('div', { class: 'mini-corner' }, restore, close),
      this.playBtn,
    );
    this.panel.append(
      h('div', { class: 'mini-seek' }, this.time, this.seek.el, this.duration),
      h(
        'div',
        { class: 'mini-controls' },
        this.shuffleBtn,
        prev,
        this.playBtn.cloneNode(true) as HTMLElement,
        next,
        this.repeatBtn,
        this.queueBtn,
        h('div', { class: 'mini-vol' }, icon(I.vol), this.volume.el),
      ),
    );
    // the panel's own ▶ (the clone above only keeps the layout): wire it too
    const panelPlay = this.panel.querySelector('.mini-play') as HTMLButtonElement;
    panelPlay.addEventListener('click', () => void api.toggle());
    this.box.append(top, this.panel);
    this.queue.el.classList.add('mini-queue');
    host.append(this.box, this.queue.el);

    // double click on the capsule: back to the big window
    top.addEventListener('dblclick', (e) => {
      if (!(e.target as HTMLElement).closest('button')) void invoke('mini_close', { restore: true });
    });
    // grow on hover, shrink a moment after the pointer leaves
    document.body.addEventListener('mouseenter', () => this.setExpanded(true));
    document.body.addEventListener('mouseleave', () => {
      window.clearTimeout(this.collapseTimer);
      this.collapseTimer = window.setTimeout(() => {
        if (!this.queue.isOpen && !document.querySelector('.slider.is-dragging')) this.setExpanded(false);
      }, COLLAPSE_AFTER_MS);
    });
    document.body.addEventListener('mouseenter', () => window.clearTimeout(this.collapseTimer));
    // a lyric line wrapping to two lines makes the capsule taller: the window follows
    const grow = new ResizeObserver(() => this.fit());
    grow.observe(this.box);
    // the queue fills in after it opens: the window grows with it
    grow.observe(this.queue.el);

    void listen<PlayerSnapshot>('player:state', (e) => {
      store.setSnapshot(e.payload);
      this.render();
    });
    void listen<Track>('player:track', (e) => {
      clock.reset(0);
      lyricsData.setTrack(e.payload);
    });
    void listen<number>('player:seek', (e) => clock.reset(e.payload));
    void listen<{ id: number; ms: number }>('player:lyrics-shift', (e) => lyricsData.setShift(e.payload.id, e.payload.ms));
    lyricsData.on((s) => {
      this.lyrics = s.kind === 'ready' && s.lyrics.found && s.lyrics.synced ? s.lyrics : null;
      this.lastLine = '';
      this.renderLine();
    });
    lyricsData.setWanted(true);
    void api.snapshot().then((s) => {
      store.setSnapshot(s);
      lyricsData.setTrack(s.track);
      this.render();
    });
    this.setExpanded(false);
  }

  private setExpanded(on: boolean): void {
    if (this.expanded === on && this.box.classList.contains(on ? 'is-open' : 'is-closed')) return;
    this.expanded = on;
    this.box.classList.toggle('is-open', on);
    this.box.classList.toggle('is-closed', !on);
    if (!on) this.queue.setOpen(false);
    this.fit();
  }

  /** The window as tall as what is shown, so empty space never catches clicks. */
  private fit(): void {
    requestAnimationFrame(() => {
      const hgt = Math.ceil(this.box.getBoundingClientRect().height + (this.queue.isOpen ? this.queue.el.getBoundingClientRect().height + 6 : 0)) + 8;
      void invoke('mini_resize', { height: hgt });
    });
  }

  private render(): void {
    const s = store.snapshot;
    const t = s?.track ?? null;
    const src = coverUrl(t?.artwork_url, 't67x67');
    if (src) {
      if (this.cover.getAttribute('src') !== src) this.cover.src = src;
    } else this.cover.removeAttribute('src');
    this.sub.textContent = t ? `${t.title} · ${t.artist}` : '';
    const playing = !!s && (s.playing || s.loading);
    this.box.querySelectorAll<HTMLButtonElement>('.mini-play').forEach((b) => {
      setIcon(b, playing ? I.pause : I.play, 16);
      b.title = playing ? T('Пауза') : T('Воспроизвести');
    });
    this.shuffleBtn.classList.toggle('is-on', !!s?.shuffle);
    setIcon(this.shuffleBtn, s?.smart ? I.smartShuffle : I.shuffle);
    setIcon(this.repeatBtn, s?.repeat === 'one' ? I.repeatOne : I.repeat);
    this.repeatBtn.classList.toggle('is-on', !!s && s.repeat !== 'off');
    this.duration.textContent = fmtTime(t?.duration_ms ?? 0);
    if (!this.volume.isDragging) this.volume.set(s?.volume ?? 0.8);
    clock.set(s?.position_ms ?? 0, !!s?.playing && !s.loading);
    this.renderLine();
    this.updateTimers();
  }

  /** The karaoke line, or the title when the song has no synced lyrics. */
  private renderLine(): void {
    const t = store.snapshot?.track;
    if (!t) {
      this.line.textContent = T('Ничего не играет');
      return;
    }
    const lines = this.lyrics?.lines;
    if (!lines?.length) {
      if (this.lastLine !== `title:${t.id}`) {
        this.lastLine = `title:${t.id}`;
        this.line.replaceChildren(h('span', { class: 'mini-title', text: t.title }));
      }
      return;
    }
    const now = clock.now();
    let i = -1;
    for (let k = 0; k < lines.length; k++) if ((lines[k]!.start_ms ?? Infinity) <= now) i = k;
    const line = i >= 0 ? lines[i]! : null;
    if (!line || !line.text.trim()) {
      if (this.lastLine !== '♪') {
        this.lastLine = '♪';
        this.line.replaceChildren(h('span', { class: 'mini-title', text: t.title }));
      }
      return;
    }
    if (line.words.length) {
      const key = `${i}:${line.words.findIndex((w) => w.end_ms > now)}`;
      if (key === this.lastLine) return;
      this.lastLine = key;
      this.line.replaceChildren(
        ...line.words.map((w) =>
          h('span', { class: w.end_ms <= now ? 'w is-sung' : w.start_ms <= now ? 'w is-now' : 'w', text: w.text }),
        ),
      );
    } else if (this.lastLine !== String(i)) {
      this.lastLine = String(i);
      this.line.replaceChildren(h('span', { class: 'w is-now', text: line.text }));
    }
  }

  private updateTimers(): void {
    const playing = !!store.snapshot?.playing;
    if (playing && !this.poll) {
      this.poll = window.setInterval(() => {
        void api.position().then((ms) => {
          clock.set(ms, true);
          const d = store.snapshot?.track?.duration_ms ?? 0;
          if (!this.seek.isDragging) {
            this.seek.set(d ? ms / d : 0);
            this.time.textContent = fmtTime(ms);
          }
        });
      }, POLL_MS);
      // karaoke a little smoother than the position polling
      this.karaoke = window.setInterval(() => this.renderLine(), 100);
    } else if (!playing && this.poll) {
      window.clearInterval(this.poll);
      window.clearInterval(this.karaoke);
      this.poll = this.karaoke = 0;
    }
  }
}
