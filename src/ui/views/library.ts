// «Лайки»: Треки / Плейлисты / Альбомы / Артисты.
import { api, errorMessage, fmtCount, type Playlist, type Track, type User } from '../api';
import { artistGrid, playlistGrid } from '../cards';
import { h, toast } from '../dom';
import { I } from '../icons';
import type { LibTab } from '../router';
import { SegTabs } from '../seg_tabs';
import { store } from '../store';
import { ROW_HEIGHT, trackRowRenderer } from '../track_row';
import { ArraySource, PagedSource, VirtualList } from '../virtual_list';
import { btn, emptyState, viewHead, type View } from './common';
import { T } from '../i18n';

export class LibraryView implements View {
  el: HTMLElement;
  private tabs: SegTabs<LibTab>;
  private body = h('div', { class: 'view-body' });
  private syncBtn: HTMLButtonElement;
  private list: VirtualList<Track>;
  private trackTotal = 0;
  private playlists: Playlist[] | null = null;
  private artists: User[] | null = null;
  private loadedTracks = false;

  constructor() {
    this.syncBtn = btn(T('Синхронизировать'), I.refresh);
    this.syncBtn.addEventListener('click', () => void this.sync());
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
    this.list = new VirtualList<Track>(new ArraySource(), trackRowRenderer(), ROW_HEIGHT);
    this.list.onRowActivate((i) => api.playLikes(i).catch((e) => toast(errorMessage(e), 'error')));
    this.el = h('section', { class: 'view' }, viewHead(T('Лайки'), this.syncBtn), h('div', { class: 'tabs-row' }, this.tabs.el), this.body);
    const refresh = () => this.list.refresh();
    store.on('player', refresh);
    store.on('likes', refresh);
    store.on('dislikes', refresh);
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
      else scroller.append(playlistGrid(items));
    }
    this.body.replaceChildren(scroller);
  }

  private emptyTracks(): HTMLElement {
    return emptyState(
      store.auth.has_credentials
        ? T('Кэш пуст. Нажмите «Синхронизировать», чтобы загрузить лайки.')
        : T('Войдите в SoundCloud в Настройках.'),
    );
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

  private async reloadTracks(): Promise<void> {
    try {
      this.trackTotal = await api.likesCount();
      this.loadedTracks = true;
      this.list.setSource(new PagedSource<Track>(this.trackTotal, (o, l) => api.likesPage(o, l)));
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
