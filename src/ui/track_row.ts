// Shared track row: index, cover, title (→ track page) / artist (→ artist page),
// dislike + like (dislike only on hover or when set), duration.
import { coverUrl, fmtTime, type Track } from './api';
import { icon, I } from './icons';
import { trackMenu } from './menus';
import { addToRoom } from './views/rooms';
import { openTrack, openTrackArtist } from './router';
import { store } from './store';
import type { RowRenderer } from './virtual_list';
import { T } from './i18n';

export type Density = 'compact' | 'normal' | 'large';

/** [row height, cover size] in px for each list density */
const DENSITY: Record<Density, [number, number]> = { compact: [32, 22], normal: [44, 32], large: [60, 48] };

let rowH = DENSITY.normal[0];

/** The height of a track row now (the density of lists in Настройки → Тема). */
export function rowHeight(): number {
  return rowH;
}

/** Sets the row size for the chosen density; the views made after it use it (a change reloads the window). */
export function applyDensity(d: Density | undefined): void {
  const [h, c] = DENSITY[d ?? 'normal'];
  rowH = h;
  document.documentElement.style.setProperty('--row-h', `${h}px`);
  document.documentElement.style.setProperty('--row-cover', `${c}px`);
}

type RowEl = HTMLElement & { __track?: Track };

export function trackRowRenderer(): RowRenderer<Track> {
  return {
    create() {
      const row = document.createElement('div') as RowEl;
      row.className = 'row';
      row.innerHTML =
        '<span class="row-idx"><span class="row-num"></span></span><img class="row-cover" alt="" decoding="async" loading="lazy">' +
        '<div class="row-main"><button type="button" class="row-title link"></button><button type="button" class="row-artist link"></button></div>' +
        '<button type="button" class="btn-icon row-dislike"></button><button type="button" class="btn-icon row-like"></button><span class="row-dur"></span>' +
        '<button type="button" class="btn-icon row-room"></button>';
      const q = <T extends HTMLElement>(s: string) => row.querySelector(s) as T;
      q('.row-idx').append(icon(I.play));
      row.title = T('Нажмите, чтобы включить');
      q('.row-like').append(icon(I.heart));
      q('.row-room').append(icon(I.users));
      q('.row-dislike').append(icon(I.dislike));
      const stop = (e: Event) => e.stopPropagation();
      q('.row-like').addEventListener('click', (e) => {
        stop(e);
        if (row.__track) void store.setLiked(row.__track, !store.liked.has(row.__track.id));
      });
      q('.row-dislike').addEventListener('click', (e) => {
        stop(e);
        if (row.__track) void store.setDisliked(row.__track, !store.disliked.has(row.__track.id));
      });
      q('.row-room').addEventListener('click', (e) => {
        stop(e);
        if (row.__track) void addToRoom(row.__track);
      });
      q('.row-title').addEventListener('click', (e) => {
        stop(e);
        if (row.__track) openTrack(row.__track.id);
      });
      q('.row-artist').addEventListener('click', (e) => {
        stop(e);
        if (row.__track) void openTrackArtist(row.__track);
      });
      // «Слушать» in the menu = same as clicking the row (plays its list from here)
      row.addEventListener('contextmenu', (e) => {
        if (row.__track) trackMenu(e, row.__track, () => row.click());
      });
      return row;
    },
    update(el, track, index) {
      const row = el as RowEl;
      row.__track = track;
      const idx = row.querySelector('.row-num') as HTMLElement;
      const img = row.children[1] as HTMLImageElement;
      const title = row.querySelector('.row-title') as HTMLButtonElement;
      const artist = row.querySelector('.row-artist') as HTMLButtonElement;
      const like = row.querySelector('.row-like') as HTMLButtonElement;
      const dislike = row.querySelector('.row-dislike') as HTMLButtonElement;
      const dur = row.children[5] as HTMLElement;

      idx.textContent = String(index + 1);
      if (!track) {
        row.classList.add('row-placeholder');
        row.classList.remove('row-current', 'row-disliked');
        title.textContent = '';
        artist.textContent = '';
        dur.textContent = '';
        img.removeAttribute('src');
        like.hidden = dislike.hidden = true;
        return;
      }
      row.classList.remove('row-placeholder');
      row.classList.toggle('row-current', store.currentId() === track.id);
      const isDisliked = store.disliked.has(track.id);
      row.classList.toggle('row-disliked', isDisliked);
      title.textContent = track.title;
      title.title = T('{0} — открыть страницу трека', track.title);
      title.dir = 'auto';
      artist.textContent = track.artist;
      artist.title = T('{0} — открыть страницу автора', track.artist);
      artist.dir = 'auto';
      dur.textContent = fmtTime(track.duration_ms);

      const liked = store.liked.has(track.id);
      like.hidden = dislike.hidden = false;
      (row.querySelector('.row-room') as HTMLButtonElement).title = T('В очередь комнаты');
      like.classList.toggle('is-on', liked);
      like.title = liked ? T('Убрать из лайков') : T('Лайкнуть');
      dislike.classList.toggle('is-on', isDisliked);
      dislike.title = isDisliked ? T('Снова рекомендовать') : T('Не рекомендовать');

      const src = coverUrl(track.artwork_url, 't67x67');
      if (src) {
        if (img.getAttribute('src') !== src) img.src = src;
      } else img.removeAttribute('src');
    },
  };
}
