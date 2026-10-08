// Detail pages: track, artist, playlist/album.
import { api, coverUrl, errorMessage, fmtCount, fmtTime, plural, type Track, type User } from '../api';
import { playlistGrid } from '../cards';
import { h, toast } from '../dom';
import { icon, iconButton, I } from '../icons';
import { openArtist } from '../router';
import { SegTabs } from '../seg_tabs';
import { store } from '../store';
import { btn, emptyState, sectionTitle, staticTrackList, viewHead, type View } from './common';

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
    (n) => toast(`Волна: ${n} треков`),
    (e) => toast(errorMessage(e), 'error'),
  );
}

// ------------------------------------------------------------------ track

export class TrackPageView implements View {
  el: HTMLElement;

  constructor(id: number) {
    const body = h('div', { class: 'view-scroll pad' }, emptyState('Загрузка…'));
    this.el = h('section', { class: 'view' }, viewHead(null), body);
    api.trackPage(id).then(
      (p) => {
        const t = p.track;
        const artist = h('button', { type: 'button', class: 'link hero-artist', dir: 'auto', text: t.artist });
        artist.addEventListener('click', () => openArtist(t.user_id));
        const meta = [t.genre, fmtTime(t.duration_ms), p.year, ...p.tags.slice(0, 4).map((x) => `#${x}`)].filter(Boolean).join(' · ');
        const stats = [p.plays !== null ? `${fmtCount(p.plays)} прослушиваний` : null, p.likes !== null ? `${fmtCount(p.likes)} лайков` : null]
          .filter(Boolean)
          .join(' · ');

        const playBtn = btn('Слушать', I.play, true);
        playBtn.addEventListener('click', () => play([t, ...p.related]));
        const like = iconButton(I.heart, 'Лайкнуть', 'btn btn-square');
        like.addEventListener('click', () => void store.setLiked(t, !store.liked.has(t.id)));
        const dislike = iconButton(I.dislike, 'Не рекомендовать', 'btn btn-square');
        dislike.addEventListener('click', () => void store.setDisliked(t, !store.disliked.has(t.id)));
        const wave = btn('Волна по треку', I.wave);
        wave.addEventListener('click', () => waveFrom(t.id, null));
        const ext = iconButton(I.external, 'Открыть на SoundCloud', 'btn btn-square');
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
            h('div', { class: 'eyebrow', text: 'Трек' }),
            h('h2', { class: 'hero-title', dir: 'auto', text: t.title }),
            artist,
            h('div', { class: 'muted small', text: meta }),
            h('div', { class: 'muted small', text: stats }),
            h('div', { class: 'hero-actions' }, playBtn, like, dislike, wave, ext),
          ),
          sectionTitle('Похожие треки'),
          p.related.length ? staticTrackList(p.related) : emptyState('Нет похожих треков.'),
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
    const body = h('div', { class: 'view-scroll pad' }, emptyState('Загрузка…'));
    this.el = h('section', { class: 'view' }, viewHead(null), body);
    api.artistGet(id).then(
      (u) => body.replaceChildren(this.header(u), this.content),
      (e) => body.replaceChildren(emptyState(errorMessage(e))),
    );
  }

  private header(u: User): HTMLElement {
    const tabs = new SegTabs<ArtistTab>(
      [
        { key: 'popular', label: 'Популярное' },
        { key: 'tracks', label: 'Треки' },
        { key: 'albums', label: 'Альбомы' },
        { key: 'playlists', label: 'Плейлисты' },
        { key: 'reposts', label: 'Репосты' },
      ],
      'popular',
      (k) => void this.load(k),
    );
    const playBtn = btn('Слушать', I.play, true);
    playBtn.addEventListener('click', async () => {
      const s = await api.artistSection(u.id, 'popular').catch(() => null);
      if (s) play(s.tracks);
    });
    const follow = h('button', { type: 'button', class: 'btn' });
    const renderFollow = () => {
      const on = store.following.has(u.id);
      follow.replaceChildren(icon(on ? I.following : I.plus), h('span', { text: on ? 'Вы подписаны' : 'Подписаться' }));
      follow.classList.toggle('is-on', on);
    };
    follow.addEventListener('click', () => void store.setFollow(u, !store.following.has(u.id)));
    store.on('follows', renderFollow);
    renderFollow();
    const wave = btn('Волна по артисту', I.wave);
    wave.addEventListener('click', () => waveFrom(null, u.id));
    const ban = iconButton(I.banArtist, 'Не ставить в волну', 'btn btn-square');
    ban.addEventListener('click', () =>
      api.waveDislikeArtist(u.id, u.username).then(
        () => toast(`${u.username} исключён из волны`),
        (e) => toast(errorMessage(e), 'error'),
      ),
    );
    const sub = [u.city, u.followers_count !== null ? `${fmtCount(u.followers_count)} подписчиков` : null,
      u.track_count !== null ? `${u.track_count} ${plural(u.track_count, 'трек', 'трека', 'треков')}` : null]
      .filter(Boolean)
      .join(' · ');
    void this.load('popular');
    return h(
      'div',
      {},
      hero(
        coverImg(u.avatar_url, 'hero-cover round'),
        h('div', { class: 'eyebrow', text: 'Артист' }),
        h('h2', { class: 'hero-title', dir: 'auto', text: u.username }),
        h('div', { class: 'muted small', text: sub }),
        h('div', { class: 'hero-actions' }, playBtn, follow, wave, ban),
      ),
      h('div', { class: 'tabs-row flush' }, tabs.el),
    );
  }

  private async load(section: ArtistTab): Promise<void> {
    const seq = ++this.seq;
    this.content.replaceChildren(emptyState('Загрузка…'));
    try {
      const s = await api.artistSection(this.id, section);
      if (seq !== this.seq) return;
      if (s.tracks.length) this.content.replaceChildren(staticTrackList(s.tracks));
      else if (s.playlists.length) this.content.replaceChildren(playlistGrid(s.playlists));
      else this.content.replaceChildren(emptyState('Здесь пока пусто.'));
    } catch (e) {
      if (seq === this.seq) this.content.replaceChildren(emptyState(errorMessage(e)));
    }
  }
}

// --------------------------------------------------------------- playlist

export class PlaylistPageView implements View {
  el: HTMLElement;

