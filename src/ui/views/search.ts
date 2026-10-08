// «Поиск» по всему SoundCloud: Всё / Треки / Артисты / Плейлисты.
import { api, errorMessage, type Track } from '../api';
import { artistGrid, playlistGrid, userChips } from '../cards';
import { h, toast } from '../dom';
import { iconButton, icon, I } from '../icons';
import { SegTabs } from '../seg_tabs';
import { store } from '../store';
import { ROW_HEIGHT, trackRowRenderer } from '../track_row';
import { ArraySource, VirtualList } from '../virtual_list';
import { emptyState, sectionTitle, staticTrackList, type View } from './common';

type Tab = 'all' | 'tracks' | 'users' | 'playlists';

export class SearchView implements View {
  el: HTMLElement;
  private input: HTMLInputElement;
  private tabs: SegTabs<Tab>;
  private body = h('div', { class: 'view-body' });
  private query = '';
  private seq = 0;
  // tracks tab (virtualized, paged)
  private source = new ArraySource<Track>();
  private list: VirtualList<Track>;
  private nextOffset: number | null = null;
  private busy = false;

  constructor() {
    this.input = h('input', { type: 'search', class: 'input search-input', placeholder: 'Треки, артисты, плейлисты…', 'aria-label': 'Поиск' });
    const clear = iconButton(I.close, 'Очистить');
    clear.addEventListener('click', () => {
      this.input.value = '';
      this.input.focus();
      void this.run();
    });
    let timer = 0;
    this.input.addEventListener('input', () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => void this.run(), 450);
    });
    this.input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        window.clearTimeout(timer);
        void this.run();
      }
    });
    this.tabs = new SegTabs<Tab>(
      [
        { key: 'all', label: 'Всё' },
        { key: 'tracks', label: 'Треки' },
        { key: 'users', label: 'Артисты' },
        { key: 'playlists', label: 'Плейлисты' },
      ],
      'all',
      () => void this.run(true),
    );
    this.list = new VirtualList<Track>(this.source, trackRowRenderer(), ROW_HEIGHT);
    this.list.onRowActivate((i) => api.playTracks(this.source.items, i).catch((e) => toast(errorMessage(e), 'error')));
    const refresh = () => this.list.refresh();
    store.on('player', refresh);
    store.on('likes', refresh);
    store.on('dislikes', refresh);

    this.el = h(
      'section',
      { class: 'view' },
      h('header', { class: 'view-head' }, h('div', { class: 'search-box' }, icon(I.search), this.input, clear)),
      h('div', { class: 'tabs-row' }, this.tabs.el),
      this.body,
    );
    this.body.append(emptyState('Введите запрос: ищем по всему SoundCloud.'));
  }

  show(): void {
    this.input.focus();
  }

  private async run(force = false): Promise<void> {
    const q = this.input.value.trim();
    if (q === this.query && !force) return;
    this.query = q;
    const seq = ++this.seq;
    if (!q) {
      this.body.replaceChildren(emptyState('Введите запрос: ищем по всему SoundCloud.'));
      return;
    }
    this.body.replaceChildren(emptyState('Поиск…'));
    try {
      const tab = this.tabs.value;
      if (tab === 'all') {
        const r = await api.searchAll(q);
        if (seq !== this.seq) return;
        const s = h('div', { class: 'view-scroll pad' });
        if (r.users.length) s.append(sectionTitle('Артисты'), userChips(r.users.slice(0, 3)));
        if (r.tracks.length) {
          const all = h('button', { type: 'button', class: 'btn btn-quiet', text: 'Показать все' });
          all.addEventListener('click', () => this.tabs.select('tracks', true));
          s.append(sectionTitle('Треки', all), staticTrackList(r.tracks.slice(0, 8)));
        }
        if (r.playlists.length) s.append(sectionTitle('Плейлисты'), playlistGrid(r.playlists.slice(0, 4)));
        if (!r.users.length && !r.tracks.length && !r.playlists.length) s.append(emptyState('Ничего не найдено.'));
        this.body.replaceChildren(s);
      } else if (tab === 'tracks') {
        const page = await api.searchTracks(q, 0);
        if (seq !== this.seq) return;
        this.nextOffset = page.next_offset;
        this.source = new ArraySource<Track>(page.tracks, () => this.more(seq));
        this.list.setSource(this.source);
        this.body.replaceChildren(page.tracks.length ? this.list.el : emptyState('Ничего не найдено.'));
      } else if (tab === 'users') {
        const users = await api.searchUsers(q);
        if (seq !== this.seq) return;
        this.body.replaceChildren(h('div', { class: 'view-scroll' }, users.length ? artistGrid(users) : emptyState('Ничего не найдено.')));
      } else {
        const pls = await api.searchPlaylists(q);
        if (seq !== this.seq) return;
        this.body.replaceChildren(h('div', { class: 'view-scroll' }, pls.length ? playlistGrid(pls) : emptyState('Ничего не найдено.')));
      }
    } catch (e) {
      if (seq === this.seq) this.body.replaceChildren(emptyState(errorMessage(e)));
    }
  }

  private async more(seq: number): Promise<boolean> {
    if (this.busy || this.nextOffset === null || seq !== this.seq) return false;
    this.busy = true;
    try {
      const page = await api.searchTracks(this.query, this.nextOffset);
      if (seq !== this.seq) return false;
      this.nextOffset = page.next_offset;
      this.source.items.push(...page.tracks);
      this.list.refresh();
      return page.tracks.length > 0;
    } catch (e) {
      toast(errorMessage(e), 'error');
      this.nextOffset = null;
      return false;
    } finally {
      this.busy = false;
    }
  }
}
