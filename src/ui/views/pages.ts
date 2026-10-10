// Detail pages: track, artist, playlist/album.
import { api, coverUrl, errorMessage, fmtCount, fmtTime, plural, type Track, type User } from '../api';
import { playlistGrid } from '../cards';
import { findCategory } from '../categories';
import { h, toast } from '../dom';
import { icon, iconButton, I } from '../icons';
import { openArtist, openTrackArtist } from '../router';
import { SegTabs } from '../seg_tabs';
import { store } from '../store';
import { btn, emptyState, sectionTitle, staticTrackList, viewHead, type View } from './common';
import { T } from '../i18n';
import { tr } from '../i18n';
import { playlistEditor } from '../playlist_editor';

function hero(art: HTMLElement, ...meta: (HTMLElement | null)[]): HTMLElement {
  return h('div', { class: 'hero' }, art, h('div', { class: 'hero-meta' }, ...meta));
}

function coverImg(url: string | null | undefined, cls: string): HTMLImageElement {
  const img = h('img', { class: cls, alt: '' });
  const src = coverUrl(url, 't500x500');
  if (src) img.src = src;
  return img;
}

function play(tracks: Track[], i = 0): void {
  if (tracks.length) api.playTracks(tracks, i).catch((e) => toast(errorMessage(e), 'error'));
}

function waveFrom(trackId: number | null, artistId: number | null): void {
  api.waveStartFrom(trackId, artistId).then(
    (n) => toast(T('Волна: {0} треков', n)),
    (e) => toast(errorMessage(e), 'error'),
  );
}

// ------------------------------------------------------------------ track

export class TrackPageView implements View {
  el: HTMLElement;

  constructor(id: number) {
    const body = h('div', { class: 'view-scroll pad' }, emptyState(T('Загрузка…')));
    this.el = h('section', { class: 'view' }, viewHead(null), body);
    api.trackPage(id).then(
      (p) => {
        const t = p.track;
        const artist = h('button', { type: 'button', class: 'link hero-artist', dir: 'auto', text: t.artist });
        artist.addEventListener('click', () => void openTrackArtist(t));
        const meta = [t.genre, fmtTime(t.duration_ms), p.year, ...p.tags.slice(0, 4).map((x) => `#${x}`)].filter(Boolean).join(' · ');
        const stats = [p.plays !== null ? T('{0} прослушиваний', fmtCount(p.plays)) : null, p.likes !== null ? T('{0} лайков', fmtCount(p.likes)) : null]
          .filter(Boolean)
          .join(' · ');

        const playBtn = btn(T('Слушать'), I.play, true);
        playBtn.addEventListener('click', () => play([t, ...p.related]));
        const like = iconButton(I.heart, T('Лайкнуть'), 'btn btn-square');
        like.addEventListener('click', () => void store.setLiked(t, !store.liked.has(t.id)));
        const dislike = iconButton(I.dislike, T('Не рекомендовать'), 'btn btn-square');
        dislike.addEventListener('click', () => void store.setDisliked(t, !store.disliked.has(t.id)));
        const wave = btn(T('Волна по треку'), I.wave);
        wave.addEventListener('click', () => waveFrom(t.id, null));
        const ext = iconButton(I.external, T('Открыть на SoundCloud'), 'btn btn-square');
        ext.addEventListener('click', () => t.permalink_url && void api.openExternal(t.permalink_url));
        const sync = () => {
          like.classList.toggle('is-on', store.liked.has(t.id));
          dislike.classList.toggle('is-on', store.disliked.has(t.id));
        };
        store.on('likes', sync);
        store.on('dislikes', sync);
        sync();

        body.replaceChildren(
          hero(
            coverImg(t.artwork_url, 'hero-cover'),
            h('div', { class: 'eyebrow', text: T('Трек') }),
            h('h2', { class: 'hero-title', dir: 'auto', text: t.title }),
            artist,
            h('div', { class: 'muted small', text: meta }),
            h('div', { class: 'muted small', text: stats }),
            h('div', { class: 'hero-actions' }, playBtn, like, dislike, wave, ext),
          ),
          sectionTitle(T('Похожие треки')),
          p.related.length ? staticTrackList(p.related) : emptyState(T('Нет похожих треков.')),
        );
      },
      (e) => body.replaceChildren(emptyState(errorMessage(e))),
    );
  }
}

// ----------------------------------------------------------------- artist

type ArtistTab = 'popular' | 'tracks' | 'albums' | 'playlists' | 'reposts';

export class ArtistPageView implements View {
  el: HTMLElement;
  private content = h('div');
  private seq = 0;

