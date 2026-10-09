// «Главная»: SoundCloud's own shelves (mixes, stations, trending by genre,
// curated playlists) and «Лента»: posts and reposts of followings, by day.
import { api, coverUrl, errorMessage, plural, type FeedItem, type HomeCard, type HomeSection, type Track } from '../api';
import { h, toast } from '../dom';
import { icon, I } from '../icons';
import { listen } from '@tauri-apps/api/event';
import { CATEGORY_GROUPS, type Category } from '../categories';
import { openCategory, openChart, openMix, openPlaylist, type HomeTab } from '../router';
import { SegTabs } from '../seg_tabs';
import { btn, emptyState, sectionTitle, staticTrackList, viewHead, type View } from './common';

async function cardTracks(c: HomeCard): Promise<Track[]> {
  if (c.kind === 'mix' && c.urn) return (await api.mixPage(c.urn)).tracks;
  return (await api.playlistPage(c.id)).tracks;
}

function open(c: HomeCard): void {
  if (c.kind === 'mix' && c.urn) openMix(c.urn);
  else openPlaylist(c.id);
}

function shelf(cards: HomeCard[]): HTMLElement {
  const row = h('div', { class: 'shelf' });
  for (const c of cards) {
    const img = h('img', { class: 'card-cover', alt: '', loading: 'lazy', decoding: 'async' });
    const src = coverUrl(c.artwork_url, 't300x300');
    if (src) img.src = src;
    const play = h('button', { type: 'button', class: 'card-play', title: 'Слушать', 'aria-label': 'Слушать' }, icon(I.play));
    play.addEventListener('click', async (e) => {
      e.stopPropagation();
      try {
        const tracks = await cardTracks(c);
        if (tracks.length) await api.playTracks(tracks, 0);
        else toast('Здесь пока пусто');
      } catch (err) {
        toast(errorMessage(err), 'error');
      }
    });
    const sub = c.subtitle || (c.track_count ? `${c.track_count} ${plural(c.track_count, 'трек', 'трека', 'треков')}` : '');
    const card = h(
      'div',
      { class: 'card', tabindex: 0, role: 'button', title: c.title },
      h('div', { class: 'card-art' }, img, play),
      h('div', { class: 'card-title', dir: 'auto', text: c.title }),
      h('div', { class: 'card-sub', dir: 'auto', text: sub }),
    );
    card.addEventListener('click', () => open(c));
    card.addEventListener('keydown', (e) => e.key === 'Enter' && open(c));
    row.append(card);
  }
  return row;
}

function dayLabel(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return 'Раньше';
  const today = new Date();
  const start = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((start(today) - start(d)) / 86_400_000);
  if (diff === 0) return 'Сегодня';
  if (diff === 1) return 'Вчера';
  return d.toLocaleDateString('ru-RU', { day: 'numeric', month: 'long', year: d.getFullYear() === today.getFullYear() ? undefined : 'numeric' });
}

export class HomeView implements View {
  el: HTMLElement;
  private tabs: SegTabs<HomeTab>;
  private body = h('div', { class: 'view-scroll pad' });
  private refreshBtn: HTMLButtonElement;
  private sections: HomeSection[] | null = null;
  private feed: FeedItem[] = [];
  private feedNext: string | null = null;
  private feedLoaded = false;

  constructor() {
    this.refreshBtn = btn('Обновить', I.refresh);
    this.refreshBtn.classList.add('btn-quiet');
    this.refreshBtn.addEventListener('click', () => void this.reload());
    this.tabs = new SegTabs<HomeTab>(
      [
        { key: 'home', label: 'Главная' },
        { key: 'categories', label: 'Категории' },
        { key: 'feed', label: 'Лента' },
      ],
      'home',
      () => this.render(),
    );
    this.el = h('section', { class: 'view' }, viewHead('Главная', this.refreshBtn), h('div', { class: 'tabs-row' }, this.tabs.el), this.body);
  }

  show(tab?: HomeTab): void {
    if (tab) this.tabs.select(tab);
    if (this.sections === null) void this.loadHome(false);
    if (!this.feedLoaded) void this.loadFeed(false);
    this.render();
  }

  private async reload(): Promise<void> {
    this.refreshBtn.disabled = true;
    try {
      if (this.tabs.value === 'home') await this.loadHome(true);
      else await this.loadFeed(false);
    } finally {
      this.refreshBtn.disabled = false;
    }
  }

  private async loadHome(force: boolean): Promise<void> {
    try {
      this.sections = await api.homeSections(force);
    } catch (e) {
      this.sections = this.sections ?? [];
      toast(errorMessage(e), 'error');
    }
    if (this.tabs.value === 'home') this.render();
  }

  private async loadFeed(more: boolean): Promise<void> {
    try {
      const page = await api.feedPage(more ? this.feedNext : null);
      const seen = new Set(more ? this.feed.map((i) => i.track.id) : []);
      const fresh = page.items.filter((i) => !seen.has(i.track.id));
      this.feed = more ? this.feed.concat(fresh) : fresh;
      this.feedNext = page.next;
      this.feedLoaded = true;
    } catch (e) {
      toast(errorMessage(e), 'error');
    }
    if (this.tabs.value === 'feed') this.render();
  }

