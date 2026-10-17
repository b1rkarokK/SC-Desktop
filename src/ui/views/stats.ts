// «Итоги» (variant A): period, four numbers, top tracks and artists, genres
// and when you listen. Everything comes from the local listening history.
import { api, coverUrl, errorMessage, fmtCount, plural, type Stats, type Track } from '../api';
import { h, toast } from '../dom';
import { T } from '../i18n';
import { openArtist } from '../router';
import { SegTabs } from '../seg_tabs';
import { emptyState, sectionTitle, viewHead, type View } from './common';

type Period = 'day' | 'week' | 'month' | 'year' | 'all';

function hours(ms: number): string {
  const h = ms / 3_600_000;
  return h >= 10 ? String(Math.round(h)) : h.toFixed(1).replace('.', ',');
}

/** «1 ч 05 мин» / «54 мин» */
function fmtMinutes(n: number): string {
  if (n < 60) return T('{0} мин', n);
  const m = n % 60;
  return m ? T('{0} ч {1} мин', Math.floor(n / 60), m) : T('{0} ч', Math.floor(n / 60));
}

function clockTime(sec: number): string {
  const d = new Date(sec * 1000);
  return `${d.getHours()}:${String(d.getMinutes()).padStart(2, '0')}`;
}

function dayTitle(daysAgo: number): string {
  if (daysAgo === 0) return T('Сегодня');
  if (daysAgo === 1) return T('Вчера');
  const d = new Date(Date.now() - daysAgo * 86_400_000);
  return d.toLocaleDateString(undefined, { weekday: 'long', day: 'numeric', month: 'long' });
}

/** «hip-hop & rap» → «Hip-Hop & Rap»; short names that are written in capitals stay so. */
function genreName(g: string): string {
  const caps = new Set(['edm', 'dnb', 'rnb', 'uk', 'us', 'idm']);
  return g.replace(/[\p{L}\p{N}]+/gu, (w) => (caps.has(w) ? w.toUpperCase() : w[0]!.toUpperCase() + w.slice(1)));
}

function tile(label: string, value: string): HTMLElement {
  return h('div', { class: 'st-tile' }, h('span', { class: 'muted small', text: label }), h('b', { text: value }));
}

export class StatsView implements View {
  el: HTMLElement;
  private tabs: SegTabs<Period>;
  private body = h('div', { class: 'view-scroll pad' });
  private daysAgo = 0;