  constructor(private id: number) {
    const body = h('div', { class: 'view-scroll pad' }, emptyState(T('Загрузка…')));
    this.el = h('section', { class: 'view' }, viewHead(null), body);
    api.artistGet(id).then(
      (u) => body.replaceChildren(this.header(u), this.content),
      (e) => body.replaceChildren(emptyState(errorMessage(e))),
    );
  }

  private header(u: User): HTMLElement {
    const tabs = new SegTabs<ArtistTab>(
      [
        { key: 'popular', label: T('Популярное') },
        { key: 'tracks', label: T('Треки') },
        { key: 'albums', label: T('Альбомы') },
        { key: 'playlists', label: T('Плейлисты') },
        { key: 'reposts', label: T('Репосты') },
      ],
      'popular',
      (k) => void this.load(k),
    );
    const playBtn = btn(T('Слушать'), I.play, true);
    playBtn.addEventListener('click', async () => {
      const s = await api.artistSection(u.id, 'popular').catch(() => null);
      if (s) play(s.tracks);
    });
    const follow = h('button', { type: 'button', class: 'btn' });
    const renderFollow = () => {
      const on = store.following.has(u.id);
      follow.replaceChildren(icon(on ? I.following : I.plus), h('span', { text: on ? T('Вы подписаны') : T('Подписаться') }));
      follow.classList.toggle('is-on', on);
    };
    follow.addEventListener('click', () => void store.setFollow(u, !store.following.has(u.id)));
    store.on('follows', renderFollow);
    renderFollow();
    const wave = btn(T('Волна по артисту'), I.wave);
    wave.addEventListener('click', () => waveFrom(null, u.id));
    const ban = iconButton(I.banArtist, T('Не ставить в волну'), 'btn btn-square');
    ban.addEventListener('click', () =>
      api.waveDislikeArtist(u.id, u.username).then(
        () => toast(T('{0} исключён из волны', u.username)),
        (e) => toast(errorMessage(e), 'error'),
      ),
    );
    const sub = [u.city, u.followers_count !== null ? T('{0} подписчиков', fmtCount(u.followers_count)) : null,
      u.track_count !== null ? T('{0} {1}', u.track_count, plural(u.track_count, 'трек', 'трека', 'треков')) : null]
      .filter(Boolean)
      .join(' · ');
    void this.load('popular');
    return h(
      'div',
      {},
      hero(
        coverImg(u.avatar_url, 'hero-cover round'),
        h('div', { class: 'eyebrow', text: T('Артист') }),
        h('h2', { class: 'hero-title', dir: 'auto', text: u.username }),
        h('div', { class: 'muted small', text: sub }),
        h('div', { class: 'hero-actions' }, playBtn, follow, wave, ban),
      ),
      h('div', { class: 'tabs-row flush' }, tabs.el),
    );
  }

  private async load(section: ArtistTab): Promise<void> {
    const seq = ++this.seq;
    this.content.replaceChildren(emptyState(T('Загрузка…')));
    try {
      const s = await api.artistSection(this.id, section);
      if (seq !== this.seq) return;
      if (s.tracks.length) this.content.replaceChildren(staticTrackList(s.tracks));
      else if (s.playlists.length) this.content.replaceChildren(playlistGrid(s.playlists));
      else this.content.replaceChildren(emptyState(T('Здесь пока пусто.')));
    } catch (e) {
      if (seq === this.seq) this.content.replaceChildren(emptyState(errorMessage(e)));
    }
  }
}

// --------------------------------------------------------------- playlist

export class PlaylistPageView implements View {
  el: HTMLElement;

