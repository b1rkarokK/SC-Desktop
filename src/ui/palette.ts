// Ctrl+K: quick search over everything, a bar at the top with filters.
import { api, coverUrl, errorMessage, fmtTime, type Playlist, type Track, type User } from './api';
import { h, toast } from './dom';
import { T } from './i18n';
import { router } from './router';
import { store } from './store';
import { addToRoom, room } from './views/rooms';

type Filter = 'all' | 'tracks' | 'artists' | 'playlists' | 'likes' | 'lyrics';

interface Hit {
  kind: 'track' | 'artist' | 'playlist';
  title: string;
  sub: string;
  art: string | null;
  track?: Track;
  id: number;
  /** the first of the results from all of SoundCloud: a line above it */
  split?: boolean;
}


const FILTERS: [Filter, string][] = [
  ['all', T('Всё')],
  ['tracks', T('Треки')],
  ['artists', T('Артисты')],
  ['playlists', T('Плейлисты')],
  ['likes', T('Мои лайки')],
  ['lyrics', T('По тексту')],
];

let open: { close: () => void } | null = null;

/** `pick`: a track picker (add to a shared playlist): only tracks and likes, a click hands the track over and the window stays open. */
export function openPalette(pick?: (t: Track) => void | Promise<void>): void {
  if (open) {
    open.close();
    return;
  }
  document.dispatchEvent(new Event('sc:close-popovers'));
  let filter: Filter = pick ? 'tracks' : 'all';
  const filters = pick ? FILTERS.filter(([k]) => k === 'tracks' || k === 'likes') : FILTERS;
  let hits: Hit[] = [];
  let sel = 0;
  let seq = 0;
  let likes: Track[] | null = null;

  const input = h('input', { type: 'search', class: 'input pal-input', placeholder: pick ? T('Найдите трек для добавления…') : T('Треки, артисты, плейлисты…'), 'aria-label': T('Быстрый поиск') });
  const chips = h('div', { class: 'pal-chips' });
  const list = h('div', { class: 'pal-list', role: 'listbox' });
  const foot = h('div', { class: 'pal-foot muted small', text: pick ? T('Enter или клик - добавить · Esc - закрыть') : T('Enter - играть · Ctrl+Enter - в очередь · Esc - закрыть') });
  // a grip above the field: the window can be moved (the place is remembered; a double click puts it back)
  const grip = h('div', { class: 'pal-grip', title: T('Потяните, чтобы переместить') });
  const box = h('div', { class: 'pal panel', role: 'dialog', 'aria-modal': 'true', 'aria-label': T('Быстрый поиск') }, grip, input, chips, list, foot);
  let shift = { x: 0, y: 0 };
  try {
    const saved = JSON.parse(localStorage.getItem('palette:pos') ?? 'null') as { x: number; y: number } | null;
    if (saved && Math.abs(saved.x) < 3000 && Math.abs(saved.y) < 3000) shift = saved;
  } catch {
    /* no saved place */
  }
  const place = () => {
    box.style.transform = `translate(${shift.x}px, ${shift.y}px)`;
  };
  place();
  grip.addEventListener('pointerdown', (e) => {
    e.preventDefault();
    grip.setPointerCapture(e.pointerId);
    const from = { x: e.clientX - shift.x, y: e.clientY - shift.y };
    const move = (m: PointerEvent) => {
      // not out of the window: some of the box always stays on the screen
      shift = { x: Math.max(-window.innerWidth * 0.5, Math.min(window.innerWidth * 0.5, m.clientX - from.x)), y: Math.max(-40, Math.min(window.innerHeight - 120, m.clientY - from.y)) };
      place();
    };
    const up = () => {
      grip.removeEventListener('pointermove', move);
      grip.removeEventListener('pointerup', up);
      try {
        localStorage.setItem('palette:pos', JSON.stringify(shift));
      } catch {
        /* not kept */
      }
    };
    grip.addEventListener('pointermove', move);
    grip.addEventListener('pointerup', up);
  });
  grip.addEventListener('dblclick', () => {
    shift = { x: 0, y: 0 };
    place();
    try {
      localStorage.removeItem('palette:pos');
    } catch {
      /* nothing to remove */
    }
  });
  const backdrop = h('div', { class: 'upd-backdrop pal-backdrop' }, box);

  // Esc closes it from anywhere, not only while the focus is inside the box
  const onEsc = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      close();
    }
  };
  const close = () => {
    document.removeEventListener('keydown', onEsc, true);
    backdrop.remove();
    open = null;
  };
  open = { close };
  document.addEventListener('keydown', onEsc, true);

  const renderChips = () => {
    chips.replaceChildren(
      ...filters.map(([k, label]) => {
        const b = h('button', { type: 'button', class: 'pal-chip' + (k === filter ? ' is-active' : ''), text: label });
        b.addEventListener('click', () => {
          filter = k;
          renderChips();
          void run();
          input.focus();
        });
        return b;
      }),
    );
  };

  const trackHit = (t: Track): Hit => ({
    kind: 'track',
    title: t.title,
    sub: `${t.artist} · ${fmtTime(t.duration_ms)}${store.liked.has(t.id) ? ' · ' + T('в лайках') : ''}`,
    art: t.artwork_url,
    track: t,
    id: t.id,
  });
  const userHit = (u: User): Hit => ({ kind: 'artist', title: u.username, sub: T('Артист'), art: u.avatar_url, id: u.id });
  const plHit = (p: Playlist): Hit => ({ kind: 'playlist', title: p.title, sub: `${p.artist} · ${p.is_album ? T('Альбом') : T('Плейлист')}`, art: p.artwork_url, id: p.id });

  const render = () => {
    sel = Math.min(sel, Math.max(0, hits.length - 1));
    list.replaceChildren(
      ...hits.map((x, i) => {
        const img = h('img', { class: 'pal-art' + (x.kind === 'artist' ? ' is-round' : ''), alt: '' });
        const src = coverUrl(x.art, 't67x67');
        if (src) img.src = src;
        const row = h('div', { class: 'pal-row' + (i === sel ? ' is-sel' : '') + (x.split ? ' pal-split' : ''), role: 'option' }, img, h('div', { class: 'pal-text' }, h('div', { class: 'pal-title', dir: 'auto', text: x.title }), h('div', { class: 'muted small', dir: 'auto', text: x.sub })));
        row.addEventListener('click', () => go(x, false));
        row.addEventListener('mousemove', () => {
          if (sel !== i) {
            sel = i;
            mark();
          }
        });
        return row;
      }),
    );
    if (!hits.length && input.value.trim()) {
      list.replaceChildren(h('div', { class: 'pal-empty muted', text: filter === 'lyrics' ? T('Ничего не нашлось ни в текстах лайков, ни среди песен SoundCloud. Тексты лайков индексируются в Настройках.') : T('Ничего не нашлось') }));
    }
  };
  const mark = () => {
    [...list.children].forEach((c, i) => c.classList.toggle('is-sel', i === sel));
    (list.children[sel] as HTMLElement | undefined)?.scrollIntoView({ block: 'nearest' });
  };

  const go = (x: Hit, queue: boolean) => {
    if (pick) {
      if (x.kind === 'track' && x.track) void Promise.resolve(pick(x.track));
      return;
    }
    if (x.kind === 'track' && x.track) {
      // in a room the song goes to the room's queue: a wave from it would take the music from the others
      if (room.active()) {
        void addToRoom(x.track);
        return;
      }
      if (queue) {
        void api.enqueue(x.track, false);
        toast(T('Добавлено в очередь'));
        return;
      }
      if (filter === 'likes' || filter === 'lyrics') {
        const list = hits.filter((y) => y.track);
        void api.playTracks(list.map((y) => y.track!), Math.max(0, list.indexOf(x)));
      } else {
        // a lone track would end the music: carry on with a wave from it
        const t = x.track;
        void api.waveStartFrom(t.id, null).catch(() => api.playTracks([t], 0).catch((e) => toast(errorMessage(e), 'error')));
      }
    } else if (x.kind === 'artist') router.go({ name: 'artist', id: x.id });
    else router.go({ name: 'playlist', id: x.id });
    close();
  };

  const run = async () => {
    const q = input.value.trim();
    const my = ++seq;
    // the likes show at once, before anything is typed
    if (!q && filter !== 'likes') {
      hits = [];
      render();
      return;
    }
    try {
      let next: Hit[] = [];
      if (filter === 'lyrics') {
        // a line of a song you remember: the liked tracks whose lyrics are known come first...
        const mine = (await api.likesSearchLyrics(q)).map((x) => ({ ...trackHit(x.track), sub: `«${x.line}»` }));
        if (my !== seq) return;
        hits = mine;
        sel = 0;
        render();
        // ...and below them the songs of all SoundCloud with these words in the lyrics
        try {
          const own = new Set(mine.map((x) => x.id));
          const all = (await api.lyricsSearchGlobal(q)).filter((t) => !own.has(t.id)).map((t, i) => ({ ...trackHit(t), sub: `SoundCloud · ${t.artist}`, split: i === 0 && mine.length > 0 }));
          if (my !== seq) return;
          next = [...mine, ...all];
        } catch {
          return; // the liked ones stay; no network search this time
        }
      } else if (filter === 'likes') {
        likes ??= await api.likesPage(0, 5000);
        const w = q.toLowerCase();
        next = (w ? likes.filter((t) => `${t.title} ${t.artist}`.toLowerCase().includes(w)) : likes).slice(0, 50).map(trackHit);
      } else if (filter === 'all') {
        const r = await api.searchAll(q);
        next = [...r.tracks.slice(0, 6).map(trackHit), ...r.users.slice(0, 3).map(userHit), ...r.playlists.slice(0, 3).map(plHit)];
      } else if (filter === 'tracks') next = (await api.searchTracks(q, 0)).tracks.slice(0, 20).map(trackHit);
      else if (filter === 'artists') next = (await api.searchUsers(q)).slice(0, 20).map(userHit);
      else next = (await api.searchPlaylists(q)).slice(0, 20).map(plHit);
      if (my !== seq) return;
      hits = next;
      sel = 0;
      render();
    } catch (e) {
      if (my === seq) toast(errorMessage(e), 'error');
    }
  };

  let timer = 0;
  input.addEventListener('input', () => {
    window.clearTimeout(timer);
    timer = window.setTimeout(() => void run(), 250);
  });
  box.addEventListener('keydown', (e) => {
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      if (hits.length) sel = (sel + (e.key === 'ArrowDown' ? 1 : -1) + hits.length) % hits.length;
      mark();
    } else if (e.key === 'Enter') {
      e.preventDefault();
      window.clearTimeout(timer);
      const x = hits[sel];
      if (x) go(x, e.ctrlKey);
      else void run();
    }
  });
  backdrop.addEventListener('pointerdown', (e) => e.target === backdrop && close());
  renderChips();
  document.body.append(backdrop);
  input.focus();
}
