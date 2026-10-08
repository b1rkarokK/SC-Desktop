// Context menus for tracks, playlists, artists and lyrics.
import { api, errorMessage, type Playlist, type Track, type User } from './api';
import { clock } from './clock';
import { openMenu, type MenuItem } from './context_menu';
import { toast } from './dom';
import { I } from './icons';
import { openArtist, openPlaylist, openTrack } from './router';
import { store } from './store';

async function copy(text: string, done: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    toast(done);
  } catch (e) {
    toast(errorMessage(e), 'error');
  }
}

function fail(e: unknown): void {
  toast(errorMessage(e), 'error');
}

/** `play`: how "Слушать" starts this track in its list context. */
export function trackMenu(e: MouseEvent, t: Track, play?: () => void): void {
  const liked = store.liked.has(t.id);
  const disliked = store.disliked.has(t.id);
  const isCurrent = store.currentId() === t.id;
  const items: MenuItem[] = [
    { label: 'Слушать', icon: I.play, action: play ?? (() => void api.playTracks([t], 0).catch(fail)) },
    { label: 'Играть следующим', icon: I.queueNext, action: () => void api.enqueue(t, true).then(() => toast('Будет следующим'), fail) },
    { label: 'В конец очереди', icon: I.queueEnd, action: () => void api.enqueue(t, false).then(() => toast('Добавлен в очередь'), fail) },
    'separator',
    { label: liked ? 'Убрать из лайков' : 'Лайкнуть', icon: I.heart, on: liked, action: () => void store.setLiked(t, !liked) },
    { label: disliked ? 'Снова рекомендовать' : 'Не рекомендовать', icon: I.dislike, on: disliked, action: () => void store.setDisliked(t, !disliked) },
    { label: 'Волна по треку', icon: I.wave, action: () => void api.waveStartFrom(t.id, null).then((n) => toast(`Волна: ${n} треков`), fail) },
    'separator',
    { label: 'Перейти к треку', icon: I.track, action: () => openTrack(t.id) },
    { label: 'Перейти к автору', icon: I.user, action: () => openArtist(t.user_id), disabled: !t.user_id },
    'separator',
  ];
  if (t.permalink_url) {
    const url = t.permalink_url;
    items.push({ label: 'Скопировать ссылку', icon: I.link, action: () => void copy(url, 'Ссылка скопирована') });
    if (isCurrent) {
      items.push({
        label: 'Ссылка с текущей секундой',
        icon: I.timecode,
        action: () => {
          const s = Math.floor(clock.now() / 1000);
          void copy(`${url}#t=${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`, 'Ссылка с таймкодом скопирована');
        },
      });
    }
    items.push({ label: 'Открыть на SoundCloud', icon: I.external, action: () => void api.openExternal(url) });
  }
  openMenu(e, items);
}

export function playlistMenu(e: MouseEvent, p: Playlist): void {
  const play = async (shuffle: boolean) => {
    try {
      const page = await api.playlistPage(p.id);
      if (!page.tracks.length) return;
      await api.setShuffle(shuffle);
      await api.playTracks(page.tracks, shuffle ? Math.floor(Math.random() * page.tracks.length) : 0);
      void api.historyPlaylistAdd(p);
    } catch (err) {
      fail(err);
    }
  };
  const items: MenuItem[] = [
    { label: 'Слушать', icon: I.play, action: () => void play(false) },
    { label: 'Перемешать', icon: I.shuffle, action: () => void play(true) },
    { label: p.is_album ? 'Открыть альбом' : 'Открыть плейлист', icon: I.playlist, action: () => openPlaylist(p.id) },
    { label: 'Перейти к автору', icon: I.user, action: () => openArtist(p.user_id), disabled: !p.user_id },
    'separator',
  ];
  if (p.permalink_url) {
    const url = p.permalink_url;
    items.push({ label: 'Скопировать ссылку', icon: I.link, action: () => void copy(url, 'Ссылка скопирована') });
    items.push({ label: 'Открыть на SoundCloud', icon: I.external, action: () => void api.openExternal(url) });
  }
  openMenu(e, items);
}

export function artistMenu(e: MouseEvent, u: User): void {
  const following = store.following.has(u.id);
  const items: MenuItem[] = [
    { label: 'Открыть', icon: I.user, action: () => openArtist(u.id) },
    { label: 'Волна по артисту', icon: I.wave, action: () => void api.waveStartFrom(null, u.id).then((n) => toast(`Волна: ${n} треков`), fail) },
    { label: following ? 'Отписаться' : 'Подписаться', icon: following ? I.following : I.plus, on: following, action: () => void store.setFollow(u, !following) },
    {
      label: 'Не ставить в волну',
      icon: I.banArtist,
      action: () => void api.waveDislikeArtist(u.id, u.username).then(() => toast(`${u.username} исключён из волны`), fail),
    },
    'separator',
  ];
  if (u.permalink_url) {
    const url = u.permalink_url;
    items.push({ label: 'Скопировать ссылку', icon: I.link, action: () => void copy(url, 'Ссылка скопирована') });
    items.push({ label: 'Открыть на SoundCloud', icon: I.external, action: () => void api.openExternal(url) });
  }
  openMenu(e, items);
}

export function lyricsMenu(e: MouseEvent, text: string | null, url: string | null, reload: () => void): void {
  const selection = window.getSelection()?.toString() ?? '';
  const items: MenuItem[] = [];
  if (selection) items.push({ label: 'Скопировать выделенное', icon: I.link, action: () => void copy(selection, 'Скопировано') });
  if (text) items.push({ label: 'Скопировать весь текст', icon: I.lyrics, action: () => void copy(text, 'Текст скопирован') });
  items.push({ label: 'Искать текст заново', icon: I.refresh, action: reload });
  if (url) items.push({ label: 'Открыть на Genius', icon: I.external, action: () => void api.openExternal(url) });
  openMenu(e, items);
}
