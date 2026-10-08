// Karaoke renderer shared by the side panel and the fullscreen view.
// The 12 Hz loop runs only while: lyrics are synced, the view is visible and music plays.
import { api, type LyricLine, type Lyrics, type Word } from './api';
import { clock } from './clock';
import { h, isVisible, onVisibilityChange } from './dom';
import type { Highlight } from './prefs';
import { store } from './store';

interface RenderedLine {
  el: HTMLElement;
  line: LyricLine;
  /** word spans + their timings (real or estimated) */
  words: { el: HTMLElement; w: Word }[];
}

const USER_SCROLL_PAUSE_MS = 4000;
const TICK_MS = 80;

export class LyricsView {
  readonly el: HTMLDivElement;
  private lines: RenderedLine[] = [];
  private lyrics: Lyrics | null = null;
  private activeLine = -1;
  private activeWord = -1;
  private frame = 0;
  private shown = true;
  private userScrollUntil = 0;
  highlight: Highlight = 'word';
  offset = 0;

  constructor(className: string) {
    this.el = h('div', { class: `lyrics-view ${className}` });
    this.el.addEventListener('wheel', () => (this.userScrollUntil = performance.now() + USER_SCROLL_PAUSE_MS), { passive: true });
    this.el.addEventListener('click', (e) => {
      const lineEl = (e.target as HTMLElement).closest('[data-line]') as HTMLElement | null;
      const line = lineEl ? this.lines[Number(lineEl.dataset.line)]?.line : undefined;
      if (line?.start_ms !== null && line?.start_ms !== undefined && this.lyrics?.synced) {
        void api.seek(Math.max(0, line.start_ms - this.offset));
        clock.reset(line.start_ms - this.offset);
        this.userScrollUntil = 0;
      }
    });
    clock.on(() => this.kick());
    store.on('player', () => this.kick());
    onVisibilityChange(() => this.kick());
  }

  setShown(shown: boolean): void {
    this.shown = shown;
    this.kick();
  }

  setMessage(text: string): void {
    this.lyrics = null;
    this.lines = [];
    this.el.replaceChildren(h('div', { class: 'empty', text }));
    this.stop();
  }

  setLyrics(l: Lyrics): void {
    this.lyrics = l;
    this.render();
  }

  rerender(): void {
    if (this.lyrics) this.render();
  }

  private render(): void {
    const l = this.lyrics!;
    this.activeLine = -1;
    this.activeWord = -1;
    const frag = document.createDocumentFragment();
    this.lines = [];
    const wordMode = l.synced && this.highlight === 'word';
    l.lines.forEach((line, i) => {
      const isSection = /^\s*\[.*\]\s*$/.test(line.text) || /^\s*\(.*\)\s*$/.test(line.text) && !l.synced;
      const el = h('div', { class: `ly-line${isSection ? ' ly-section' : ''}${line.text.trim() ? '' : ' ly-gap'}`, dir: 'auto', 'data-line': i });
      const words: RenderedLine['words'] = [];
      if (wordMode && line.text.trim() && line.start_ms !== null) {
        for (const w of line.words.length ? line.words : estimateWords(line)) {
          const span = h('span', { class: 'w', text: w.text });
          el.append(span);
          words.push({ el: span, w });
        }
      } else {
        el.textContent = line.text || ' ';
      }
      if (!l.synced) el.classList.add('ly-static');
      frag.append(el);
      this.lines.push({ el, line, words });
    });
    this.el.replaceChildren(frag);
    this.el.scrollTop = 0;
    this.kick();
  }

  /** (Re)start or stop the animation loop depending on conditions. */
  private kick(): void {
    const run = !!this.lyrics?.synced && this.shown && isVisible() && this.highlight !== 'none';
    if (!run) {
      this.stop();
      if (this.lyrics?.synced && this.highlight !== 'none') this.tick(); // static frame (paused)
      return;
    }
    this.tick();
    if (clock.playing && !this.frame) this.loop();
    if (!clock.playing) this.stop();
  }

  /** ~12 updates/s: enough for word highlighting, far cheaper than 60 fps rAF. */
  private loop = (): void => {
    this.frame = window.setTimeout(() => {
      this.frame = 0;
      this.tick();
      if (clock.playing && this.shown && isVisible()) this.loop();
    }, TICK_MS);
  };

  private stop(): void {
    if (this.frame) window.clearTimeout(this.frame);
    this.frame = 0;
  }

  private tick(): void {
    if (!this.lyrics?.synced || !this.lines.length) return;
    const t = clock.now() + this.offset;
    const idx = findLine(this.lines, t);
    if (idx !== this.activeLine) {
      this.lines.forEach((r, i) => {
        r.el.classList.toggle('is-past', i < idx);
        r.el.classList.toggle('is-current', i === idx);
      });
      if (this.activeLine >= 0) this.lines[this.activeLine]?.words.forEach((x) => x.el.classList.remove('is-sung', 'is-now'));
      this.activeLine = idx;
      this.activeWord = -1;
      this.scrollTo(idx);
    }
    if (this.highlight !== 'word' || idx < 0) return;
    const words = this.lines[idx]!.words;
    let wi = -1;
    for (let k = 0; k < words.length; k++) if (words[k]!.w.start_ms <= t) wi = k;
    if (wi !== this.activeWord) {
      words.forEach((x, k) => {
        x.el.classList.toggle('is-sung', k < wi);
        x.el.classList.toggle('is-now', k === wi);
      });
      this.activeWord = wi;
    }
  }

  private scrollTo(idx: number): void {
    if (idx < 0 || performance.now() < this.userScrollUntil) return;
    const el = this.lines[idx]!.el;
    const target = el.offsetTop - this.el.clientHeight * 0.4 + el.offsetHeight / 2;
    this.el.scrollTo({ top: Math.max(0, target), behavior: 'smooth' });
  }
}

function findLine(lines: RenderedLine[], t: number): number {
  let lo = 0;
  let hi = lines.length - 1;
  let ans = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const s = lines[mid]!.line.start_ms;
    if (s !== null && s <= t) {
      ans = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  // skip empty "gap" lines: highlight nothing during instrumental breaks
  if (ans >= 0 && !lines[ans]!.line.text.trim()) return -1;
  return ans;
}

/** Line-level lyrics → word timings distributed by character length. */
function estimateWords(line: LyricLine): Word[] {
  const start = line.start_ms ?? 0;
  const end = Math.max(start + 400, Math.min(line.end_ms ?? start + 4000, start + 8000));
  const tokens = line.text.match(/\S+\s*/g) ?? [line.text];
  const total = tokens.reduce((n, t) => n + t.trim().length + 1, 0);
  // sing over ~90% of the line, leave a little tail
  const span = (end - start) * 0.9;
  let acc = start;
  return tokens.map((tok) => {
    const d = (span * (tok.trim().length + 1)) / total;
    const w = { start_ms: Math.round(acc), end_ms: Math.round(acc + d), text: tok };
    acc += d;
    return w;
  });
}
