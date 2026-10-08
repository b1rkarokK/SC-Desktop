// Bottom player bar. Position is polled at 4 Hz only while playing AND the
// window is visible; in the tray there are zero timers on the JS side.
import { api, coverUrl, errorMessage, fmtTime, type RepeatMode } from './api';
import { clock } from './clock';
import { h, isVisible, onVisibilityChange, toast } from './dom';
import type { EqPopover } from './eq_popover';
import { iconButton, I, setIcon } from './icons';
import { onPrefs, prefs, updatePrefs } from './prefs';
import { openArtist, openTrack } from './router';
import { Slider } from './slider';
import { store } from './store';

const POLL_MS = 250;

export class PlayerBar {
  private cover = h('img', { class: 'pb-cover', alt: '', title: 'Открыть страницу трека' });
  private title = h('button', { type: 'button', class: 'pb-title link', dir: 'auto' });
  private artist = h('button', { type: 'button', class: 'pb-artist link', dir: 'auto' });
  private like = iconButton(I.heart, 'Лайкнуть');
  private dislike = iconButton(I.dislike, 'Не рекомендовать');
  private playBtn = iconButton(I.play, 'Воспроизвести', 'btn-play');
  private shuffleBtn = iconButton(I.shuffle, 'Перемешать');
  private repeatBtn = iconButton(I.repeat, 'Повтор');
  private time = h('span', { class: 'pb-time', text: '0:00' });
  private duration = h('span', { class: 'pb-time', text: '0:00' });
  private seek: Slider;
  private volume: Slider;
  private muteBtn = iconButton(I.vol, 'Без звука');
  private eqBtn = iconButton(I.eq, 'Эквалайзер');
  private fsBtn = iconButton(I.fullscreen, 'Полный экран');
  private lyricsBtn = iconButton(I.lyrics, 'Текст песни');
  private timer = 0;
  private lastVolume = 0.8;

  constructor(host: HTMLElement, eq: EqPopover, onFullscreen: () => void) {
    const prev = iconButton(I.prev, 'Предыдущий');
    const next = iconButton(I.next, 'Следующий');
    prev.addEventListener('click', () => void api.prev());
    next.addEventListener('click', () => void api.next());
    this.playBtn.addEventListener('click', () => void api.toggle());
    this.shuffleBtn.addEventListener('click', () => void api.setShuffle(!store.snapshot?.shuffle));
    this.repeatBtn.addEventListener('click', () => void api.setRepeat(nextRepeat(store.snapshot?.repeat ?? 'off')));
    const cur = () => store.snapshot?.track ?? null;
    this.like.addEventListener('click', () => {
      const t = cur();
      if (t) void store.setLiked(t, !store.liked.has(t.id));
    });
    this.dislike.addEventListener('click', () => {
      const t = cur();
      if (t) void store.setDisliked(t, !store.disliked.has(t.id));
    });
    this.title.addEventListener('click', () => cur() && openTrack(cur()!.id));
    this.cover.addEventListener('click', () => cur() && openTrack(cur()!.id));
    this.artist.addEventListener('click', () => cur() && openArtist(cur()!.user_id));

    this.seek = new Slider(
      'Позиция',
      (v) => (this.time.textContent = fmtTime(v * (cur()?.duration_ms ?? 0))),
      (v) => {
        const d = cur()?.duration_ms ?? 0;
        if (d > 0) {
          clock.reset(v * d);
          void api.seek(v * d).catch((e) => toast(errorMessage(e), 'error'));
        }
      },
    );
    this.volume = new Slider('Громкость', (v) => this.applyVolume(v, false), (v) => this.applyVolume(v, true));
    this.muteBtn.addEventListener('click', () => {
      const v = (store.snapshot?.volume ?? 0) > 0 ? 0 : this.lastVolume || 0.8;
      this.volume.set(v);
      this.applyVolume(v, true);
    });
    this.eqBtn.addEventListener('click', () => eq.toggle());
    this.fsBtn.addEventListener('click', onFullscreen);
    this.lyricsBtn.addEventListener('click', () => updatePrefs((p) => (p.lyricsOpen = !p.lyricsOpen)));
    onPrefs(() => this.renderLyricsBtn());
    this.renderLyricsBtn();

    host.append(
      h('div', { class: 'pb-now' }, this.cover, h('div', { class: 'pb-meta' }, this.title, this.artist), this.like, this.dislike),
      h(
        'div',
        { class: 'pb-center' },
        h('div', { class: 'pb-controls' }, this.shuffleBtn, prev, this.playBtn, next, this.repeatBtn),
        h('div', { class: 'pb-seek' }, this.time, this.seek.el, this.duration),
      ),
      h('div', { class: 'pb-right' }, this.eqBtn, this.muteBtn, h('div', { class: 'pb-volume' }, this.volume.el), this.fsBtn, this.lyricsBtn),
    );

    store.on('player', () => this.render());
    store.on('likes', () => this.renderFlags());
    store.on('dislikes', () => this.renderFlags());
    onVisibilityChange(() => this.updateTimer());
  }

