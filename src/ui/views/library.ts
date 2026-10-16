// «Лайки»: Треки / Плейлисты / Альбомы / Артисты.
import { holdReorder } from '../hold_reorder';
import { api, errorMessage, fmtCount, type Playlist, type Track, type User } from '../api';
import { artistGrid, playlistGrid } from '../cards';
import { groupSeries } from '../playlist_series';
import { h, toast } from '../dom';
import { I } from '../icons';
import type { LibTab } from '../router';
import { SegTabs } from '../seg_tabs';
import { store } from '../store';
import { rowHeight, trackRowRenderer } from '../track_row';
import { ArraySource, PagedSource, VirtualList } from '../virtual_list';
import { btn, emptyState, signInPrompt, viewHead, type View } from './common';
import { importDialog } from '../import_dialog';
import { duplicatesDialog } from '../duplicates_dialog';
import { T } from '../i18n';

export class LibraryView implements View {
  el: HTMLElement;
  private tabs: SegTabs<LibTab>;
  private body = h('div', { class: 'view-body' });
  private syncBtn: HTMLButtonElement;
  private importBtn = btn(T('Импорт'), I.download);
  private dupesBtn = btn(T('Дубли'), I.copy);
  private list: VirtualList<Track>;
  private trackTotal = 0;
  private playlists: Playlist[] | null = null;
  private artists: User[] | null = null;
  private loadedTracks = false;

  constructor() {
    this.syncBtn = btn(T('Синхронизировать'), I.refresh);
    this.syncBtn.addEventListener('click', () => void this.sync());
    this.dupesBtn.title = T('Найти песни, лайкнутые несколько раз (разные загрузки одной песни)');
    this.dupesBtn.addEventListener('click', () => void duplicatesDialog());
    this.importBtn.title = T('Импорт плейлиста из Яндекс Музыки или Spotify');
    this.importBtn.addEventListener('click', () => importDialog(() => void this.loadPlaylists(true)));
    this.tabs = new SegTabs<LibTab>(
      [
        { key: 'tracks', label: T('Треки') },
        { key: 'playlists', label: T('Плейлисты') },
        { key: 'albums', label: T('Альбомы') },
        { key: 'artists', label: T('Артисты') },
      ],
      'tracks',
      () => this.renderTab(),
    );
    this.list = new VirtualList<Track>(new ArraySource(), trackRowRenderer(), rowHeight());
    this.list.onRowActivate((i) => api.playLikes(i).catch((e) => toast(errorMessage(e), 'error')));
    this.el = h('section', { class: 'view' }, viewHead(T('Лайки'), h('div', { class: 'row-actions' }, this.dupesBtn, this.importBtn, this.syncBtn)), h('div', { class: 'tabs-row' }, this.tabs.el), this.body);
    const refresh = () => this.list.refresh();
    store.on('player', refresh);
    // the playing like follows the skips: when it leaves the screen, the list moves to it
    let shownId: number | null = null;
    store.on('player', () => {
      const s = store.snapshot;
      const id = s?.track?.id ?? null;
      if (id === shownId) return;
      shownId = id;
      if (s && s.source === 'likes' && !s.shuffle && this.tabs.value === 'tracks') this.list.reveal(s.queue_pos);
    });
    // a like or an unlike shows in the list at once, no «Синхронизировать» needed
    let likesTimer = 0;
    store.on('likes', () => {
      refresh();
      window.clearTimeout(likesTimer);
      likesTimer = window.setTimeout(() => void this.reloadTracks(true), 400);
    });
    // hold a track and drag it: the likes in an order of your own (kept in the app; SoundCloud sorts by date)
    holdReorder(this.list.el, {
      scroller: this.list.el,
      onMove: (from, to) =>
        void api
          .likesMove(from, to)
          .then(() => this.reloadTracks(true))
          .catch((e) => toast(errorMessage(e), 'error')),
    });
    store.on('dislikes', refresh);
    window.addEventListener('sc:playlists', () => void this.loadPlaylists(false));
  }

  show(tab?: LibTab): void {
    if (tab) this.tabs.select(tab);
    void this.refreshCounts();
    if (!this.loadedTracks) void this.reloadTracks();
    if (this.playlists === null) void this.loadPlaylists(false);
    if (this.artists === null) void this.loadArtists(false);
    this.renderTab();
  }

