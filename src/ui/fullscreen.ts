// «Сейчас играет» fullscreen (mockups 2 and 3) with the «Вид» settings drawer.
import { api, coverUrl, errorMessage, fmtTime } from './api';
import { clock } from './clock';
import { h, isVisible, onVisibilityChange, toast } from './dom';
import { iconButton, I, setIcon } from './icons';
import { LyricsView } from './lyrics_view';
import { lyricsMenu, trackMenu } from './menus';
import { lyricsData, SOURCE_NAMES, type LyricsState } from './lyrics_data';
import { onPrefs, prefs, updatePrefs, type FsBackground, type FsLayout, type Highlight } from './prefs';
import { openTrack, openTrackArtist } from './router';
import { Slider } from './slider';
import { store } from './store';
import { T } from './i18n';
import { windowControls } from './titlebar';

export class Fullscreen {
  readonly el: HTMLDivElement;
  private view = new LyricsView('ly-fs');
  private cover = h('img', { class: 'fs-cover', alt: '', crossorigin: 'anonymous' });
  private title = h('button', { type: 'button', class: 'fs-title link', dir: 'auto' });
  private artist = h('button', { type: 'button', class: 'fs-artist link', dir: 'auto' });
  private like = iconButton(I.heart, T('Лайкнуть'));
  private dislike = iconButton(I.dislike, T('Не рекомендовать'));
  private context = h('span', { class: 'muted small' });
  private playBtn = iconButton(I.play, T('Воспроизвести'), 'btn-play btn-play-lg');
  private shuffleBtn = iconButton(I.shuffle, T('Перемешать'));
  private repeatBtn = iconButton(I.repeat, T('Повтор'));
  private muteBtn = iconButton(I.vol, T('Без звука'));
  private volume!: Slider;
  private seek: Slider;
  private time = h('span', { class: 'pb-time' });
  private dur = h('span', { class: 'pb-time' });
  private drawer: HTMLElement;
  private open = false;
  private coverColor: string | null = null;
  private timer = 0;

