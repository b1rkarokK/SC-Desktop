// «Профиль» (avatar at the bottom of the nav): the user's own uploads,
// downloaded files and likes waiting to reach SoundCloud.
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { api, coverUrl, errorMessage, fmtCount, plural, type FriendRow, type Listening, type Track, type User } from '../api';
import { h, toast } from '../dom';
import { I, iconButton } from '../icons';
import { findPeople } from '../friends_dialog';
import { openArtist, router, type ProfileTab } from '../router';
import { SegTabs } from '../seg_tabs';
import { store } from '../store';
import { confirmDialog } from '../confirm';
import { trackMenu } from '../menus';
import { uploadTrack } from '../upload_dialog';
import { btn, emptyState, viewHead, type View } from './common';
import { T } from '../i18n';
import { prefs, updatePrefs } from '../prefs';

function row(t: Track, note: string | null, ...actions: HTMLElement[]): HTMLElement {
  return line(t.artwork_url, t.title, t.artist, note, ...actions);
}

function userRow(u: User, note: string): HTMLElement {
  const r = line(u.avatar_url, u.username, T('артист'), note);
  r.querySelector('.row-cover')?.classList.add('is-round');
  return r;
}

function line(cover: string | null | undefined, title: string, sub: string, note: string | null, ...actions: HTMLElement[]): HTMLElement {
  const img = h('img', { class: 'row-cover', alt: '' });
  const src = coverUrl(cover, 't67x67');
  if (src) img.src = src;
  return h(
    'div',
    { class: 'profile-row' },
    img,
    h('div', { class: 'row-main' }, h('div', { class: 'row-title', dir: 'auto', text: title }), h('div', { class: 'row-artist', dir: 'auto', text: sub })),
    note ? h('span', { class: 'muted small', text: note }) : h('span'),
    h('div', { class: 'profile-actions' }, ...actions),
  );
}

export class ProfileView implements View {
  el: HTMLElement;
  private tabs: SegTabs<ProfileTab>;
  private body = h('div', { class: 'view-scroll pad' });
  private action = h('div', { class: 'row-actions' });

  constructor() {
    this.tabs = new SegTabs<ProfileTab>(
      [
        { key: 'mine', label: T('Мои треки') },
        { key: 'friends', label: T('Друзья') },
        { key: 'files', label: T('Мои файлы') },
        { key: 'downloads', label: T('Скачанные') },
        { key: 'queue', label: T('В очереди') },
      ],
      'mine',
      () => void this.render(),
    );
    this.el = h('section', { class: 'view' }, viewHead(T('Профиль'), this.action), h('div', { class: 'tabs-row' }, this.tabs.el), this.body);
    const refresh = () => {
      void this.counts();
      if (this.el.isConnected) void this.render();
    };
    void listen('likes:queue', refresh);
    void listen('friends:changed', refresh);
    void listen('bridge:captcha', refresh);
    void listen('downloads:changed', refresh);
  }

  show(tab?: ProfileTab): void {
    if (tab) this.tabs.select(tab);
    void this.counts();
    void this.render();
  }

  private async counts(): Promise<void> {
    const [d, q, f] = await Promise.all([
      api.downloads().catch(() => []),
      api.likesPending().catch(() => []),
      api.followsPending().catch(() => []),
    ]);
    const n = q.length + f.length;
    this.tabs.setCount('downloads', d.length ? fmtCount(d.length) : '');
    this.tabs.setCount('queue', n ? fmtCount(n) : '');
  }

  private async render(): Promise<void> {
    if (this.tabs.value === 'mine') await this.renderMine();
    else if (this.tabs.value === 'friends') await this.renderFriends();
    else if (this.tabs.value === 'downloads') await this.renderDownloads();
    else if (this.tabs.value === 'files') await this.renderFiles();
    else await this.renderQueue();
  }

