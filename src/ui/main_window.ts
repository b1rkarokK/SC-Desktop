// Left navigation — a slim icon rail that smoothly slides out with labels on
// hover (over the content, layout doesn't jump) — plus the center outlet
// driven by the router.
import { coverUrl } from './api';
import { h } from './dom';
import { icon, I } from './icons';
import { router, type Route, type Section } from './router';
import { store } from './store';
import type { View } from './views/common';
import { HistoryView } from './views/history';
import { ProfileView } from './views/profile';
import { LibraryView } from './views/library';
import { ArtistPageView, PlaylistPageView, TrackPageView } from './views/pages';
import { SearchView } from './views/search';
import { SettingsView } from './views/settings';
import { WaveView } from './views/wave';

const NAV: [Section, string, Parameters<typeof icon>[0]][] = [
  ['likes', 'Лайки', I.heart],
  ['wave', 'Моя волна', I.wave],
  ['history', 'История', I.history],
  ['search', 'Поиск', I.search],
  ['settings', 'Настройки', I.settings],
];

export class MainWindow {
  readonly library = new LibraryView();
  private wave = new WaveView();
  private history = new HistoryView();
  private search = new SearchView();
  private settings = new SettingsView();
  private profile = new ProfileView();
  private tabs = new Map<Section, HTMLButtonElement>();
  private user = h('button', { type: 'button', class: 'nav-user', title: 'Профиль: скачанные и очередь лайков' });

  constructor(nav: HTMLElement, private center: HTMLElement) {
    const list = h('div', { class: 'nav-list', role: 'tablist' });
    for (const [id, label, ic] of NAV) {
      const b = h('button', { type: 'button', class: 'nav-item', role: 'tab' }, icon(ic), h('span', { class: 'nav-label', text: label }));
      b.addEventListener('click', () => {
        router.go({ name: id } as Route);
        b.blur(); // otherwise focus keeps the rail expanded
      });
      this.tabs.set(id, b);
      list.append(b);
    }
    this.user.addEventListener('click', () => {
      router.go({ name: 'profile' });
      this.user.blur();
    });
    nav.append(h('div', { class: 'nav-panel' }, list, this.user));

    store.on('auth', () => this.renderUser());
    this.renderUser();
    router.on((r) => this.render(r));
  }

  start(route: Route): void {
    router.go(route);
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
  }

  private render(r: Route): void {
    const section = router.section;
    for (const [id, tab] of this.tabs) {
      tab.classList.toggle('is-active', id === section);
      tab.setAttribute('aria-selected', String(id === section));
    }
    this.user.classList.toggle('is-active', section === 'profile');
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
      case 'history':
        view = this.history;
        break;
      case 'search':
        view = this.search;
        break;
      case 'settings':
        view = this.settings;
        break;
      case 'profile':
        this.center.replaceChildren(this.profile.el);
        this.profile.show(r.tab);
        return;
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