  constructor() {
    const exit = iconButton(I.exitFullscreen, T('Свернуть (Esc)'));
    exit.addEventListener('click', () => this.setOpen(false));
    const paletteBtn = iconButton(I.palette, T('Вид полного экрана'));
    paletteBtn.addEventListener('click', () => (this.drawer.hidden = !this.drawer.hidden));
    const prev = iconButton(I.prev, T('Предыдущий'));
    const next = iconButton(I.next, T('Следующий'));
    prev.addEventListener('click', () => void api.prev());
    next.addEventListener('click', () => void api.next());
    this.playBtn.addEventListener('click', () => void api.toggle());
    const t = () => store.snapshot?.track ?? null;
    this.title.addEventListener('click', () => t() && (this.setOpen(false), openTrack(t()!.id)));
    this.artist.addEventListener('click', () => t() && (this.setOpen(false), void openTrackArtist(t()!)));
    this.like.addEventListener('click', () => t() && void store.setLiked(t()!, !store.liked.has(t()!.id)));
    this.dislike.addEventListener('click', () => t() && void store.setDisliked(t()!, !store.disliked.has(t()!.id)));
    this.seek = new Slider(
      T('Позиция'),
      (v) => (this.time.textContent = fmtTime(v * (t()?.duration_ms ?? 0))),
      (v) => {
        const d = t()?.duration_ms ?? 0;
        if (d) {
          clock.reset(v * d);
          void api.seek(v * d).catch((e) => toast(errorMessage(e), 'error'));
        }
      },
    );
    this.cover.addEventListener('load', () => this.sampleColor());
    this.shuffleBtn.addEventListener('click', () => void api.setShuffle(!store.snapshot?.shuffle));
    this.repeatBtn.addEventListener('click', () => {
      const m = store.snapshot?.repeat ?? 'off';
      void api.setRepeat(m === 'off' ? 'all' : m === 'all' ? 'one' : 'off');
    });
    this.volume = new Slider(
      T('Громкость'),
      (v) => void api.setVolume(v, false),
      (v) => void api.setVolume(v, true),
    );
    this.muteBtn.addEventListener('click', () => {
      const v = (store.snapshot?.volume ?? 0) > 0 ? 0 : 0.8;
      this.volume.set(v);
      void api.setVolume(v, true);
    });

    this.drawer = this.buildDrawer();
    this.drawer.hidden = true;

    this.el = h(
      'div',
      { class: 'fs', role: 'dialog', 'aria-label': T('Сейчас играет') },
      h('div', { class: 'fs-top', 'data-tauri-drag-region': true }, exit, h('span', { class: 'muted small', text: T('Сейчас играет') }), this.context, h('div', { class: 'spacer' }), paletteBtn, windowControls()),
      h(
        'div',
        { class: 'fs-main' },
        h(
          'div',
          { class: 'fs-art' },
          this.cover,
          h('div', { class: 'fs-meta' }, h('div', { class: 'fs-meta-text' }, this.title, this.artist), this.dislike, this.like),
        ),
        this.view.el,
      ),
      h(
        'div',
        { class: 'fs-bottom' },
        h('div', { class: 'pb-seek' }, this.time, this.seek.el, this.dur),
        h(
          'div',
          { class: 'fs-controls' },
          h('div', { class: 'fs-side' }),
          h('div', { class: 'pb-controls' }, this.shuffleBtn, prev, this.playBtn, next, this.repeatBtn),
          h('div', { class: 'fs-side fs-vol' }, this.muteBtn, this.volume.el),
        ),
      ),
      this.drawer,
    );
    this.el.hidden = true;
    this.el.querySelector('.fs-art')!.addEventListener('contextmenu', (e) => {
      const tr = t();
      if (tr) trackMenu(e as MouseEvent, tr, () => void api.toggle());
    });
    this.view.el.addEventListener('contextmenu', (e) => this.lyricsContext(e));

    lyricsData.on((s) => this.renderLyrics(s));
    store.on('player', () => this.renderTrack());
    store.on('likes', () => this.renderFlags());
    store.on('dislikes', () => this.renderFlags());
    onPrefs(() => this.applyPrefs());
    onVisibilityChange(() => this.updateTimer());
    document.addEventListener('keydown', (e) => {
      if (e.key === 'Escape' && this.open) {
        if (!this.drawer.hidden) this.drawer.hidden = true;
        else this.setOpen(false);
      }
    });
    this.applyPrefs();
  }

  toggle(): void {
    this.setOpen(!this.open);
  }

  setOpen(open: boolean): void {
    this.open = open;
    this.el.hidden = !open;
    document.body.classList.toggle('fs-open', open);
    this.view.setShown(open);
    lyricsData.setWanted(open || prefs().lyricsOpen);
    if (open) {
      this.renderTrack();
      this.renderLyrics(lyricsData.state);
    }
    this.updateTimer();
  }

  // ----------------------------------------------------------- render

  private applyPrefs(): void {
    const f = prefs().fs;
    this.el.dataset.layout = f.layout;
    this.el.dataset.align = f.align;
    this.el.style.setProperty('--fs-size', `${f.size}px`);
    const hl: Highlight = !prefs().syncedLyrics ? 'none' : f.highlight;
    if (hl !== this.view.highlight || f.offset !== this.view.offset) {
      this.view.highlight = hl;
      this.view.offset = f.offset;
      this.view.rerender();
    }
    this.applyBackground();
    this.renderDrawer();
  }

  private applyBackground(): void {
    const bg = prefs().fs.background;
    this.el.dataset.bg = bg;
    if (bg === 'cover' && this.coverColor) this.el.style.setProperty('--fs-bg', this.coverColor);
    else this.el.style.removeProperty('--fs-bg');
  }

