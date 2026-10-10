// «История»: recently played tracks grouped by day (local, from the plays table).
import { api, errorMessage, plural, type HistoryItem, type Playlist } from '../api';
import { playlistGrid } from '../cards';
import { h, toast } from '../dom';
import { I } from '../icons';
import { store } from '../store';
import { btn, emptyState, sectionTitle, staticTrackList, viewHead, type View } from './common';
import { T } from '../i18n';

const PAGE = 200;

function dayLabel(d: Date): string {
  const today = new Date();
  const start = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((start(today) - start(d)) / 86_400_000);
  if (diff === 0) return T('Сегодня');
  if (diff === 1) return T('Вчера');
  return d.toLocaleDateString('ru-RU', { day: 'numeric', month: 'long', year: d.getFullYear() === today.getFullYear() ? undefined : 'numeric' });
}

export class HistoryView implements View {
  el: HTMLElement;
  private body = h('div', { class: 'view-scroll pad' });
  private items: HistoryItem[] = [];
  private playlists: { playlist: Playlist; played_at: number }[] = [];
  private more: HTMLButtonElement;
  private lastTrack: number | null = null;

  constructor() {
    const clear = btn(T('Очистить'), null);
    clear.classList.add('btn-quiet');
    clear.addEventListener('click', async () => {
      try {
        await api.historyClear();
        this.items = [];
        this.playlists = [];
        this.render(false);
      } catch (e) {
        toast(errorMessage(e), 'error');
      }
    });
    this.more = btn(T('Показать ещё'), I.chevronDown);
    this.more.addEventListener('click', () => void this.load(true));
    this.el = h('section', { class: 'view' }, viewHead(T('История'), clear), this.body);
    // a new track started → refresh if the view is on screen
    store.on('player', () => {
      const id = store.currentId();
      if (id !== this.lastTrack) {
        this.lastTrack = id;
        if (this.el.isConnected && !store.snapshot?.loading) window.setTimeout(() => void this.load(false), 1500);
      }
    });
  }

  show(): void {
    void this.load(false);
  }

  private async load(append: boolean): Promise<void> {
    try {
      const [page, pls] = await Promise.all([
        api.history(append ? this.items.length : 0, PAGE),
        append ? Promise.resolve(this.playlists) : api.historyPlaylists(300),
      ]);
      this.items = append ? this.items.concat(page) : page;
      this.playlists = pls;
      this.render(page.length === PAGE);
    } catch (e) {
      toast(errorMessage(e), 'error');
    }
  }

  private render(hasMore: boolean): void {
    if (!this.items.length && !this.playlists.length) {
      this.body.replaceChildren(emptyState(T('Здесь появятся треки, альбомы и плейлисты, которые вы слушали.')));
      return;
    }
    // day → playlists + tracks, newest day first
    const days = new Map<string, { at: number; tracks: HistoryItem[]; playlists: Playlist[] }>();
    const day = (ts: number) => {
      const label = dayLabel(new Date(ts * 1000));
      let d = days.get(label);
      if (!d) days.set(label, (d = { at: ts, tracks: [], playlists: [] }));
      d.at = Math.max(d.at, ts);
      return d;
    };
    const oldestTrack = this.items.length ? this.items[this.items.length - 1]!.played_at : 0;
    for (const p of this.playlists) if (!hasMore || p.played_at >= oldestTrack) day(p.played_at).playlists.push(p.playlist);
    for (const it of this.items) day(it.played_at).tracks.push(it);
    const frag = document.createDocumentFragment();
    for (const [label, d] of [...days].sort((a, b) => b[1].at - a[1].at)) {
      const parts = [
        d.playlists.length ? T('{0} {1}', d.playlists.length, plural(d.playlists.length, 'плейлист', 'плейлиста', 'плейлистов')) : '',
        d.tracks.length ? T('{0} {1}', d.tracks.length, plural(d.tracks.length, 'трек', 'трека', 'треков')) : '',
      ].filter(Boolean);
      frag.append(sectionTitle(`${label} · ${parts.join(', ')}`));
      if (d.playlists.length) frag.append(h('div', { class: 'history-pls' }, playlistGrid(d.playlists)));
      if (d.tracks.length) frag.append(staticTrackList(d.tracks.map((x) => x.track)));
    }
    if (hasMore) frag.append(h('div', { class: 'row-actions center-row' }, this.more));
    const top = this.body.scrollTop;
    this.body.replaceChildren(frag);
    this.body.scrollTop = top;
  }
}