  private renderTab(): void {
    const tab = this.tabs.value;
    this.syncBtn.hidden = false;
    // import makes playlists: shown where the playlists are
    this.importBtn.hidden = tab !== 'playlists';
    this.dupesBtn.hidden = tab !== 'tracks';
    if (tab === 'tracks') {
      this.body.replaceChildren(this.trackTotal || !this.loadedTracks ? this.list.el : this.emptyTracks());
      this.list.refresh();
      return;
    }
    const scroller = h('div', { class: 'view-scroll' });
    if (tab === 'artists') {
      if (this.artists === null) scroller.append(emptyState(T('Загрузка…')));
      else if (!this.artists.length) scroller.append(emptyState(T('Вы пока ни на кого не подписаны.')));
      else scroller.append(artistGrid(this.artists));
    } else {
      const items = (this.playlists ?? []).filter((p) => (tab === 'albums' ? p.is_album : !p.is_album));
      if (this.playlists === null) scroller.append(emptyState(T('Загрузка…')));
      else if (!items.length) scroller.append(emptyState(tab === 'albums' ? T('Нет лайкнутых альбомов.') : T('Нет лайкнутых плейлистов.')));
      else scroller.append(playlistGrid(groupSeries(items)));
    }
    this.body.replaceChildren(scroller);
  }

  private emptyTracks(): HTMLElement {
    return store.auth.has_credentials
      ? emptyState(T('Кэш пуст. Нажмите «Синхронизировать», чтобы загрузить лайки.'))
      : signInPrompt(T('Войдите в SoundCloud, чтобы увидеть свои лайки.'));
  }

  private async refreshCounts(): Promise<void> {
    try {
      const c = await api.libraryCounts();
      this.tabs.setCount('tracks', fmtCount(c.tracks));
      this.tabs.setCount('playlists', c.playlists ? fmtCount(c.playlists) : '');
      this.tabs.setCount('albums', c.albums ? fmtCount(c.albums) : '');
      this.tabs.setCount('artists', c.artists ? fmtCount(c.artists) : '');
    } catch {
      /* counts are cosmetic */
    }
  }

  private async reloadTracks(keepScroll = false): Promise<void> {
    try {
      this.trackTotal = await api.likesCount();
      this.loadedTracks = true;
      const source = new PagedSource<Track>(this.trackTotal, (o, l) => api.likesPage(o, l));
      if (keepScroll) this.list.replaceSource(source);
      else this.list.setSource(source);
      if (this.tabs.value === 'tracks') this.renderTab();
    } catch (e) {
      toast(errorMessage(e), 'error');
    }
  }

  private async loadPlaylists(force: boolean): Promise<void> {
    if (!store.auth.has_credentials) return;
    try {
      this.playlists = await api.libraryPlaylists(force);
      void this.refreshCounts();
      if (this.tabs.value === 'playlists' || this.tabs.value === 'albums') this.renderTab();
    } catch (e) {
      this.playlists = this.playlists ?? [];
      toast(errorMessage(e), 'error');
    }
  }

  private async loadArtists(force: boolean): Promise<void> {
    if (!store.auth.has_credentials) return;
    try {
      this.artists = await api.libraryArtists(force);
      store.setFollowing(this.artists);
      void this.refreshCounts();
      if (this.tabs.value === 'artists') this.renderTab();
    } catch (e) {
      this.artists = this.artists ?? [];
      toast(errorMessage(e), 'error');
    }
  }

  async sync(): Promise<void> {
    this.syncBtn.disabled = true;
    try {
      const n = await api.likesSync();
      toast(T('Загружено лайков: {0}', n));
      await Promise.all([store.reloadSets(), this.reloadTracks(), this.loadPlaylists(true), this.loadArtists(true)]);
    } catch (e) {
      toast(errorMessage(e), 'error');
    } finally {
      this.syncBtn.disabled = false;
    }
  }

  /** Likes changed on SoundCloud (background sync) — re-read the cache. */
  reloadFromCache(): void {
    void this.reloadTracks();
    void this.refreshCounts();
  }

  setProgress(n: number): void {
    this.tabs.setCount('tracks', T('загрузка… {0}', n));
  }
}