  private async renderMine(): Promise<void> {
    const add = btn(T('Загрузить'), I.upload, true);
    add.addEventListener('click', () => void uploadTrack(() => void this.render()));
    this.action.replaceChildren(add);
    let list;
    try {
      list = await api.myTracks();
    } catch (e) {
      if (this.tabs.value === 'mine') this.body.replaceChildren(emptyState(errorMessage(e)));
      return;
    }
    if (this.tabs.value !== 'mine') return;
    this.tabs.setCount('mine', list.length ? fmtCount(list.length) : '');
    if (!list.length) {
      this.body.replaceChildren(emptyState(T('Здесь будут ваши треки на SoundCloud. Нажмите «Загрузить» и выберите файл: mp3, wav, flac и другие.')));
      return;
    }
    const tracks = list.map((m) => m.track);
    this.body.replaceChildren(
      ...list.map((m, i) => {
        const note = [m.private ? T('закрытый') : null, T('{0} {1}', fmtCount(m.plays), plural(m.plays, 'прослушивание', 'прослушивания', 'прослушиваний'))]
          .filter(Boolean)
          .join(' · ');
        // your own tracks can be liked like any other
        const like = iconButton(I.heart, T('Лайк'), 'btn-icon pf-like');
        const paint = () => {
          const on = store.liked.has(m.track.id);
          like.classList.toggle('is-on', on);
          like.title = on ? T('Убрать из лайков') : T('Лайкнуть');
        };
        paint();
        like.addEventListener('click', async (e) => {
          e.stopPropagation();
          await store.setLiked(m.track, !store.liked.has(m.track.id));
          paint();
        });
        const ext = iconButton(I.external, T('Открыть на SoundCloud'));
        ext.addEventListener('click', (e) => {
          e.stopPropagation();
          if (m.track.permalink_url) void api.openExternal(m.track.permalink_url);
        });
        const rm = iconButton(I.trash, T('Удалить с SoundCloud'));
        rm.addEventListener('click', async (e) => {
          e.stopPropagation();
          const ok = await confirmDialog(T('Удалить «{0}» с SoundCloud?', m.track.title), T('Трек пропадёт с сайта вместе с прослушиваниями и комментариями. Отменить это нельзя.'), T('Удалить'));
          if (!ok) return;
          try {
            await api.uploadDelete(m.track.id);
            toast(T('Трек удалён'));
            void this.render();
          } catch (err) {
            toast(errorMessage(err), 'error');
          }
        });
        const r = row(m.track, note, like, ext, rm);
        r.addEventListener('contextmenu', (e) => trackMenu(e, m.track, () => r.click()));
        r.classList.toggle('is-current', store.currentId() === m.track.id);
        r.addEventListener('click', () => void api.playTracks(tracks, i).catch((err) => toast(errorMessage(err), 'error')));
        return r;
      }),
    );
  }