  constructor(id: number) {
    const body = h('div', { class: 'view-scroll pad' }, emptyState(T('Загрузка…')));
    this.el = h('section', { class: 'view' }, viewHead(null), body);
    api.playlistPage(id).then(
      ({ playlist: p, tracks: loaded }) => {
        let tracks = loaded;
        const artist = h('button', { type: 'button', class: 'link hero-artist', dir: 'auto', text: p.artist });
        artist.addEventListener('click', () => openArtist(p.user_id));
        const total = tracks.reduce((n, t) => n + t.duration_ms, 0);
        const remember = () => void api.historyPlaylistAdd(p);
        const playBtn = btn(T('Слушать'), I.play, true);
        playBtn.addEventListener('click', () => {
          play(tracks);
          remember();
        });
        const shuffleBtn = iconButton(I.shuffle, T('Перемешать'), 'btn btn-square');
        shuffleBtn.addEventListener('click', async () => {
          await api.setShuffle(true);
          play(tracks, Math.floor(Math.random() * tracks.length));
          remember();
        });
        const wave = btn(p.is_album ? T('Волна по альбому') : T('Волна по плейлисту'), I.wave);
        wave.addEventListener('click', () =>
          api.waveStartPlaylist(p.id).then(
            (n) => toast(T('Волна: {0} треков', n)),
            (e) => toast(errorMessage(e), 'error'),
          ),
        );
        const ext = iconButton(I.external, T('Открыть на SoundCloud'), 'btn btn-square');
        ext.addEventListener('click', () => p.permalink_url && void api.openExternal(p.permalink_url));
        const sub = [p.year, T('{0} {1}', tracks.length, plural(tracks.length, 'трек', 'трека', 'треков')), fmtTime(total)].filter(Boolean).join(' · ');
        // own playlist: «Изменить» → drag to reorder, ✕ to remove, saved on SoundCloud
        const listBox = h('div');
        const showList = () =>
          listBox.replaceChildren(tracks.length ? staticTrackList(tracks, remember) : emptyState(T('Плейлист пуст или недоступен.')));
        const own = !p.is_album && store.auth.user_id === p.user_id;
        const editBtn = own ? btn(T('Изменить'), I.edit) : null;
        editBtn?.addEventListener('click', () =>
          listBox.replaceChildren(
            playlistEditor(
              p.id,
              tracks,
              (saved) => {
                tracks = saved;
                showList();
              },
              showList,
            ),
          ),
        );
        showList();
        body.replaceChildren(
          hero(
            coverImg(p.artwork_url ?? tracks[0]?.artwork_url, 'hero-cover'),
            h('div', { class: 'eyebrow', text: p.is_album ? T('Альбом') : T('Плейлист') }),
            h('h2', { class: 'hero-title', dir: 'auto', text: p.title }),
            artist,
            h('div', { class: 'muted small', text: sub }),
            h('div', { class: 'hero-actions' }, playBtn, shuffleBtn, wave, editBtn, ext),
          ),
          listBox,
        );
      },
      (e) => body.replaceChildren(emptyState(errorMessage(e))),
    );
  }
}

/** SoundCloud system playlist: "Ваш микс 1", "Daily Drops", a station, a genre chart. */
export class MixPageView implements View {
  el: HTMLElement;

  constructor(urn: string) {
    const body = h('div', { class: 'view-scroll pad' }, emptyState(T('Загрузка…')));
    this.el = h('section', { class: 'view' }, viewHead(null), body);
    api.mixPage(urn).then(
      (p) => {
        const tracks = p.tracks;
        const total = tracks.reduce((n, t) => n + t.duration_ms, 0);
        const playBtn = btn(T('Слушать'), I.play, true);
        playBtn.addEventListener('click', () => play(tracks));
        const shuffleBtn = iconButton(I.shuffle, T('Перемешать'), 'btn btn-square');
        shuffleBtn.addEventListener('click', async () => {
          await api.setShuffle(true);
          play(tracks, Math.floor(Math.random() * tracks.length));
        });
        const ext = iconButton(I.external, T('Открыть на SoundCloud'), 'btn btn-square');
        ext.addEventListener('click', () => p.permalink_url && void api.openExternal(p.permalink_url));
        const sub = [T('{0} {1}', tracks.length, plural(tracks.length, 'трек', 'трека', 'треков')), fmtTime(total)].join(' · ');
        body.replaceChildren(
          hero(
            coverImg(p.artwork_url ?? tracks[0]?.artwork_url, 'hero-cover'),
            h('div', { class: 'eyebrow', text: T('Микс SoundCloud') }),
            h('h2', { class: 'hero-title', dir: 'auto', text: p.title }),
            p.description ? h('div', { class: 'muted small mix-desc', dir: 'auto', text: tr(p.description) }) : null,
            h('div', { class: 'muted small', text: sub }),
            h('div', { class: 'hero-actions' }, playBtn, shuffleBtn, ext),
          ),
          tracks.length ? staticTrackList(tracks) : emptyState(T('Микс пуст или недоступен.')),
        );
      },
      (e) => body.replaceChildren(emptyState(errorMessage(e))),
    );
  }
}

/** «Категории» tile: an app pick («Только в SC Desk») or a theme ("90-е", "AI-треки") as tracks. */
export class CategoryPageView implements View {
  el: HTMLElement;