  private renderTrack(): void {
    const s = store.snapshot;
    const t = s?.track;
    this.title.textContent = t?.title ?? T('Ничего не играет');
    this.artist.textContent = t?.artist ?? '';
    this.dur.textContent = fmtTime(t?.duration_ms ?? 0);
    setIcon(this.playBtn, s?.playing || s?.loading ? I.pause : I.play, 20);
    this.shuffleBtn.classList.toggle('is-on', !!s?.shuffle);
    setIcon(this.repeatBtn, s?.repeat === 'one' ? I.repeatOne : I.repeat);
    this.repeatBtn.classList.toggle('is-on', !!s && s.repeat !== 'off');
    if (s && !this.volume.isDragging) {
      this.volume.set(s.volume);
      setIcon(this.muteBtn, s.volume === 0 ? I.mute : s.volume < 0.5 ? I.volLow : I.vol);
    }
    const src = coverUrl(t?.artwork_url, 't500x500');
    if (src && this.cover.getAttribute('src') !== src) {
      this.coverColor = null;
      this.cover.src = src;
    }
    this.cover.hidden = !src;
    this.context.textContent = s?.source === 'wave' ? T('· Моя волна') : s?.source === 'likes' ? T('· Лайки') : '';
    this.renderFlags();
    this.renderPos(clock.now());
    this.updateTimer();
  }

  private renderFlags(): void {
    const t = store.snapshot?.track;
    this.like.classList.toggle('is-on', !!t && store.liked.has(t.id));
    this.dislike.classList.toggle('is-on', !!t && store.disliked.has(t.id));
  }

  private renderPos(ms: number): void {
    const d = store.snapshot?.track?.duration_ms ?? 0;
    if (!this.seek.isDragging) {
      this.seek.set(d ? ms / d : 0);
      this.time.textContent = fmtTime(ms);
    }
  }

  private updateTimer(): void {
    const should = this.open && !!store.snapshot?.playing && isVisible();
    if (should && !this.timer) this.timer = window.setInterval(() => this.renderPos(clock.now()), 250);
    else if (!should && this.timer) {
      window.clearInterval(this.timer);
      this.timer = 0;
    }
  }

  private renderLyrics(s: LyricsState): void {
    if (s.kind === 'ready' && s.lyrics.found) {
      this.view.setLyrics(s.lyrics);
      const l = s.lyrics;
      this.el.dataset.source = `${SOURCE_NAMES[l.source ?? ''] ?? ''}`;
    } else if (s.kind === 'loading') this.view.setMessage(T('Ищем текст…'));
    else if (s.kind === 'error') this.view.setMessage(s.message);
    else if (s.kind === 'ready') this.view.setMessage(T('Текст не найден.'));
    else this.view.setMessage('');
  }

  private lyricsContext(e: MouseEvent): void {
    const s = lyricsData.state;
    const l = s.kind === 'ready' && s.lyrics.found ? s.lyrics : null;
    lyricsMenu(e, l ? l.lines.map((x) => x.text).join('\n') : null, l?.url ?? null, () => lyricsData.reload());
  }

  /** Average cover colour, darkened so white text stays readable. */
  private sampleColor(): void {
    try {
      const c = document.createElement('canvas');
      c.width = c.height = 16;
      const ctx = c.getContext('2d', { willReadFrequently: true });
      if (!ctx) return;
      ctx.drawImage(this.cover, 0, 0, 16, 16);
      const d = ctx.getImageData(0, 0, 16, 16).data;
      let r = 0, g = 0, b = 0;
      for (let i = 0; i < d.length; i += 4) {
        r += d[i]!;
        g += d[i + 1]!;
        b += d[i + 2]!;
      }
      const n = d.length / 4;
      const k = 0.32; // darken
      this.coverColor = `rgb(${Math.round((r / n) * k)}, ${Math.round((g / n) * k)}, ${Math.round((b / n) * k)})`;
      this.applyBackground();
    } catch {
      this.coverColor = null; // tainted canvas — keep theme background
    }
  }

