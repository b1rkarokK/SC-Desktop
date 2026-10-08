// «Моя волна»: current track + why it's here, mood, up next, bans.
import { api, coverUrl, errorMessage, type Mood, type WaveInfo } from '../api';
import { h, toast } from '../dom';
import { iconButton, I } from '../icons';
import { openArtist, openTrack } from '../router';
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
  private label = h('div', { class: 'muted small' });
  private title = h('button', { type: 'button', class: 'wave-title link', dir: 'auto' });
  private sub = h('div', { class: 'muted small', dir: 'auto' });
  private startBtn = btn('Запустить волну', I.wave, true);
  private dislikeBtn = btn('Не нравится', I.dislike);
  private banBtn = btn('Не ставить артиста', I.banArtist);
  private moods = h('div', { class: 'chips' });
  private next = h('div');
  private bans = h('div', { class: 'chips' });
  private bansNote = h('div', { class: 'muted small' });
  private info: WaveInfo | null = null;
  private lastTrack: number | null = null;

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
          this.cover,
          h('div', { class: 'wave-meta' }, this.label, this.title, this.sub),
          h('div', { class: 'wave-actions' }, this.startBtn, this.dislikeBtn, this.banBtn),
        ),
        sectionTitle('Настроение волны'),
        this.moods,
        sectionTitle('Далее в волне'),
        this.next,
        sectionTitle('Исключены', reset),
        this.bans,
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
      artist.addEventListener('click', () => openArtist(t.user_id));
      this.sub.append(artist);
      if (inWave && this.info?.reason) this.sub.append(` · ${this.info.reason}`);
    }
    const src = coverUrl(t?.artwork_url, 't300x300');
    if (src) {
      if (this.cover.getAttribute('src') !== src) this.cover.src = src;
      this.cover.hidden = false;
    } else this.cover.hidden = true;
    this.startBtn.hidden = inWave;
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

  private async renderNext(): Promise<void> {
    if (store.snapshot?.source !== 'wave') {
      this.next.replaceChildren(emptyState('Запустите волну — здесь появятся следующие треки.'));
      return;
    }
    const tracks = await api.upcoming(8);
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
    const n = this.info?.disliked_tracks ?? 0;
    this.bansNote.textContent = `Дизлайкнутых треков: ${n} — они никогда не попадут в волну и рекомендации.`;
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