  constructor(key: string) {
    const c = findCategory(key);
    const body = h('div', { class: 'view-scroll pad' }, emptyState(c?.queries ? T('Собираю треки из лучших подборок…') : T('Загрузка…')));
    this.el = h('section', { class: 'view' }, viewHead(c?.title ?? T('Категория')), body);
    const empty: Record<string, string> = {
      'pick:radar': T('Пока не из чего собрать: полайкайте треки, и здесь появятся похожие малоизвестные артисты.'),
      'pick:forgotten': T('Забытых лайков нет: вы слушали всё, что лайкали.'),
      'pick:year-ago': T('Год назад в это время лайков не было.'),
      'pick:repeat': T('Пока нечего показать: здесь будут треки, которые вы слушали чаще всего за месяц.'),
    };
    const load = c?.pick ? api.picks(c.pick) : c?.queries ? api.categoryTracks(c.key, c.queries) : null;
    if (!c || !load) {
      body.replaceChildren(emptyState(T('Категория не найдена.')));
      return;
    }
    load.then(
      (tracks) => {
        if (!tracks.length) {
          body.replaceChildren(emptyState(empty[c.key] ?? T('Пока пусто.')));
          return;
        }
        const playBtn = btn(T('Слушать'), I.play, true);
        playBtn.addEventListener('click', () => play(tracks));
        const shuffleBtn = iconButton(I.shuffle, T('Перемешать'), 'btn btn-square');
        shuffleBtn.addEventListener('click', async () => {
          await api.setShuffle(true);
          play(tracks, Math.floor(Math.random() * tracks.length));
        });
        const note = c.note ? c.note[0]!.toUpperCase() + c.note.slice(1) + '. ' : '';
        const count = T('{0} {1}', tracks.length, plural(tracks.length, 'трек', 'трека', 'треков'));
        body.replaceChildren(
          h('p', { class: 'muted small', text: c.queries ? T('{0} из самых любимых подборок, обновляется раз в день.', count) : `${note}${count}.` }),
          h('div', { class: 'hero-actions pick-actions' }, playBtn, shuffleBtn),
          staticTrackList(tracks),
        );
      },
      (e) => body.replaceChildren(emptyState(errorMessage(e))),
    );
  }
}

/** A real chart matched onto SoundCloud; year charts get a region switch and a year picker. */
export class ChartPageView implements View {
  el: HTMLElement;
  private body = h('div', { class: 'view-scroll pad' });

  constructor(private kind: string) {
    this.el = h('section', { class: 'view' }, viewHead(null), this.body);
    void this.load();
  }

  private picker(): HTMLElement | null {
    const [region, what] = this.kind.split(':') as [string, string];
    if (!/^\d{4}$/.test(what)) return null;
    const box = h('div', { class: 'chart-picker' });
    const regions = h('div', { class: 'chips' });
    const names: [string, string][] = [
      ['ru', T('Россия')],
      ['world', T('Мир')],
    ];
    for (const [r, label] of names) {
      const b = h('button', { type: 'button', class: `chip-btn${r === region ? ' is-active' : ''}`, text: label });
      b.addEventListener('click', () => this.go(`${r}:${what}`));
      regions.append(b);
    }
    const years = h('div', { class: 'chips chips-scroll' });
    for (let y = 2025; y >= 2001; y--) {
      const b = h('button', { type: 'button', class: `chip-btn${String(y) === what ? ' is-active' : ''}`, text: String(y) });
      b.addEventListener('click', () => this.go(`${region}:${y}`));
      years.append(b);
    }
    box.append(regions, years);
    return box;
  }

  private go(kind: string): void {
    this.kind = kind;
    void this.load();
  }

  private async load(): Promise<void> {
    const kind = this.kind;
    const picker = this.picker();
    const keep = (xs: (HTMLElement | null)[]) => xs.filter((x): x is HTMLElement => x !== null);
    this.body.replaceChildren(...keep([picker, emptyState(T('Ищу песни чарта на SoundCloud. В первый раз это до минуты, дальше мгновенно.'))]));
    try {
      const p = await api.chartPage(kind);
      if (kind !== this.kind) return;
      const tracks = p.tracks;
      const playBtn = btn(T('Слушать'), I.play, true);
      playBtn.addEventListener('click', () => play(tracks));
      const shuffleBtn = iconButton(I.shuffle, T('Перемешать'), 'btn btn-square');
      shuffleBtn.addEventListener('click', async () => {
        await api.setShuffle(true);
        play(tracks, Math.floor(Math.random() * tracks.length));
      });
      const missing = p.missing ? T(' Не нашлось на SoundCloud: {0}.', p.missing) : '';
      this.body.replaceChildren(
        ...keep([
          h('h2', { class: 'hero-title chart-title', text: tr(p.title) }),
          picker,
          h('p', { class: 'muted small', text: `${tr(p.note)}${missing}` }),
          tracks.length ? h('div', { class: 'hero-actions pick-actions' }, playBtn, shuffleBtn) : null,
          tracks.length ? staticTrackList(tracks) : emptyState(T('Пока пусто.')),
        ]),
      );
    } catch (e) {
      if (kind === this.kind) this.body.replaceChildren(...keep([picker, emptyState(errorMessage(e))]));
    }
  }
}
