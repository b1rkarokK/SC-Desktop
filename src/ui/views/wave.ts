// «Моя волна»: current track + why it's here, mood, up next, bans.
import { api, coverUrl, errorMessage, type Mood, type WaveInfo } from '../api';
import { h, toast } from '../dom';
import { icon, iconButton, I } from '../icons';
import { openTrack, openTrackArtist } from '../router';
import { store } from '../store';
import { btn, emptyState, sectionTitle, staticTrackList, viewHead, type View } from './common';

const MOODS: [Mood, string][] = [
  ['normal', 'Как обычно'],
  ['fresh', 'Больше нового'],
  ['familiar', 'Только знакомое'],
  ['calm', 'Спокойнее'],
  ['energetic', 'Энергичнее'],
];

export class WaveView implements View {
  el: HTMLElement;
  private cover = h('img', { class: 'wave-cover', alt: '' });
  private coverBox = h('div', { class: 'wave-cover-box' }, this.cover, icon(I.wave, 28));
  private label = h('div', { class: 'muted small' });
  private title = h('button', { type: 'button', class: 'wave-title link', dir: 'auto' });
  private sub = h('div', { class: 'muted small', dir: 'auto' });
  private startBtn = btn('Запустить волну', I.wave, true);
  private dislikeBtn = btn('Не нравится', I.dislike);
  private banBtn = btn('Не ставить артиста', I.banArtist);
  private moods = h('div', { class: 'chips' });
  private next = h('div');
  private bans = h('div', { class: 'chips' });
  private bansNote = h('div', { class: 'disliked-list' });
  private info: WaveInfo | null = null;
  private lastTrack: number | null = null;
  private noLikedBtn = h('button', { type: 'button', class: 'toggle', role: 'switch' }, h('span', { class: 'toggle-knob' }));
  private noLikedRow = h(
    'div',
    { class: 'set-row wave-option' },
    h(
      'div',
      { class: 'set-label' },
      h('div', { text: 'Без лайкнутых' }),
      h('div', { class: 'muted small', text: 'не предлагать то, что уже в лайках, и их копии с другим названием. Другие версии (speed up, slowed, hardtekk) остаются' }),
    ),
    this.noLikedBtn,
  );

  constructor() {
    this.startBtn.addEventListener('click', () => void this.start());
    this.dislikeBtn.addEventListener('click', () => {
      const t = store.snapshot?.track;
      if (t) void store.setDisliked(t, true);
    });
    this.banBtn.addEventListener('click', () => void this.ban());
    this.title.addEventListener('click', () => {
      const t = store.snapshot?.track;
      if (t) openTrack(t.id);
    });
    this.noLikedBtn.addEventListener('click', async () => {
      const on = !(this.info?.no_liked ?? true);
      this.renderNoLiked(on);
      try {
        await api.waveSetNoLiked(on);
        await this.refresh();
      } catch (e) {
        toast(errorMessage(e), 'error');
      }
    });
    const reset = h('button', { type: 'button', class: 'btn btn-quiet', text: 'Сбросить всё' });
    reset.addEventListener('click', () => void this.resetBans());

    this.el = h(
      'section',
      { class: 'view' },
      viewHead('Моя волна'),
      h(
        'div',
        { class: 'view-scroll pad' },
        h(
          'div',
          { class: 'wave-now panel' },
          this.coverBox,
          h('div', { class: 'wave-meta' }, this.label, this.title, this.sub),
          h('div', { class: 'wave-actions' }, this.startBtn, this.dislikeBtn, this.banBtn),
        ),
        sectionTitle('Настроение волны'),
        this.moods,
        this.noLikedRow,
        sectionTitle('Далее в волне'),
        this.next,
        sectionTitle('Исключённые артисты', reset),
        this.bans,
        sectionTitle('Не рекомендовать (треки)'),
        this.bansNote,
      ),
    );
    store.on('player', () => this.onPlayer());
  }

  show(): void {
    void this.refresh();
  }

  private onPlayer(): void {
    const id = store.currentId();
    this.renderNow();
    if (id !== this.lastTrack && this.el.isConnected) {
      this.lastTrack = id;
      void this.refresh();
    }
  }

  private async refresh(): Promise<void> {
    try {
      this.info = await api.waveInfo(store.currentId());
      this.renderNow();
      this.renderMoods();
      this.renderNoLiked(this.info.no_liked);
      this.renderBans();
      await this.renderNext();
    } catch (e) {
      toast(errorMessage(e), 'error');
    }
  }