  /** Friends of the app: requests to answer, invitations that arrived, the list, requests we sent. */
  private async renderFriends(): Promise<void> {
    const find = btn(T('Добавить друга'), I.plus, true);
    find.addEventListener('click', () => findPeople(() => void this.render()));
    this.action.replaceChildren(find);
    let v;
    try {
      v = await api.friendsList();
    } catch (e) {
      if (this.tabs.value === 'friends') this.body.replaceChildren(emptyState(errorMessage(e)));
      return;
    }
    if (this.tabs.value !== 'friends') return;
    this.tabs.setCount('friends', v.friends.length ? fmtCount(v.friends.length) : '');
    const fail = (e: unknown) => toast(errorMessage(e), 'error');
    const person = (u: FriendRow, note: string, ...actions: HTMLElement[]) => {
      const r = line(u.avatar_url, u.name, note, null, ...actions);
      r.querySelector('.row-cover')?.classList.add('is-round');
      return r;
    };
    const parts: HTMLElement[] = [];
    const head = (text: string) => h('div', { class: 'rm-sectiontitle', text });
    this.friendRows.clear();

    if (v.incoming.length) {
      // who you follow on SoundCloud is a hint that the request is from the person it says
      const following = new Set((await api.libraryArtists(false).catch(() => [])).map((x) => x.id));
      parts.push(head(T('Запросы в друзья · {0}', v.incoming.length)));
      parts.push(h('p', { class: 'muted small profile-note', text: T('Имя и аватар берутся с SoundCloud, но запрос мог отправить кто угодно. Принимайте тех, кого знаете.') }));
      for (const u of v.incoming) {
        const yes = btn(T('Принять'), I.check, true);
        yes.addEventListener('click', () => void api.friendAccept(u.id).then(() => this.render(), fail));
        const no = btn(T('Отклонить'), null);
        no.addEventListener('click', () => void api.friendDecline(u.id).then(() => this.render(), fail));
        const known = following.has(u.id) ? ' · ' + T('вы подписаны на него') : '';
        parts.push(person(u, (u.rekey ? T('просит добавить заново (новая установка)') : T('хочет дружить')) + known, yes, no));
      }
    }

    if (v.invites.length) {
      parts.push(head(T('Приглашения · {0}', v.invites.length)));
      for (const inv of v.invites) {
        const join = btn(T('Присоединиться'), I.link, true);
        join.addEventListener('click', async () => {
          try {
            if (inv.kind === 'room') {
              await invoke('room_join', { link: inv.link, password: inv.password });
              router.go({ name: 'rooms' });
            } else {
              await invoke('sp_join', { link: inv.link });
              router.go({ name: 'shared' });
            }
            await api.friendInviteDismiss(inv.id);
          } catch (e) {
            fail(e);
          }
        });
        const hide = iconButton(I.close, T('Скрыть'));
        hide.addEventListener('click', () => void api.friendInviteDismiss(inv.id).then(() => this.render(), fail));
        const row = line(null, inv.title || (inv.kind === 'room' ? T('Комната') : T('Общий плейлист')), `${inv.from_name} · ${inv.kind === 'room' ? T('зовёт в комнату') : T('зовёт в общий плейлист')}`, null, join, hide);
        row.querySelector('.row-cover')?.classList.add('is-round');
        parts.push(row);
      }
    }

    parts.push(head(T('Друзья · {0}', v.friends.length)));
    if (!v.friends.length) parts.push(emptyState(T('Друзей пока нет. Нажмите «Добавить друга», найдите человека на SoundCloud и отправьте запрос: когда он согласится, вы сможете звать друг друга в комнаты и общие плейлисты.')));
    for (const u of v.friends) {
      const rm = iconButton(I.close, T('Удалить из друзей'));
      rm.addEventListener('click', async (e) => {
        e.stopPropagation();
        const ok = await confirmDialog(T('Удалить «{0}» из друзей?', u.name), T('Человек узнает об этом, когда откроет программу. Добавить его снова можно в любой момент.'), T('Удалить'));
        if (ok) void api.friendRemove(u.id).then(() => this.render(), fail);
      });
      const open = iconButton(I.user, T('Открыть профиль'));
      open.addEventListener('click', (e) => {
        e.stopPropagation();
        openArtist(u.id);
      });
      const row = person(u, T('друг'), open, rm);
      this.friendRows.set(u.id, row);
      parts.push(row);
    }

    if (v.outgoing.length) {
      parts.push(head(T('Отправленные запросы · {0}', v.outgoing.length)));
      for (const u of v.outgoing) {
        const cancel = btn(T('Отменить'), null);
        cancel.addEventListener('click', () => void api.friendCancel(u.id).then(() => this.render(), fail));
        parts.push(person(u, T('ждёт ответа'), cancel));
      }
    }
    this.body.replaceChildren(...parts);
    void this.paintListening();
    // what the friends listen to is looked up again every half a minute while this tab is open
    if (!this.nowTimer) {
      this.nowTimer = window.setInterval(() => {
        if (this.el.isConnected && this.tabs.value === 'friends') void this.paintListening();
        else {
          window.clearInterval(this.nowTimer);
          this.nowTimer = 0;
        }
      }, 30_000);
    }
  }

  private friendRows = new Map<number, HTMLElement>();
  private nowTimer = 0;

  /** "listening: …" under the name of each friend who shares what plays. */
  private async paintListening(): Promise<void> {
    let list: Listening[] = [];
    try {
      list = await api.friendsListening();
    } catch {
      return; // the relay is not reachable: the rows stay as they are
    }
    for (const [id, row] of this.friendRows) {
      const sub = row.querySelector('.row-artist');
      if (!sub) continue;
      const now = list.find((x) => x.id === id);
      sub.classList.toggle('ok', !!now);
      sub.textContent = now ? T('слушает: {0} - {1}', now.title, now.artist) : T('друг');
    }
  }

