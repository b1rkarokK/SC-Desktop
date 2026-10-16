// «История»: recently played tracks grouped by day (local, from the plays table).
import { api, errorMessage, plural, type HistoryItem, type Playlist } from '../api';
import { playlistGrid } from '../cards';
import { h, toast } from '../dom';
import { I } from '../icons';
import { store } from '../store';
import { btn, emptyState, sectionTitle, staticTrackList, viewHead, type View } from './common';
import { T } from '../i18n';
import { confirmDialog } from '../confirm';

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
  private query = '';
  /** all of the history is loaded (a search looks through all of it, not only the shown part) */
  private everything = false;
  private lastHasMore = false;

  constructor() {
    const clear = btn(T('Очистить'), null);
    clear.classList.add('btn-quiet');
    clear.addEventListener('click', async () => {
      const ok = await confirmDialog(
        T('Очистить историю?'),
        T('Пропадут все прослушанные треки, альбомы и плейлисты, а с ними и «Итоги». Вернуть их будет нельзя.'),
        T('Очистить'),
      );
      if (!ok) return;
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
    // «что я слушал вчера вечером»: a search by title and artist over all the history
    const search = h('input', { type: 'search', class: 'input hist-search', placeholder: T('Найти в истории…'), 'aria-label': T('Найти в истории') });
    let typing = 0;
    search.addEventListener('input', () => {
      window.clearTimeout(typing);
      typing = window.setTimeout(async () => {
        this.query = search.value.trim().toLowerCase();
        if (this.query && !this.everything) {
          try {
            this.items = await api.history(0, 5000);
            this.everything = true;
            this.lastHasMore = false;
          } catch (e) {
            toast(errorMessage(e), 'error');
          }
        }
        this.render(this.lastHasMore && !this.query);
      }, 200);
    });
    this.el = h('section', { class: 'view' }, viewHead(T('История'), h('div', { class: 'row-actions' }, search, clear)), this.body);
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
      this.everything = false;
      this.lastHasMore = page.length === PAGE;
      this.render(this.lastHasMore && !this.query);
    } catch (e) {
      toast(errorMessage(e), 'error');
    }
  }

  private render(hasMore: boolean): void {
    const q = this.query;
    const shown = q ? this.items.filter((it) => `${it.track.title} ${it.track.artist}`.toLowerCase().includes(q)) : this.items;
    const shownPlaylists = q ? [] : this.playlists;
    if (q && !shown.length) {
      this.body.replaceChildren(emptyState(T('В истории такого нет.')));
      return;
    }
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
    for (const p of shownPlaylists) if (!hasMore || p.played_at >= oldestTrack) day(p.played_at).playlists.push(p.playlist);
    for (const it of shown) day(it.played_at).tracks.push(it);
    const frag = document.createDocumentFragment();
    for (const [label, d] of [...days].sort((a, b) => b[1].at - a[1].at)) {
      const parts = [
        d.playlists.length ? T('{0} {1}', d.playlists.length, plural(d.playlists.length, 'плейлист', 'плейлиста', 'плейлистов')) : '',
        d.tracks.length ? T('{0} {1}', d.tracks.length, plural(d.tracks.length, 'трек', 'трека', 'треков')) : '',
      ].filter(Boolean);
      frag.append(sectionTitle(`${label} · ${parts.join(', ')}`));
      if (d.playlists.length) frag.append(h('div', { class: 'history-pls' }, playlistGrid(d.playlists)));
      if (d.tracks.length) {
        // a day by the time of day: night, morning, day, evening (the newest part first)
        const part = (ts: number) => {
          const hr = new Date(ts * 1000).getHours();
          return hr < 6 ? 0 : hr < 12 ? 1 : hr < 18 ? 2 : 3;
        };
        const names = [T('Ночью'), T('Утром'), T('Днём'), T('Вечером')];
        const used = [...new Set(d.tracks.map((x) => part(x.played_at)))].sort((a, b) => b - a);
        for (const p of used) {
          const tracks = d.tracks.filter((x) => part(x.played_at) === p).map((x) => x.track);
          if (used.length > 1) frag.append(h('div', { class: 'muted small hist-part', text: names[p]! }));
          frag.append(staticTrackList(tracks));
        }
      }
    }
    if (hasMore) frag.append(h('div', { class: 'row-actions center-row' }, this.more));
    const top = this.body.scrollTop;
    this.body.replaceChildren(frag);
    this.body.scrollTop = top;
  }
}
