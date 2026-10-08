// Grid cards for playlists/albums and artists.
import { api, coverUrl, errorMessage, fmtCount, plural, type Playlist, type User } from './api';
import { h, toast } from './dom';
import { icon, I } from './icons';
import { artistMenu, playlistMenu } from './menus';
import { openArtist, openPlaylist } from './router';

export function playlistGrid(items: Playlist[]): HTMLElement {
  const grid = h('div', { class: 'grid grid-cards' });
  for (const p of items) {
    const img = h('img', { class: 'card-cover', alt: '', loading: 'lazy', decoding: 'async' });
    const src = coverUrl(p.artwork_url, 't300x300');
    if (src) img.src = src;
    const play = h('button', { type: 'button', class: 'card-play', title: 'Слушать', 'aria-label': 'Слушать' }, icon(I.play));
    play.addEventListener('click', async (e) => {
      e.stopPropagation();
      try {
        const page = await api.playlistPage(p.id);
        if (page.tracks.length) {
          await api.playTracks(page.tracks, 0);
          void api.historyPlaylistAdd(p);
        }
      } catch (err) {
        toast(errorMessage(err), 'error');
      }
    });
    const sub = [p.own ? 'Мой' : p.artist, p.is_album && p.year ? p.year : null, `${p.track_count} ${plural(p.track_count, 'трек', 'трека', 'треков')}`]
      .filter(Boolean)
      .join(' · ');
    const card = h(
      'div',
      { class: 'card', tabindex: 0, role: 'button' },
      h('div', { class: 'card-art' }, img, play),
      h('div', { class: 'card-title', dir: 'auto', text: p.title }),
      h('div', { class: 'card-sub', dir: 'auto', text: sub }),
    );
    card.addEventListener('click', () => openPlaylist(p.id));
    card.addEventListener('contextmenu', (e) => playlistMenu(e, p));
    card.addEventListener('keydown', (e) => e.key === 'Enter' && openPlaylist(p.id));
    grid.append(card);
  }
  return grid;
}

export function artistGrid(items: User[]): HTMLElement {
  const grid = h('div', { class: 'grid grid-artists' });
  for (const u of items) {
    const img = h('img', { class: 'artist-avatar', alt: '', loading: 'lazy', decoding: 'async' });
    const src = coverUrl(u.avatar_url, 't300x300');
    if (src) img.src = src;
    const card = h(
      'div',
      { class: 'artist-card', tabindex: 0, role: 'button' },
      img,
      h('div', { class: 'card-title center', dir: 'auto', text: u.username }),
      h('div', { class: 'card-sub center', text: u.followers_count !== null ? `${fmtCount(u.followers_count)} подписчиков` : '' }),
    );
    card.addEventListener('click', () => openArtist(u.id));
    card.addEventListener('contextmenu', (e) => artistMenu(e, u));
    card.addEventListener('keydown', (e) => e.key === 'Enter' && openArtist(u.id));
    grid.append(card);
  }
  return grid;
}

export function userChips(items: User[]): HTMLElement {
  const row = h('div', { class: 'user-chips' });
  for (const u of items) {
    const img = h('img', { class: 'chip-avatar', alt: '', loading: 'lazy' });
    const src = coverUrl(u.avatar_url, 't67x67');
    if (src) img.src = src;
    const card = h(
      'button',
      { type: 'button', class: 'user-chip panel' },
      img,
      h(
        'div',
        { class: 'user-chip-meta' },
        h('div', { class: 'card-title', dir: 'auto', text: u.username }),
        h('div', { class: 'card-sub', text: u.followers_count !== null ? `${fmtCount(u.followers_count)} подписчиков` : '' }),
      ),
    );
    card.addEventListener('click', () => openArtist(u.id));
    card.addEventListener('contextmenu', (e) => artistMenu(e, u));
    row.append(card);
  }
  return row;
}
