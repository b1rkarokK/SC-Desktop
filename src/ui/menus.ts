// Context menus for tracks, playlists, artists and lyrics.
import { api, errorMessage, type Playlist, type Track, type User } from './api';
import { clock } from './clock';
import { openMenu, type MenuItem } from './context_menu';
import { addToPlaylistMenu } from './playlist_picker';
import { toast } from './dom';
import { I } from './icons';
import { openArtist, openPlaylist, openTrack, openTrackArtist } from './router';
import { store } from './store';
import { T } from './i18n';

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
    { label: T('Слушать'), icon: I.play, action: play ?? (() => void api.playTracks([t], 0).catch(fail)) },
    { label: T('Играть следующим'), icon: I.queueNext, action: () => void api.enqueue(t, true).then(() => toast(T('Будет следующим')), fail) },
    { label: T('В конец очереди'), icon: I.queueEnd, action: () => void api.enqueue(t, false).then(() => toast(T('Добавлен в очередь')), fail) },
    { label: T('Добавить в плейлист'), icon: I.playlistAdd, action: () => void addToPlaylistMenu(e, t) },
    'separator',
    { label: liked ? T('Убрать из лайков') : T('Лайкнуть'), icon: I.heart, on: liked, action: () => void store.setLiked(t, !liked) },
    { label: disliked ? T('Снова рекомендовать') : T('Не рекомендовать'), icon: I.dislike, on: disliked, action: () => void store.setDisliked(t, !disliked) },
    {
      label: T('Скачать'),
      icon: I.download,
      action: () => {
        toast(T('Скачиваю…'));
        void api.download(t).then((d) => toast(T('Сохранено: {0}', d.path.split(/[\/]/).pop())), fail);
      },
    },
    { label: T('Волна по треку'), icon: I.wave, action: () => void api.waveStartFrom(t.id, null).then((n) => toast(T('Волна: {0} треков', n)), fail) },
    'separator',
    { label: T('Перейти к треку'), icon: I.track, action: () => openTrack(t.id) },
    { label: T('Перейти к автору'), icon: I.user, action: () => void openTrackArtist(t), disabled: !t.user_id },
    'separator',
  ];
  if (t.permalink_url) {
    const url = t.permalink_url;
    items.push({ label: T('Скопировать ссылку'), icon: I.link, action: () => void copy(url, T('Ссылка скопирована')) });
    if (isCurrent) {
      items.push({
        label: T('Ссылка с текущей секундой'),
        icon: I.timecode,
        action: () => {
          const s = Math.floor(clock.now() / 1000);
          void copy(`${url}#t=${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`, T('Ссылка с таймкодом скопирована'));
        },
      });
    }
    items.push({ label: T('Открыть на SoundCloud'), icon: I.external, action: () => void api.openExternal(url) });
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
    { label: T('Слушать'), icon: I.play, action: () => void play(false) },
    { label: T('Перемешать'), icon: I.shuffle, action: () => void play(true) },
    {
      label: p.is_album ? T('Волна по альбому') : T('Волна по плейлисту'),
      icon: I.wave,
      action: () => void api.waveStartPlaylist(p.id).then((n) => toast(T('Волна: {0} треков', n)), fail),
    },
    { label: p.is_album ? T('Открыть альбом') : T('Открыть плейлист'), icon: I.playlist, action: () => openPlaylist(p.id) },
    { label: T('Перейти к автору'), icon: I.user, action: () => openArtist(p.user_id), disabled: !p.user_id },
    'separator',
  ];
  if (p.permalink_url) {
    const url = p.permalink_url;
    items.push({ label: T('Скопировать ссылку'), icon: I.link, action: () => void copy(url, T('Ссылка скопирована')) });
    items.push({ label: T('Открыть на SoundCloud'), icon: I.external, action: () => void api.openExternal(url) });
  }
  openMenu(e, items);
}

export function artistMenu(e: MouseEvent, u: User): void {
  const following = store.following.has(u.id);
  const items: MenuItem[] = [
    { label: T('Открыть'), icon: I.user, action: () => openArtist(u.id) },
    { label: T('Волна по артисту'), icon: I.wave, action: () => void api.waveStartFrom(null, u.id).then((n) => toast(T('Волна: {0} треков', n)), fail) },
    { label: following ? T('Отписаться') : T('Подписаться'), icon: following ? I.following : I.plus, on: following, action: () => void store.setFollow(u, !following) },
    {
      label: T('Не ставить в волну'),
      icon: I.banArtist,
      action: () => void api.waveDislikeArtist(u.id, u.username).then(() => toast(T('{0} исключён из волны', u.username)), fail),
    },
    'separator',
  ];
  if (u.permalink_url) {
    const url = u.permalink_url;
    items.push({ label: T('Скопировать ссылку'), icon: I.link, action: () => void copy(url, T('Ссылка скопирована')) });
    items.push({ label: T('Открыть на SoundCloud'), icon: I.external, action: () => void api.openExternal(url) });
  }
  openMenu(e, items);
}

export function lyricsMenu(e: MouseEvent, text: string | null, url: string | null, reload: () => void): void {
  const selection = window.getSelection()?.toString() ?? '';
  const items: MenuItem[] = [];
  if (selection) items.push({ label: T('Скопировать выделенное'), icon: I.link, action: () => void copy(selection, T('Скопировано')) });
  if (text) items.push({ label: T('Скопировать весь текст'), icon: I.lyrics, action: () => void copy(text, T('Текст скопирован')) });
  items.push({ label: T('Искать текст заново'), icon: I.refresh, action: reload });
  if (url) items.push({ label: T('Открыть на Genius'), icon: I.external, action: () => void api.openExternal(url) });
  openMenu(e, items);
}
