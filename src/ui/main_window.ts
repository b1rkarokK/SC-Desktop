// Left navigation — a slim icon rail that smoothly slides out with labels on
// hover (over the content, layout doesn't jump) — plus the center outlet
// driven by the router.
import { listen } from '@tauri-apps/api/event';
import { api, coverUrl } from './api';
import { h } from './dom';
import { icon, I } from './icons';
import { router, type Route, type Section } from './router';
import { store } from './store';
import type { View } from './views/common';
import { HistoryView } from './views/history';
import { StatsView } from './views/stats';
import { HomeView } from './views/home';
import { ProfileView } from './views/profile';
import { LibraryView } from './views/library';
import { ArtistPageView, CategoryPageView, ChartPageView, MixPageView, PlaylistPageView, TrackPageView } from './views/pages';
import { SearchView } from './views/search';
import { SettingsView } from './views/settings';
import { WaveView } from './views/wave';
import { T } from './i18n';

const NAV: [Section, string, Parameters<typeof icon>[0]][] = [
  ['home', T('Главная'), I.home],
  ['likes', T('Лайки'), I.heart],
  ['wave', T('Моя волна'), I.wave],
  ['history', T('История'), I.history],
  ['stats', T('Итоги'), I.stats],
  ['search', T('Поиск'), I.search],
  ['settings', T('Настройки'), I.settings],
];

export class MainWindow {
  readonly library = new LibraryView();
  private home = new HomeView();
  private wave = new WaveView();
  private history = new HistoryView();
  private stats = new StatsView();
  private search = new SearchView();
  private settings = new SettingsView();
  private profile = new ProfileView();
  private tabs = new Map<Section, HTMLButtonElement>();
  private user = h('button', { type: 'button', class: 'nav-user', title: T('Профиль: скачанные и очередь лайков') });

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
      // a pending check: straight to the queue, where it is passed
      router.go({ name: 'profile', tab: this.user.classList.contains('has-alert') ? 'queue' : undefined });
      this.user.blur();
    });
    const alert = (on: boolean) => {
      this.user.classList.toggle('has-alert', on);
      this.user.title = on ? T('SoundCloud просит пройти проверку: лайки ждут в очереди') : T('Профиль: мои треки, скачанные и очередь лайков');
    };
    void api.captchaWaiting().then(alert, () => {});
    void listen<boolean>('bridge:captcha', (e) => alert(e.payload));
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
      h('span', { class: 'nav-label', text: a.username ?? (a.has_credentials ? '' : T('Не выполнен вход')) }),
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
      case 'home':
        this.center.replaceChildren(this.home.el);
        this.home.show(r.tab);
        return;
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
      case 'stats':
        view = this.stats;
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
      case 'mix':
        view = new MixPageView(r.urn);
        break;
      case 'category':
        view = new CategoryPageView(r.key);
        break;
      case 'chart':
        view = new ChartPageView(r.kind);
        break;
    }
    this.center.replaceChildren(view.el);
    view.show?.();
  }
}
