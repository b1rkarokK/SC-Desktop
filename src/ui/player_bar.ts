// Bottom player bar. Position is polled at 4 Hz only while playing AND the
// window is visible; in the tray there are zero timers on the JS side.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { api, coverUrl, errorMessage, fmtTime, type RepeatMode, type SleepState } from './api';
import { openMenu, type MenuItem } from './context_menu';
import { askSleepTime } from './sleep_dialog';
import { clock } from './clock';
import { h, isVisible, onVisibilityChange, toast } from './dom';
import type { EqPopover } from './eq_popover';
import type { QueuePopover } from './queue_popover';
import { iconButton, I, setIcon } from './icons';
import { trackMenu } from './menus';
import { onPrefs, prefs, updatePrefs } from './prefs';
import { openTrack, openTrackArtist } from './router';
import { Slider } from './slider';
import { store } from './store';
import { T } from './i18n';

const POLL_MS = 250;

export class PlayerBar {
  private cover = h('img', { class: 'pb-cover', alt: '', title: T('Открыть страницу трека') });
  private title = h('button', { type: 'button', class: 'pb-title link', dir: 'auto' });
  private artist = h('button', { type: 'button', class: 'pb-artist link', dir: 'auto' });
  private like = iconButton(I.heart, T('Лайкнуть'));
  private dislike = iconButton(I.dislike, T('Не рекомендовать'));
  private playBtn = iconButton(I.play, T('Воспроизвести'), 'btn-play');
  private shuffleBtn = iconButton(I.shuffle, T('Перемешать'));
  private repeatBtn = iconButton(I.repeat, T('Повтор'));
  private time = h('span', { class: 'pb-time', text: '0:00' });
  private duration = h('span', { class: 'pb-time', text: '0:00' });
  private seek: Slider;
  private volume: Slider;
  private muteBtn = iconButton(I.vol, T('Без звука'));
  private eqBtn = iconButton(I.eq, T('Эквалайзер'));
  private sleepBtn = iconButton(I.sleep, T('Таймер сна'));
  private queueBtn = iconButton(I.queue, T('Очередь'));
  private miniBtn = iconButton(I.mini, T('Мини-плеер'));
  private sleepTimer = 0;
  private fsBtn = iconButton(I.fullscreen, T('Полный экран'));
  private lyricsBtn = iconButton(I.lyrics, T('Текст песни'));
  private timer = 0;
  private lastVolume = 0.8;

  constructor(host: HTMLElement, eq: EqPopover, queue: QueuePopover, onFullscreen: () => void) {
    const prev = iconButton(I.prev, T('Предыдущий'));
    const next = iconButton(I.next, T('Следующий'));
    prev.addEventListener('click', () => void api.prev());
    next.addEventListener('click', () => void api.next());
    this.playBtn.addEventListener('click', () => void api.toggle());
    // off → shuffle → smart shuffle (recommendations mixed in) → off; the wave has its own order
    this.shuffleBtn.addEventListener('click', () => {
      const s = store.snapshot;
      if (!s?.shuffle) void api.setShuffle(true);
      else if (!s.smart && s.source !== 'wave') void api.setSmartShuffle(true);
      else void api.setShuffle(false);
    });
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
    this.artist.addEventListener('click', () => cur() && void openTrackArtist(cur()!));

    this.seek = new Slider(
      T('Позиция'),
      (v) => (this.time.textContent = fmtTime(v * (cur()?.duration_ms ?? 0))),
      (v) => {
        const d = cur()?.duration_ms ?? 0;
        if (d > 0) {
          clock.reset(v * d);
          void api.seek(v * d).catch((e) => toast(errorMessage(e), 'error'));
        }
      },
    );
    this.volume = new Slider(T('Громкость'), (v) => this.applyVolume(v, false), (v) => this.applyVolume(v, true));
    this.muteBtn.addEventListener('click', () => {
      const v = (store.snapshot?.volume ?? 0) > 0 ? 0 : this.lastVolume || 0.8;
      this.volume.set(v);
      this.applyVolume(v, true);
    });
    // one window above the bar at a time
    this.eqBtn.addEventListener('click', () => {
      queue.setOpen(false);
      eq.toggle();
    });
    this.queueBtn.addEventListener('click', () => {
      eq.close();
      queue.toggle();
    });
    queue.init((open) => this.queueBtn.classList.toggle('is-on', open));
    this.miniBtn.addEventListener('click', () => void invoke('mini_open').catch((e) => toast(errorMessage(e), 'error')));
    this.sleepBtn.addEventListener('click', () => this.sleepMenu());
    void listen<SleepState>('player:sleep', (e) => this.renderSleep(e.payload));
    void api.sleepGet().then((s) => this.renderSleep(s), () => {});
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
      h('div', { class: 'pb-right' }, this.miniBtn, this.queueBtn, this.sleepBtn, this.eqBtn, this.muteBtn, h('div', { class: 'pb-volume' }, this.volume.el), this.fsBtn, this.lyricsBtn),
    );