  constructor() {
    this.tabs = new SegTabs<Period>(
      [
        { key: 'day', label: T('День') },
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
      s = await api.statsGet(period, period === 'day' ? this.daysAgo : undefined);
    } catch (e) {
      toast(errorMessage(e), 'error');
      return;
    }
    if (period !== this.tabs.value) return;
    const nav = period === 'day' ? this.dayNav() : null;
    if (!s.plays) {
      this.body.replaceChildren(...(nav ? [nav] : []), emptyState(period === 'day' ? T('В этот день музыки не было.') : T('Пока нечего считать: послушайте музыку, и здесь появятся ваши итоги.')));
      return;
    }
    const top = s.top_tracks.map((x) => x.track);
    const day = period === 'day';
    this.body.replaceChildren(
      ...(nav ? [nav] : []),
      h(
        'div',
        { class: 'st-tiles' },
        tile(day ? T('музыки') : T('часов музыки'), day ? fmtMinutes(Math.round(s.ms / 60_000)) : hours(s.ms)),
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
      sectionTitle(day ? T('Когда вы слушали') : T('В какие часы слушаете')),
      this.clock(s.hours, day),
      ...(day && s.timeline.length ? [sectionTitle(T('Что играло')), ...s.timeline.map((x, i) => this.playRow(x, i, s.timeline.map((y) => y.track)))] : []),
    );
  }

  /** ‹ Вчера › above a single day */
  private dayNav(): HTMLElement {
    const prev = h('button', { type: 'button', class: 'btn-icon', title: T('Раньше'), text: '‹' });
    const next = h('button', { type: 'button', class: 'btn-icon', title: T('Позже'), text: '›' });
    next.disabled = this.daysAgo === 0;
    prev.addEventListener('click', () => {
      this.daysAgo += 1;
      void this.render();
    });
    next.addEventListener('click', () => {
      this.daysAgo = Math.max(0, this.daysAgo - 1);
      void this.render();
    });
    return h('div', { class: 'st-daynav' }, prev, h('b', { text: dayTitle(this.daysAgo) }), next);
  }

  private playRow(x: { track: Track; at: number }, i: number, all: Track[]): HTMLElement {
    const r = this.trackRow(x.track, 0, i, all);
    r.querySelector('.st-n')!.textContent = clockTime(x.at);
    r.querySelector('.st-n')!.classList.add('is-time');
    r.lastElementChild!.remove();
    return r;
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

  /** Ten genres at first, the rest on a click. A track counts under its genre and its tags, so the shares are of all plays and do not add up to 100. */
  private genres(s: Stats): HTMLElement[] {
    const total = s.plays || 1;
    const rows = s.genres.map(([g, c]) => {
      const pct = Math.min(100, Math.round((c / total) * 100));
      return h(
        'div',
        { class: 'st-genre' },
        h('span', { class: 'st-genre-name', title: g, text: genreName(g) }),
        h('div', { class: 'st-bar' }, h('div', { class: 'st-bar-fill', style: `width:${pct}%` })),
        h('span', { class: 'muted small', text: `${pct}%` }),
      );
    });
    if (rows.length <= 10) return rows;
    rows.slice(10).forEach((r) => r.classList.add('is-hidden'));
    const more = h('button', { type: 'button', class: 'btn btn-quiet st-more', text: T('Показать все жанры ({0})', rows.length) });
    more.addEventListener('click', () => {
      rows.forEach((r) => r.classList.remove('is-hidden'));
      more.remove();
    });
    return [...rows, more];
  }

  /** Minutes of music in each hour of the day: a grid, a value over the loudest hours, a tick every three hours. */
  private clock(byHour: number[], day: boolean): HTMLElement {
    const max = Math.max(1, ...byHour);
    const peak = byHour.indexOf(Math.max(...byHour));
    const span = (hr: number) => `${hr}:00–${(hr + 1) % 24}:00`;
    const bars = byHour.map((n, hr) => {
      const col = h(
        'div',
        { class: 'st-col' + (n && hr === peak ? ' is-peak' : ''), title: T('{0} · {1}', span(hr), n ? fmtMinutes(n) : T('тишина')) },
        h('span', { class: 'st-val', text: n && (day || hr === peak) ? String(n) : '' }),
        h('i', { style: `height:${n ? Math.max(3, Math.round((n / max) * 100)) : 0}%` }),
      );
      return col;
    });
    const ticks = [0, 3, 6, 9, 12, 15, 18, 21].map((hr) => h('span', { style: `left:${(hr / 24) * 100}%`, text: `${hr}:00` }));
    return h(
      'div',
      { class: 'st-clock' },
      h(
        'div',
        { class: 'st-plot' },
        h('div', { class: 'st-grid' }, h('i', { 'data-v': fmtMinutes(max) }), h('i', { 'data-v': fmtMinutes(Math.round(max / 2)) }), h('i', { 'data-v': '0' })),
        h('div', { class: 'st-hours' }, ...bars),
      ),
      h('div', { class: 'st-hours-axis muted small' }, ...ticks),
      h('div', { class: 'muted small st-peak', text: byHour.some((n) => n) ? T('Чаще всего: {0}, {1}', span(peak), fmtMinutes(byHour[peak]!)) : '' }),
      h('div', { class: 'muted small', text: day ? T('Столбик: сколько минут музыки началось в этот час.') : T('Столбик: сколько минут музыки за период началось в этот час.') }),
    );
  }
}