  constructor(id: number) {
    const body = h('div', { class: 'view-scroll pad' }, emptyState('Загрузка…'));
    this.el = h('section', { class: 'view' }, viewHead(null), body);
    api.playlistPage(id).then(
      ({ playlist: p, tracks }) => {
        const artist = h('button', { type: 'button', class: 'link hero-artist', dir: 'auto', text: p.artist });
        artist.addEventListener('click', () => openArtist(p.user_id));
        const total = tracks.reduce((n, t) => n + t.duration_ms, 0);
        const playBtn = btn('Слушать', I.play, true);
        playBtn.addEventListener('click', () => play(tracks));
        const shuffleBtn = iconButton(I.shuffle, 'Перемешать', 'btn btn-square');
        shuffleBtn.addEventListener('click', async () => {
          await api.setShuffle(true);
          play(tracks, Math.floor(Math.random() * tracks.length));
        });
        const ext = iconButton(I.external, 'Открыть на SoundCloud', 'btn btn-square');
        ext.addEventListener('click', () => p.permalink_url && void api.openExternal(p.permalink_url));
        const sub = [p.year, `${tracks.length} ${plural(tracks.length, 'трек', 'трека', 'треков')}`, fmtTime(total)].filter(Boolean).join(' · ');
        body.replaceChildren(
          hero(
            coverImg(p.artwork_url ?? tracks[0]?.artwork_url, 'hero-cover'),
            h('div', { class: 'eyebrow', text: p.is_album ? 'Альбом' : 'Плейлист' }),
            h('h2', { class: 'hero-title', dir: 'auto', text: p.title }),
            artist,
            h('div', { class: 'muted small', text: sub }),
            h('div', { class: 'hero-actions' }, playBtn, shuffleBtn, ext),
          ),
          tracks.length ? staticTrackList(tracks) : emptyState('Плейлист пуст или недоступен.'),
        );
      },
      (e) => body.replaceChildren(emptyState(errorMessage(e))),
    );
  }
}