    host.querySelector('.pb-now')!.addEventListener('contextmenu', (e) => {
      const t = cur();
      if (t) trackMenu(e as MouseEvent, t, () => void api.toggle());
    });

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
    this.lyricsBtn.title = open ? T('Скрыть текст') : T('Показать текст');
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
    this.title.textContent = t ? t.title : T('Ничего не играет');
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
    this.playBtn.title = s.loading ? T('Загрузка…') : s.playing ? T('Пауза') : T('Воспроизвести');
    this.playBtn.classList.toggle('is-loading', s.loading);
    this.shuffleBtn.classList.toggle('is-on', s.shuffle);
    setIcon(this.shuffleBtn, s.smart ? I.smartShuffle : I.shuffle);
    this.shuffleBtn.title = s.smart
      ? T('Умное перемешивание: между треками похожие рекомендации')
      : s.shuffle
        ? T('Перемешивание (ещё раз — умное)')
        : T('Перемешать');
    this.title.title = s.recommended ? T('Рекомендация умного перемешивания') : '';
    setIcon(this.repeatBtn, s.repeat === 'one' ? I.repeatOne : I.repeat);
    this.repeatBtn.classList.toggle('is-on', s.repeat !== 'off');
    this.repeatBtn.title = { off: T('Повтор выключен'), all: T('Повтор очереди'), one: T('Повтор трека') }[s.repeat];
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
    this.like.title = liked ? T('Убрать из лайков') : T('Лайкнуть');
    this.dislike.classList.toggle('is-on', disliked);
    this.dislike.title = disliked ? T('Снова рекомендовать') : T('Не рекомендовать');
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

  // ------------------------------------------------------------ sleep timer

  private sleepMenu(): void {
    const set = (minutes: number | null, end: boolean) =>
      void api.sleepSet(minutes, end).then((st) => {
        this.renderSleep(st);
        toast(end ? T('Пауза после этого трека') : minutes ? T('Пауза через {0} мин', minutes) : T('Таймер сна выключен'));
      }, (err) => toast(errorMessage(err), 'error'));
    const active = this.sleepBtn.classList.contains('is-on');
    const items: MenuItem[] = [15, 30, 45, 60].map((m) => ({ label: T('{0} мин', m), icon: I.sleep, action: () => set(m, false) }));
    items.push({ label: T('Своё время…'), icon: I.timecode, action: () => void askSleepTime().then((m) => m && set(m, false)) });
    items.push({ label: T('После этого трека'), icon: I.queueEnd, action: () => set(null, true) });
    if (active) items.push('separator', { label: T('Выключить'), icon: I.close, action: () => set(null, false) });
    // under the button, not at the cursor
    const r = this.sleepBtn.getBoundingClientRect();
    openMenu(new MouseEvent('contextmenu', { clientX: r.left, clientY: r.top - 4 }), items);
  }

  private renderSleep(s: SleepState): void {
    window.clearInterval(this.sleepTimer);
    const on = s.end_of_track || s.remaining_s !== null;
    this.sleepBtn.classList.toggle('is-on', on);
    const label = () => {
      if (s.end_of_track) return T('Таймер сна: пауза после этого трека');
      if (s.remaining_s === null) return T('Таймер сна');
      const min = Math.max(1, Math.ceil(s.remaining_s / 60));
      const at = new Date(Date.now() + s.remaining_s * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
      return T('Таймер сна: пауза через {0} мин (в {1})', min, at);
    };
    this.sleepBtn.title = label();
    if (s.remaining_s !== null) {
      // the tooltip counts down once a minute: no busy timers
      this.sleepTimer = window.setInterval(() => {
        if (s.remaining_s === null) return;
        s = { ...s, remaining_s: Math.max(0, s.remaining_s - 60) };
        this.sleepBtn.title = label();
      }, 60_000);
    }
  }
}

function nextRepeat(m: RepeatMode): RepeatMode {
  return m === 'off' ? 'all' : m === 'all' ? 'one' : 'off';
}