  setEqActive(open: boolean, enabled: boolean): void {
    this.eqBtn.classList.toggle('is-on', open || enabled);
  }

  private renderLyricsBtn(): void {
    const open = prefs().lyricsOpen;
    this.lyricsBtn.classList.toggle('is-on', open);
    this.lyricsBtn.title = open ? 'Скрыть текст' : 'Показать текст';
  }

  private applyVolume(v: number, persist: boolean): void {
    if (v > 0) this.lastVolume = v;
    if (store.snapshot) store.snapshot.volume = v;
    this.renderMute(v);
    void api.setVolume(v, persist).catch((e) => toast(errorMessage(e), 'error'));
  }

  private renderMute(v: number): void {
    setIcon(this.muteBtn, v === 0 ? I.mute : v < 0.5 ? I.volLow : I.vol);
  }

  private render(): void {
    const s = store.snapshot;
    if (!s) return;
    const t = s.track;
    this.title.textContent = t ? t.title : 'Ничего не играет';
    this.title.disabled = !t;
    this.artist.textContent = t ? t.artist : '';
    this.artist.disabled = !t;
    const src = coverUrl(t?.artwork_url, 't67x67');
    if (src) {
      if (this.cover.getAttribute('src') !== src) this.cover.src = src;
      this.cover.hidden = false;
    } else {
      this.cover.removeAttribute('src');
      this.cover.hidden = !t;
    }
    this.duration.textContent = fmtTime(t?.duration_ms ?? 0);
    setIcon(this.playBtn, s.playing || s.loading ? I.pause : I.play);
    this.playBtn.title = s.loading ? 'Загрузка…' : s.playing ? 'Пауза' : 'Воспроизвести';
    this.playBtn.classList.toggle('is-loading', s.loading);
    this.shuffleBtn.classList.toggle('is-on', s.shuffle);
    setIcon(this.repeatBtn, s.repeat === 'one' ? I.repeatOne : I.repeat);
    this.repeatBtn.classList.toggle('is-on', s.repeat !== 'off');
    this.repeatBtn.title = { off: 'Повтор выключен', all: 'Повтор очереди', one: 'Повтор трека' }[s.repeat];
    if (!this.volume.isDragging) {
      this.volume.set(s.volume);
      this.renderMute(s.volume);
    }
    this.renderFlags();
    clock.set(s.position_ms, s.playing && !s.loading);
    this.renderPosition(s.position_ms);
    this.updateTimer();
  }

  private renderFlags(): void {
    const t = store.snapshot?.track;
    this.like.hidden = this.dislike.hidden = !t;
    const liked = !!t && store.liked.has(t.id);
    const disliked = !!t && store.disliked.has(t.id);
    this.like.classList.toggle('is-on', liked);
    this.like.title = liked ? 'Убрать из лайков' : 'Лайкнуть';
    this.dislike.classList.toggle('is-on', disliked);
    this.dislike.title = disliked ? 'Снова рекомендовать' : 'Не рекомендовать';
  }

  private renderPosition(ms: number): void {
    const d = store.snapshot?.track?.duration_ms ?? 0;
    if (!this.seek.isDragging) {
      this.seek.set(d > 0 ? ms / d : 0);
      this.time.textContent = fmtTime(ms);
    }
  }

  private poll = (): void => {
    api.position().then(
      (ms) => {
        const s = store.snapshot;
        clock.set(ms, !!s?.playing && !s.loading);
        this.renderPosition(ms);
      },
      () => undefined,
    );
  };

  private updateTimer(): void {
    const should = !!store.snapshot?.playing && isVisible();
    if (should && !this.timer) this.timer = window.setInterval(this.poll, POLL_MS);
    else if (!should && this.timer) {
      window.clearInterval(this.timer);
      this.timer = 0;
    }
    if (should) this.poll();
  }
}

function nextRepeat(m: RepeatMode): RepeatMode {
  return m === 'off' ? 'all' : m === 'all' ? 'one' : 'off';
}