  private render(): void {
    this.refreshBtn.hidden = this.tabs.value === 'categories';
    if (this.tabs.value === 'home') this.renderHome();
    else if (this.tabs.value === 'categories') this.renderCategories();
    else this.renderFeed();
  }

  private renderHome(): void {
    if (this.sections === null) {
      this.body.replaceChildren(emptyState('Загрузка…'));
      return;
    }
    if (!this.sections.length) {
      this.body.replaceChildren(emptyState('SoundCloud пока ничего не подобрал. Послушайте и полайкайте треки, и здесь появятся миксы.'));
      return;
    }
    this.body.replaceChildren(...this.sections.flatMap((s) => [sectionTitle(s.title), shelf(s.cards)]));
  }

  private covers: Record<string, string[]> = {};
  private coversAsked = false;

  /** «Ящик пластинок»: a fan of the top-3 covers and an orange mark. */
  private renderCategories(): void {
    const tile = (c: Category) => {
      const urls = this.covers[c.key] ?? [];
      // back to front: №3, №2, №1 on top
      const fan = h('div', { class: 'crate' });
      for (let i = 2; i >= 0; i--) {
        const src = coverUrl(urls[i], 't300x300');
        fan.append(
          src
            ? h('img', { class: `crate-cover crate-${i}`, alt: '', loading: 'lazy', decoding: 'async', src })
            : h('div', { class: `crate-cover crate-${i} crate-empty` }, i === 0 ? (c.icon ? icon(c.icon) : h('span', { class: 'cat-mark', text: c.mark ?? '' })) : null),
        );
      }
      const mark = c.pick ? 'только у нас' : c.chart ? (/:\d{4}$/.test(c.chart) ? 'по годам' : '№1 сейчас') : c.mix ? '№1 сегодня' : 'лучшее';
      const b = h(
        'button',
        { type: 'button', class: 'cat-tile' },
        h('span', { class: 'cat-chip', text: mark }),
        h('span', { class: 'cat-title', text: c.title }),
        h('span', { class: 'cat-sub', text: c.note ?? (c.mix ? 'чарт · каждый день' : 'из любимых подборок') }),
        fan,
      );
      b.addEventListener('click', () => (c.chart ? openChart(c.chart) : c.mix ? openMix(c.mix) : openCategory(c.key)));
      return b;
    };
    const top = this.body.scrollTop;
    this.body.replaceChildren(...CATEGORY_GROUPS.flatMap((g) => [sectionTitle(g.title), h('div', { class: 'cat-grid' }, ...g.items.map(tile))]));
    this.body.scrollTop = top;
    if (!this.coversAsked) {
      this.coversAsked = true;
      void listen('categories:covers', () => void this.loadCovers());
      void this.loadCovers();
    }
  }

  private async loadCovers(): Promise<void> {
    const tiles = CATEGORY_GROUPS.flatMap((g) => g.items).map((c) => ({ key: c.key, mix: c.mix ?? null, queries: c.queries ?? null, pick: c.pick ?? null, chart: c.chart ?? null }));
    try {
      const got = await api.categoryCovers(tiles);
      const changed = Object.keys(got).some((k) => String(got[k]) !== String(this.covers[k]));
      this.covers = got;
      if (changed && this.tabs.value === 'categories') this.renderCategories();
    } catch {
      /* covers are cosmetic */
    }
  }

  private renderFeed(): void {
    if (!this.feedLoaded) {
      this.body.replaceChildren(emptyState('Загрузка…'));
      return;
    }
    if (!this.feed.length) {
      this.body.replaceChildren(emptyState('В ленте пусто. Подпишитесь на артистов, и здесь появятся их новые треки и репосты.'));
      return;
    }
    // by day; the whole feed plays as one queue from the clicked track
    const all = this.feed.map((i) => i.track);
    const frag = document.createDocumentFragment();
    let label = '';
    let group: FeedItem[] = [];
    const flush = () => {
      if (!group.length) return;
      const reposts = group.filter((i) => i.reposted_by.length).length;
      frag.append(sectionTitle(reposts ? `${label} · репостов: ${reposts}` : label));
      const offset = all.indexOf(group[0]!.track);
      const list = staticTrackList(group.map((i) => i.track));
      // play from the whole feed, not just this day
      list.addEventListener(
        'click',
        (e) => {
          const row = (e.target as HTMLElement).closest('[data-index]') as HTMLElement | null;
          if (!row || (e.target as HTMLElement).closest('button')) return;
          e.stopImmediatePropagation();
          api.playTracks(all, offset + Number(row.dataset.index)).catch((err) => toast(errorMessage(err), 'error'));
        },
        { capture: true },
      );
      frag.append(list);
      group = [];
    };
    for (const it of this.feed) {
      const l = dayLabel(it.at);
      if (l !== label) {
        flush();
        label = l;
      }
      group.push(it);
    }
    flush();
    if (this.feedNext) {
      const more = btn('Показать ещё', I.chevronDown);
      more.addEventListener('click', async () => {
        more.disabled = true;
        await this.loadFeed(true);
      });
      frag.append(h('div', { class: 'row-actions center-row' }, more));
    }
    const top = this.body.scrollTop;
    this.body.replaceChildren(frag);
    this.body.scrollTop = top;
  }
}
