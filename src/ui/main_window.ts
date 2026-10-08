// Left navigation (collapsible to an icon rail) + center outlet driven by the router.
import { coverUrl } from './api';
import { h } from './dom';
import { icon, I } from './icons';
import { onPrefs, prefs } from './prefs';
import { router, type Route, type Section } from './router';
import { store } from './store';
import type { View } from './views/common';
import { LibraryView } from './views/library';
import { ArtistPageView, PlaylistPageView, TrackPageView } from './views/pages';
import { SearchView } from './views/search';
import { SettingsView } from './views/settings';
import { WaveView } from './views/wave';

const NAV: [Section, string, Parameters<typeof icon>[0]][] = [
  ['likes', 'Лайки', I.heart],
  ['wave', 'Моя волна', I.wave],
  ['search', 'Поиск', I.search],
  ['settings', 'Настройки', I.settings],
];

/** Below this width the nav collapses automatically. */
const AUTO_COLLAPSE_PX = 1000;

export class MainWindow {
  readonly library = new LibraryView();
  private wave = new WaveView();
  private search = new SearchView();
  private settings = new SettingsView();
  private tabs = new Map<Section, HTMLButtonElement>();
  private user = h('div', { class: 'nav-user' });

  constructor(nav: HTMLElement, private center: HTMLElement) {
    const list = h('div', { class: 'nav-list', role: 'tablist' });
    for (const [id, label, ic] of NAV) {
      const b = h('button', { type: 'button', class: 'nav-item', role: 'tab', title: label }, icon(ic), h('span', { class: 'nav-label', text: label }));
      b.addEventListener('click', () => router.go({ name: id } as Route));
      this.tabs.set(id, b);
      list.append(b);
    }
    nav.append(list, this.user);

    store.on('auth', () => this.renderUser());
    this.renderUser();
    onPrefs(() => this.applyCollapsed());
    window.addEventListener('resize', () => this.applyCollapsed());
    this.applyCollapsed();
    router.on((r) => this.render(r));
  }

  start(route: Route): void {
    router.go(route);
  }

  private applyCollapsed(): void {
    const collapsed = prefs().navCollapsed || window.innerWidth < AUTO_COLLAPSE_PX;
    document.getElementById('app')?.classList.toggle('nav-collapsed', collapsed);
  }

  private renderUser(): void {
    const a = store.auth;
    const img = h('img', { class: 'nav-avatar', alt: '' });
    const src = coverUrl(a.avatar_url, 't67x67');
    if (src) img.src = src;
    this.user.replaceChildren(
      a.has_credentials ? img : icon(I.login),
      h('span', { class: 'nav-label', text: a.username ?? (a.has_credentials ? '' : 'Не выполнен вход') }),
    );
    this.user.title = a.username ?? '';
  }

  private render(r: Route): void {
    const section = router.section;
    for (const [id, tab] of this.tabs) {
      tab.classList.toggle('is-active', id === section);
      tab.setAttribute('aria-selected', String(id === section));
    }
    let view: View;
    switch (r.name) {
      case 'likes':
        view = this.library;
        this.center.replaceChildren(view.el);
        this.library.show(r.tab);
        return;
      case 'wave':
        view = this.wave;
        break;
      case 'search':
        view = this.search;
        break;
      case 'settings':
        view = this.settings;
        break;
      case 'track':
        view = new TrackPageView(r.id);
        break;
      case 'artist':
        view = new ArtistPageView(r.id);
        break;
      case 'playlist':
        view = new PlaylistPageView(r.id);
        break;
    }
    this.center.replaceChildren(view.el);
    view.show?.();
  }
}