  /** «Мои файлы»: the user's own audio kept in the app, played like any track, never uploaded. */
  private async renderFiles(): Promise<void> {
    const add = btn(T('Добавить файлы'), I.plus, true);
    add.addEventListener('click', async () => {
      try {
        const r = await api.localAdd();
        if (r.added) toast(T('Добавлено: {0}', r.added));
        if (r.refused.length) toast(T('Не добавлены: {0}', r.refused.join('; ')), 'error');
        void this.render();
      } catch (e) {
        toast(errorMessage(e), 'error');
      }
    });
    const size = h('span', { class: 'muted small' });
    void api.storageInfo().then((s) => (size.textContent = T('Занимают: {0}', mb(s.local)))).catch(() => {});
    const clear = btn(T('Удалить все'), I.close);
    clear.addEventListener('click', async () => {
      if (!(await confirmDialog(T('Удалить все свои треки?'), T('Все файлы из «Моих файлов» удалятся из программы. Копии, которые есть у вас где-то ещё, останутся.'), T('Удалить все')))) return;
      await api.localClear().catch((e) => toast(errorMessage(e), 'error'));
      void this.render();
    });
    this.action.replaceChildren(size, add, clear);
    const list = await api.localList().catch(() => []);
    if (this.tabs.value !== 'files') return;
    if (!list.length) {
      this.body.replaceChildren(emptyState(T('Здесь ваши треки, которых нет на SoundCloud: добавьте MP3, M4A, Opus, FLAC или WAV. Они хранятся в одном файле-хранилище программы, никуда не загружаются и играют как обычные треки. Каждый файл проверяется: принимается только настоящий звук.')));
      return;
    }
    const tracks = list.map((l) => l.track);
    this.body.replaceChildren(
      ...list.map((l, i) => {
        const rm = iconButton(I.close, T('Удалить из программы'));
        rm.addEventListener('click', async (e) => {
          e.stopPropagation();
          if (!(await confirmDialog(T('Удалить «{0}» из «Моих файлов»?', l.track.title), T('Файл удалится из программы. Если он есть у вас где-то ещё, там он останется.'), T('Удалить')))) return;
          await api.localRemove(l.track.id).catch((err) => toast(errorMessage(err), 'error'));
          void this.render();
        });
        const r = row(l.track, l.origin === 'shared' ? T('{0} · из комнаты', l.kind) : l.kind, rm);
        r.classList.toggle('is-current', store.currentId() === l.track.id);
        r.addEventListener('click', () => void api.playTracks(tracks, i).catch((err) => toast(errorMessage(err), 'error')));
        return r;
      }),
    );
  }