  // ----------------------------------------------------------- drawer

  private segEls = new Map<string, HTMLButtonElement[]>();
  private sizeSlider!: Slider;
  private sizeVal = h('span', { class: 'small' });
  private offsetVal = h('span', { class: 'small' });

  private buildDrawer(): HTMLElement {
    const close = iconButton(I.close, T('Закрыть'));
    close.addEventListener('click', () => (this.drawer.hidden = true));
    const seg = <T extends string>(key: string, label: string, items: [T, string][], set: (v: T) => void) => {
      const btns = items.map(([v, text]) => {
        const b = h('button', { type: 'button', class: 'seg-tab', 'data-v': v, text });
        b.addEventListener('click', () => updatePrefs(() => set(v)));
        return b;
      });
      this.segEls.set(key, btns);
      return h('div', { class: 'dr-row' }, h('div', { class: 'muted small', text: label }), h('div', { class: 'seg seg-full' }, ...btns));
    };
    this.sizeSlider = new Slider(
      T('Размер текста'),
      (v) => (this.sizeVal.textContent = String(Math.round(16 + v * 32))),
      (v) => updatePrefs((p) => (p.fs.size = Math.round(16 + v * 32))),
    );
    const minus = h('button', { type: 'button', class: 'btn btn-square', text: '−', title: T('Текст позже') });
    const plus = h('button', { type: 'button', class: 'btn btn-square', text: '+', title: T('Текст раньше') });
    minus.addEventListener('click', () => updatePrefs((p) => (p.fs.offset -= 250)));
    plus.addEventListener('click', () => updatePrefs((p) => (p.fs.offset += 250)));
    return h(
      'div',
      { class: 'fs-drawer panel' },
      h('div', { class: 'dr-head' }, h('span', { class: 'strong', text: T('Вид полного экрана') }), h('div', { class: 'spacer' }), close),
      seg<FsLayout>('layout', T('Раскладка'), [['side', T('Рядом')], ['text', T('Текст')], ['cover', T('Обложка')]], (v) => (prefs().fs.layout = v)),
      seg<FsBackground>('background', T('Фон'), [['theme', T('Тема')], ['cover', T('По обложке')], ['oled', 'OLED']], (v) => (prefs().fs.background = v)),
      seg<Highlight>('highlight', T('Подсветка'), [['word', T('Слово')], ['line', T('Строка')], ['none', T('Нет')]], (v) => (prefs().fs.highlight = v)),
      seg<'left' | 'center'>('align', T('Выравнивание'), [['left', T('Слева')], ['center', T('По центру')]], (v) => (prefs().fs.align = v)),
      h('div', { class: 'dr-row' }, h('div', { class: 'muted small', text: T('Размер текста') }), h('div', { class: 'dr-inline' }, this.sizeSlider.el, this.sizeVal)),
      h('div', { class: 'dr-row' }, h('div', { class: 'muted small', text: T('Сдвиг текста (если отстаёт или спешит)') }), h('div', { class: 'dr-inline' }, minus, this.offsetVal, plus)),
    );
  }

  private renderDrawer(): void {
    if (!this.sizeSlider) return;
    const f = prefs().fs;
    const cur: Record<string, string> = { layout: f.layout, background: f.background, highlight: f.highlight, align: f.align };
    for (const [key, btns] of this.segEls) btns.forEach((b) => b.classList.toggle('is-active', b.dataset.v === cur[key]));
    this.sizeSlider.set((f.size - 16) / 32);
    this.sizeVal.textContent = String(f.size);
    this.offsetVal.textContent = T('{0}{1} с', f.offset > 0 ? '+' : f.offset < 0 ? '−' : '', (Math.abs(f.offset) / 1000).toFixed(2).replace('.', ','));
  }
}