  private renderNow(): void {
    const s = store.snapshot;
    const t = s?.track ?? null;
    const inWave = s?.source === 'wave';
    this.label.textContent = inWave ? 'Сейчас в волне' : t ? 'Сейчас играет (не из волны)' : 'Волна не запущена';
    this.title.textContent = t?.title ?? 'Нажмите «Запустить волну»';
    this.title.disabled = !t;
    this.sub.replaceChildren();
    if (t) {
      const artist = h('button', { type: 'button', class: 'link', text: t.artist });
      artist.addEventListener('click', () => void openTrackArtist(t));
      this.sub.append(artist);
      if (inWave && this.info?.reason) this.sub.append(` · ${this.info.reason}`);
    }
    // the cover cell always stays (empty placeholder without a track) so the
    // text keeps its own column and isn't squeezed into the 96px cover slot
    const src = coverUrl(t?.artwork_url, 't300x300');
    if (src) {
      if (this.cover.getAttribute('src') !== src) this.cover.src = src;
    } else this.cover.removeAttribute('src');
    this.coverBox.classList.toggle('is-empty', !src);
    // always available: after «Волна по треку» you can go back to the normal wave
    this.startBtn.replaceChildren(icon(I.wave), h('span', { text: inWave ? 'Обычная волна' : 'Запустить волну' }));
    this.dislikeBtn.disabled = this.banBtn.disabled = !t;
  }

  private renderMoods(): void {
    const cur = this.info?.mood ?? 'normal';
    this.moods.replaceChildren(
      ...MOODS.map(([m, label]) => {
        const b = h('button', { type: 'button', class: `chip-btn${m === cur ? ' is-active' : ''}`, text: label });
        b.addEventListener('click', async () => {
          try {
            await api.waveSetMood(m);
            await this.refresh();
          } catch (e) {
            toast(errorMessage(e), 'error');
          }
        });
        return b;
      }),
    );
  }

  private renderNoLiked(on: boolean): void {
    if (this.info) this.info.no_liked = on;
    this.noLikedBtn.classList.toggle('is-on', on);
    this.noLikedBtn.setAttribute('aria-checked', String(on));
  }

  private async renderNext(): Promise<void> {
    if (store.snapshot?.source !== 'wave') {
      this.next.replaceChildren(emptyState('Запустите волну, и здесь появятся следующие треки.'));
      return;
    }
    const tracks = await api.upcoming(20);
    this.next.replaceChildren(tracks.length ? staticTrackList(tracks) : emptyState('Подбираем следующие треки…'));
  }

  private renderBans(): void {
    const list = this.info?.disliked_artists ?? [];
    this.bans.replaceChildren(
      ...list.map((a) => {
        const rm = iconButton(I.close, `Вернуть ${a.name}`);
        rm.addEventListener('click', async () => {
          await api.waveUndislikeArtist(a.user_id).catch((e) => toast(errorMessage(e), 'error'));
          void this.refresh();
        });
        return h('div', { class: 'chip' }, h('span', { dir: 'auto', text: a.name }), rm);
      }),
    );
    if (!list.length) this.bans.append(h('span', { class: 'muted small', text: 'Артистов в исключениях нет.' }));
    void this.renderDislikedTracks();
  }

  /** Each disliked track with its own «Вернуть». */
  private async renderDislikedTracks(): Promise<void> {
    const tracks = await api.dislikedTracks().catch(() => []);
    if (!tracks.length) {
      this.bansNote.replaceChildren(h('span', { class: 'muted small', text: 'Дизлайкнутых треков нет.' }));
      return;
    }
    this.bansNote.replaceChildren(
      ...tracks.map((t) => {
        const back = h('button', { type: 'button', class: 'btn btn-quiet', text: 'Вернуть' });
        back.addEventListener('click', async () => {
          await store.setDisliked(t, false);
          void this.renderDislikedTracks();
        });
        const img = h('img', { class: 'row-cover', alt: '' });
        const src = coverUrl(t.artwork_url, 't67x67');
        if (src) img.src = src;
        return h(
          'div',
          { class: 'disliked-row' },
          img,
          h('div', { class: 'row-main' }, h('div', { class: 'row-title', dir: 'auto', text: t.title }), h('div', { class: 'row-artist', dir: 'auto', text: t.artist })),
          back,
        );
      }),
    );
  }

  private async start(): Promise<void> {
    this.startBtn.disabled = true;
    try {
      const n = await api.waveStart();
      toast(`Волна: ${n} треков, дальше подгружается сама`);
      await this.refresh();
    } catch (e) {
      toast(errorMessage(e), 'error');
    } finally {
      this.startBtn.disabled = false;
    }
  }

  private async ban(): Promise<void> {
    const t = store.snapshot?.track;
    if (!t) return;
    try {
      await api.waveDislikeArtist(t.user_id, t.artist);
      toast(`${t.artist} исключён из волны`);
      await this.refresh();
    } catch (e) {
      toast(errorMessage(e), 'error');
    }
  }

  private async resetBans(): Promise<void> {
    try {
      await api.waveClearDislikes();
      await store.reloadSets();
      await this.refresh();
    } catch (e) {
      toast(errorMessage(e), 'error');
    }
  }
}