  private async renderDownloads(): Promise<void> {
    const open = btn(T('Открыть папку'), I.folder);
    open.addEventListener('click', () => void api.downloadsOpen().catch((e) => toast(errorMessage(e), 'error')));
    this.action.replaceChildren(open);
    let list;
    let dir = '';
    try {
      [list, dir] = await Promise.all([api.downloads(), api.downloadsDir()]);
    } catch (e) {
      toast(errorMessage(e), 'error');
      return;
    }
    if (this.tabs.value !== 'downloads') return;
    const change = h('button', { type: 'button', class: 'btn btn-quiet', text: T('Изменить') });
    change.addEventListener('click', async () => {
      try {
        await api.downloadsPickDir();
        void this.render();
      } catch (e) {
        toast(errorMessage(e), 'error');
      }
    });
    const folderLine = h(
      'div',
      { class: 'profile-folder' },
      h('span', { class: 'muted small', text: T('Сохранять в:') }),
      h('span', { class: 'small profile-path', title: dir, text: dir }),
      change,
    );
    // «Лайки без интернета»: the liked tracks are kept as files (the ones playing from their own audio, the rest slowly)
    const likesOn = prefs().offlineLikes === true;
    const knob = h('button', { type: 'button', class: `toggle${likesOn ? ' is-on' : ''}`, role: 'switch', 'aria-checked': String(likesOn), 'aria-label': T('Лайки без интернета') }, h('span', { class: 'toggle-knob' }));
    knob.addEventListener('click', () => {
      const v = knob.getAttribute('aria-checked') !== 'true';
      knob.classList.toggle('is-on', v);
      knob.setAttribute('aria-checked', String(v));
      updatePrefs((p) => (p.offlineLikes = v));
      if (v) toast(T('Лайки начнут сохраняться: те, что слушаете, сразу, остальные понемногу в фоне.'));
    });
    const likesLine = h(
      'div',
      { class: 'profile-folder' },
      knob,
      h('span', { class: 'small', text: T('Лайки без интернета') }),
      h('span', { class: 'muted small', text: T('Лайкнутые треки сохраняются в эту папку: играющий сразу, без лишнего трафика, остальные по одному в фоне.') }),
    );
    if (!list.length) {
      this.body.replaceChildren(folderLine, likesLine, emptyState(T('Скачанных треков пока нет. Нажмите правой кнопкой на трек и выберите «Скачать». Скачанные треки играют без интернета.')));
      return;
    }
    const tracks = list.map((d) => d.track);
    this.body.replaceChildren(
      folderLine,
      likesLine,
      ...list.map((d, i) => {
        const show = iconButton(I.folder, T('Показать в папке'));
        show.addEventListener('click', (e) => {
          e.stopPropagation();
          void api.downloadsOpen(d.path).catch((err) => toast(errorMessage(err), 'error'));
        });
        const rm = iconButton(I.close, T('Удалить файл'));
        rm.addEventListener('click', async (e) => {
          e.stopPropagation();
          await api.downloadRemove(d.track.id).catch((err) => toast(errorMessage(err), 'error'));
        });
        const r = row(d.track, d.path.toLowerCase().endsWith('.m4a') ? 'M4A' : 'MP3', show, rm);
        r.classList.toggle('is-current', store.currentId() === d.track.id);
        r.addEventListener('click', () => void api.playTracks(tracks, i).catch((err) => toast(errorMessage(err), 'error')));
        return r;
      }),
    );
  }

  private async renderQueue(): Promise<void> {
    const check = await api.captchaWaiting().catch(() => false);
    const send = btn(check ? T('Пройти проверку') : T('Отправить сейчас'), check ? I.check : I.refresh, check);
    send.addEventListener('click', async () => {
      send.disabled = true;
      try {
        const left = await api.likesPendingFlush();
        toast(left ? T('SoundCloud пока не принял: {0}. Попробую позже сам.', left) : T('Все лайки отправлены'));
      } catch (e) {
        toast(errorMessage(e), 'error');
      } finally {
        send.disabled = false;
      }
    });
    let list, follows;
    try {
      [list, follows] = await Promise.all([api.likesPending(), api.followsPending()]);
    } catch (e) {
      toast(errorMessage(e), 'error');
      return;
    }
    if (this.tabs.value !== 'queue') return;
    const n = list.length + follows.length;
    this.action.replaceChildren(n ? send : h('span'));
    if (!n) {
      this.body.replaceChildren(emptyState(T('Очередь пуста: все лайки и подписки уже на SoundCloud.')));
      return;
    }
    this.body.replaceChildren(
      h('p', {
        class: 'muted small profile-note',
        text: check
          ? T('SoundCloud просит подтвердить, что вы не робот. Нажмите «Пройти проверку»: откроется окно SoundCloud, а после него {0} {1} на сайт. В программе они уже применены.', n, plural(n, 'действие уйдёт', 'действия уйдут', 'действий уйдут'))
          : T('SoundCloud временно не принимает {0} {1}. В программе они уже применены, а на сайт уйдут сами, даже после перезапуска.', n, plural(n, 'действие', 'действия', 'действий')),
      }),
      ...list.map((p) => row(p.track, p.liked ? T('лайк') : T('убрать лайк'))),
      ...follows.map((f) => userRow(f.user, f.follow ? T('подписка') : T('отписка'))),
    );
  }
}

/** Bytes as megabytes for people: "2,4 МБ" (under 0.1 MB: "меньше 0,1 МБ"). */
export function mb(bytes: number): string {
  const v = bytes / 1048576;
  return v < 0.1 ? T('меньше 0,1 МБ') : `${v.toFixed(v < 10 ? 1 : 0).replace('.', ',')} ${T('МБ')}`;
}
