// «Итоги» (variant A): period, four numbers, top tracks and artists, genres
// and when you listen. Everything comes from the local listening history.
import { api, coverUrl, errorMessage, fmtCount, plural, type Stats, type Track } from '../api';
import { h, toast } from '../dom';
import { T } from '../i18n';
import { openArtist } from '../router';
import { SegTabs } from '../seg_tabs';
import { emptyState, sectionTitle, viewHead, type View } from './common';

type Period = 'week' | 'month' | 'year' | 'all';

function hours(ms: number): string {
  const h = ms / 3_600_000;
  return h >= 10 ? String(Math.round(h)) : h.toFixed(1).replace('.', ',');
}

function tile(label: string, value: string): HTMLElement {
  return h('div', { class: 'st-tile' }, h('span', { class: 'muted small', text: label }), h('b', { text: value }));
}

export class StatsView implements View {
  el: HTMLElement;
  private tabs: SegTabs<Period>;
  private body = h('div', { class: 'view-scroll pad' });

  constructor() {
    this.tabs = new SegTabs<Period>(
      [
        { key: 'week', label: T('Неделя') },
        { key: 'month', label: T('Месяц') },
        { key: 'year', label: T('Год') },
        { key: 'all', label: T('Всё время') },
      ],
      'month',
      () => void this.render(),
    );
    this.el = h('section', { class: 'view' }, viewHead(T('Итоги')), h('div', { class: 'tabs-row' }, this.tabs.el), this.body);
  }

  show(): void {
    void this.render();
  }

  private async render(): Promise<void> {
    const period = this.tabs.value;
    let s: Stats;
    try {
      s = await api.statsGet(period);
    } catch (e) {
      toast(errorMessage(e), 'error');
      return;
    }
    if (period !== this.tabs.value) return;
    if (!s.plays) {
      this.body.replaceChildren(emptyState(T('Пока нечего считать: послушайте музыку, и здесь появятся ваши итоги.')));
      return;
    }
    const top = s.top_tracks.map((x) => x.track);
    this.body.replaceChildren(
      h(
        'div',
        { class: 'st-tiles' },
        tile(T('часов музыки'), hours(s.ms)),
        tile(T('прослушиваний'), fmtCount(s.plays)),
        tile(T('артистов'), fmtCount(s.artists)),
        tile(T('дней подряд'), String(s.streak)),
      ),
      h(
        'div',
        { class: 'st-cols' },
        h('div', {}, sectionTitle(T('Топ треков')), ...s.top_tracks.map((x, i) => this.trackRow(x.track, x.plays, i, top))),
        h(
          'div',
          {},
          sectionTitle(T('Топ артистов')),
          ...s.top_artists.map((a, i) => {
            const img = h('img', { class: 'st-cover is-round', alt: '', loading: 'lazy' });
            const src = coverUrl(a.artwork_url, 't67x67');
            if (src) img.src = src;
            const r = h(
              'button',
              { type: 'button', class: 'st-row' },
              h('span', { class: 'st-n', text: String(i + 1) }),
              img,
              h('span', { class: 'st-name', dir: 'auto', text: a.name }),
              h('span', { class: 'muted small', text: T('{0} ч', hours(a.ms)) }),
            );
            r.addEventListener('click', () => openArtist(a.user_id));
            return r;
          }),
        ),
      ),
      s.genres.length ? sectionTitle(T('Жанры')) : h('span'),
      ...this.genres(s),
      sectionTitle(T('Когда слушаете')),
      this.clock(s.hours),
    );
  }

  private trackRow(t: Track, plays: number, i: number, all: Track[]): HTMLElement {
    const img = h('img', { class: 'st-cover', alt: '', loading: 'lazy' });
    const src = coverUrl(t.artwork_url, 't67x67');
    if (src) img.src = src;
    const r = h(
      'button',
      { type: 'button', class: 'st-row', title: T('Слушать') },
      h('span', { class: 'st-n', text: String(i + 1) }),
      img,
      h('span', { class: 'st-name', dir: 'auto' }, h('span', { text: t.title }), h('span', { class: 'muted small', text: ` · ${t.artist}` })),
      h('span', { class: 'muted small', text: T('{0} {1}', plays, plural(plays, 'раз', 'раза', 'раз')) }),
    );
    r.addEventListener('click', () => void api.playTracks(all, i).catch((e) => toast(errorMessage(e), 'error')));
    return r;
  }

  private genres(s: Stats): HTMLElement[] {
    const total = s.genres.reduce((n, [, c]) => n + c, 0) || 1;
    return s.genres.map(([g, c]) => {
      const pct = Math.round((c / total) * 100);
      return h(
        'div',
        { class: 'st-genre' },
        h('span', { class: 'st-genre-name', text: g.replace(/(^|[\s&-])\p{L}/gu, (m) => m.toUpperCase()) }),
        h('div', { class: 'st-bar' }, h('div', { class: 'st-bar-fill', style: `width:${pct}%` })),
        h('span', { class: 'muted small', text: `${pct}%` }),
      );
    });
  }

  private clock(byHour: number[]): HTMLElement {
    const max = Math.max(1, ...byHour);
    return h(
      'div',
      { class: 'st-clock' },
      h(
        'div',
        { class: 'st-hours' },
        ...byHour.map((n, hr) => h('i', { style: `height:${Math.max(2, Math.round((n / max) * 100))}%`, title: T('{0}:00 · {1}', hr, n) })),
      ),
      h('div', { class: 'st-hours-axis muted small' }, h('span', { text: '0:00' }), h('span', { text: '12:00' }), h('span', { text: '23:00' })),
    );
  }
}
