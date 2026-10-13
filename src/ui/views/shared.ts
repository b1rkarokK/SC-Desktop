// «Общие плейлисты» (variant A): one list of access modes (closed / by link / by invitation /
// open), members with roles, every track shows who added it. Edits made while someone is
// offline reach them later through the relay.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { api, errorMessage, type Track } from '../api';
import { h, toast } from '../dom';
import { T } from '../i18n';
import { icon, I } from '../icons';
import { openPalette } from '../palette';
import { router } from '../router';
import { btn, emptyState, sectionTitle, viewHead, type View } from './common';

type Access = 'closed' | 'link' | 'invite' | 'open';
type Role = 'owner' | 'editor' | 'viewer';

interface Member {
  id: number;
  name: string;
  role: Role;
}

export interface SharedInfo {
  channel: string;
  title: string;
  role: Role | null;
  access: Access | null;
  count: number;
  unseen: number;
  mine: boolean;
  can_edit: boolean;
  members: Member[];
  cover: { color: string | null; image: string | null; avatar?: string | null };
  artworks: string[];
}

interface SharedTrack {
  entry: string;
  track_id: number;
  track: Track | null;
  added_by: number;
  added_by_name: string;
}

interface SharedEvent {
  removed: boolean;
  who: string;
  title: string;
}

interface SharedDetail {
  info: SharedInfo;
  tracks: SharedTrack[];
  history: SharedEvent[];
  link: string;
}

const MODES: [Access, string, string][] = [
  ['closed', T('Закрытый'), T('Видят и меняют только приглашённые по имени')],
  ['link', T('По ссылке'), T('Смотреть может любой со ссылкой, менять - только приглашённые')],
  ['invite', T('По приглашению'), T('Ссылка сама по себе ничего не открывает, нужно приглашение')],
  ['open', T('Открытый'), T('Любой со ссылкой смотрит и добавляет треки')],
];

const COVER_COLORS = ['#3a2418', '#1f2c3f', '#2a3a24', '#3a2438', '#3a3318', '#18353a', '#3a1f1f', '#2a2a2a'];

/** The top of a tile: the banner, else a colour, else the playlist's own picture, else the artwork of the first tracks. */
export function coverStrip(info: SharedInfo, className: string, artworks: string[] = info.artworks): HTMLElement {
  const strip = h('div', { class: className });
  if (info.cover.image) {
    strip.style.backgroundImage = `url("${info.cover.image}")`;
    strip.classList.add('has-image');
  } else if (info.cover.color) {
    strip.style.background = info.cover.color;
  } else if (info.cover.avatar || artworks.length) {
    strip.classList.add('is-mosaic');
    for (const a of info.cover.avatar ? [info.cover.avatar] : artworks.slice(0, 4)) strip.append(h('div', { class: 'sp-art', style: `background-image:url("${a}")` }));
  }
  return strip;
}

/** The banner's size: 4 : 1. */
const BANNER_W = 1200;

/** A square picture (256 px) squeezed until it is small enough to keep. */
async function pictureToAvatar(file: File): Promise<string> {
  const bitmap = await createImageBitmap(file);
  const canvas = document.createElement('canvas');
  canvas.width = canvas.height = 256;
  const ctx = canvas.getContext('2d');
  if (!ctx) throw new Error('canvas');
  const scale = Math.max(256 / bitmap.width, 256 / bitmap.height);
  ctx.drawImage(bitmap, (256 - bitmap.width * scale) / 2, (256 - bitmap.height * scale) / 2, bitmap.width * scale, bitmap.height * scale);
  for (const q of [0.85, 0.72, 0.6, 0.48, 0.36, 0.25]) {
    const url = canvas.toDataURL('image/jpeg', q);
    if (url.length <= 38_000) return url;
  }
  throw new Error(T('Картинка слишком большая'));
}

/** A picture cut to the strip's shape (4 : 1, at most 1200 px wide) and squeezed until it is small enough to keep. */
async function pictureToCover(file: File): Promise<string> {
  const bitmap = await createImageBitmap(file);
  const w = Math.min(BANNER_W, bitmap.width);
  const h2 = Math.round(w / 4);
  const canvas = document.createElement('canvas');
  canvas.width = w;
  canvas.height = h2;
  const ctx = canvas.getContext('2d');
  if (!ctx) throw new Error('canvas');
  // fill the strip: scale to cover, centred
  const scale = Math.max(w / bitmap.width, h2 / bitmap.height);
  const dw = bitmap.width * scale;
  const dh = bitmap.height * scale;
  ctx.drawImage(bitmap, (w - dw) / 2, (h2 - dh) / 2, dw, dh);
  for (const q of [0.82, 0.7, 0.58, 0.46, 0.34]) {
    const url = canvas.toDataURL('image/jpeg', q);
    if (url.length <= 220_000) return url;
  }
  throw new Error(T('Картинка слишком большая'));
}

const ROLE_TEXT: Record<Role, string> = { owner: T('владелец'), editor: T('редактор'), viewer: T('зритель') };

function fail(e: unknown): void {
  toast(errorMessage(e), 'error');
}

/** Shared playlists you can edit: for «Добавить в общий плейлист». */
export async function editableShared(): Promise<SharedInfo[]> {
  try {
    return (await invoke<SharedInfo[]>('sp_list')).filter((p) => p.can_edit);
  } catch {
    return [];
  }
}

export async function addToShared(channel: string, title: string, t: Track): Promise<void> {
  try {
    await invoke('sp_add', { channel, trackId: t.id });
    toast(T('Добавлен в «{0}»', title));
  } catch (e) {
    fail(e);
  }
}

let wanted: string | null = null;

/** Open a shared playlist in its section (from the home page). */
export function openShared(channel: string): void {
  wanted = channel;
  router.go({ name: 'shared' });
}

/** Tracks added by others since each playlist was last opened: the dot in the navigation. */
export async function unseenTotal(): Promise<number> {
  try {
    return (await invoke<SharedInfo[]>('sp_list')).reduce((n, p) => n + p.unseen, 0);
  } catch {
    return 0;
  }
}

export class SharedView implements View {
  el: HTMLElement;
  private body = h('div', { class: 'view-scroll pad' });
  private open: string | null = null;
  private list: SharedInfo[] = [];
  private detail: SharedDetail | null = null;
  private timer = 0;

  constructor() {
    const join = btn(T('Присоединиться'), I.link);
    join.addEventListener('click', () => void this.joinDialog());
    const create = btn(T('Создать'), I.plus, true);
    create.addEventListener('click', () => void this.createDialog());
    this.el = h('section', { class: 'view' }, viewHead(T('Общие плейлисты'), join, create), this.body);
    void listen<string>('playlist:changed', (e) => {
      if (!this.el.isConnected) return;
      window.clearTimeout(this.timer);
      this.timer = window.setTimeout(() => void this.load(), 150);
      void e;
    });
  }

  show(): void {
    if (wanted) {
      this.open = wanted;
      wanted = null;
    }
    void this.load();
  }

  private async load(): Promise<void> {
    try {
      this.list = await invoke<SharedInfo[]>('sp_list');
      this.detail = this.open ? await invoke<SharedDetail>('sp_get', { channel: this.open }).catch(() => null) : null;
      if (this.open && !this.detail) this.open = null;
    } catch (e) {
      fail(e);
    }
    this.render();
  }

  private render(): void {
    this.body.replaceChildren(...(this.detail ? this.page(this.detail) : this.lobby()));
  }

  // -------------------------------------------------------------------- list

  private lobby(): HTMLElement[] {
    if (!this.list.length) {
      return [emptyState(T('Пока нет общих плейлистов. Создайте свой или присоединитесь по ссылке: кнопки справа вверху.')), h('p', { class: 'muted small', text: T('Все меняют один и тот же список. Если кто-то не в сети, он получит правки позже. Данные шифруются, сервер их не читает.') })];
    }
    const grid = h('div', { class: 'sp-tiles' });
    for (const p of this.list) {
      const tile = h(
        'div',
        { class: 'sp-tile', role: 'button', tabindex: '0' },
        coverStrip(p, 'sp-tile-top'),
        h(
          'div',
          { class: 'sp-tile-body' },
          h('div', { class: 'sp-tile-title', dir: 'auto', text: p.title || T('Без названия') }),
          h('div', { class: 'muted small', text: `${T('{0} треков', p.count)} · ${T('{0} человек', p.members.length)}` }),
          p.unseen ? h('span', { class: 'rm-tag', text: T('новых: {0}', p.unseen) }) : null,
        ),
      );
      const go = () => {
        this.open = p.channel;
        void this.load();
      };
      tile.addEventListener('click', go);
      tile.addEventListener('keydown', (e) => e.key === 'Enter' && go());
      grid.append(tile);
    }
    return [grid];
  }

  /** A small dialog with one text field; null when cancelled. */
  private ask(title: string, hint: string, placeholder: string, okLabel: string, maxlength = '200'): Promise<string | null> {
    return new Promise((done) => {
      const input = h('input', { class: 'input', placeholder, maxlength, spellcheck: 'false' });
      const cancel = h('button', { type: 'button', class: 'btn', text: T('Отмена') });
      const ok = h('button', { type: 'button', class: 'btn btn-primary', text: okLabel });
      const box = h('div', { class: 'upd panel', role: 'dialog', 'aria-modal': 'true', 'aria-label': title }, h('div', { class: 'upd-title', text: title }), h('div', { class: 'muted small', text: hint }), input, h('div', { class: 'upd-actions' }, cancel, ok));
      const backdrop = h('div', { class: 'upd-backdrop' }, box);
      const close = (v: string | null) => {
        backdrop.remove();
        done(v);
      };
      const submit = () => {
        const v = input.value.trim();
        if (v) close(v);
        else input.focus();
      };
      cancel.addEventListener('click', () => close(null));
      ok.addEventListener('click', submit);
      backdrop.addEventListener('pointerdown', (ev) => ev.target === backdrop && close(null));
      input.addEventListener('keydown', (ev) => {
        if (ev.key === 'Enter') submit();
        else if (ev.key === 'Escape') close(null);
      });
      document.body.append(backdrop);
      input.focus();
    });
  }

  private async joinDialog(): Promise<void> {
    const link = await this.ask(T('Присоединиться'), T('Вставьте ссылку на общий плейлист, которую вам прислали.'), T('Ссылка на общий плейлист'), T('Присоединиться'), '400');
    if (!link) return;
    try {
      this.open = await invoke<string>('sp_join', { link });
      await this.load();
    } catch (e) {
      fail(e);
    }
  }

  private async createDialog(): Promise<void> {
    const title = await this.ask(T('Новый общий плейлист'), T('Все меняют один и тот же список. Ссылку для друзей можно скопировать на его странице.'), T('Название'), T('Создать'), '100');
    if (!title) return;
    try {
      const r = await invoke<{ info: SharedInfo; link: string }>('sp_create', { title, access: 'link' });
      await navigator.clipboard.writeText(r.link).catch(() => undefined);
      toast(T('Плейлист создан, ссылка скопирована'));
      this.open = r.info.channel;
      await this.load();
    } catch (e) {
      fail(e);
    }
  }

  // -------------------------------------------------------------------- page

  private page(d: SharedDetail): HTMLElement[] {
    const i = d.info;
    const back = btn(T('К списку'), I.back);
    back.addEventListener('click', () => {
      this.open = null;
      void this.load();
    });
    const people = h('div', { class: 'rm-people' }, ...i.members.slice(0, 5).map((m) => h('span', { class: 'rm-avatar', title: m.name, text: (m.name || '?').slice(0, 1) })));
    // the strip across the top (a picture, a colour or nothing), and under it the header with the tracks' artwork
    const hasStrip = !!(i.cover.image || i.cover.color);
    const strip = hasStrip ? coverStrip(i, 'sp-strip') : null;
    const art = d.tracks.map((t) => t.track?.artwork_url).filter((a): a is string => !!a).slice(0, 4);
    const mosaic = h('div', { class: 'sp-mosaic' });
    if (i.cover.avatar) mosaic.classList.add('is-own'), mosaic.append(h('div', { class: 'sp-art', style: `background-image:url("${i.cover.avatar}")` }));
    else if (art.length) for (const a of art) mosaic.append(h('div', { class: 'sp-art', style: `background-image:url("${a}")` }));
    else mosaic.append(icon(I.playlist, 28));
    const head = h('div', { class: 'rm-head sp-head' }, mosaic, h('div', { class: 'sp-head-text' }, h('div', { class: 'rm-title', dir: 'auto', text: i.title }), h('div', { class: 'muted small', text: `${T('общий')} · ${T('{0} треков', d.tracks.length)} · ${T('{0} человек', i.members.length)}${i.role ? ' · ' + ROLE_TEXT[i.role] : ''}` })), people);

    const play = btn(T('Играть'), I.play, true);
    play.addEventListener('click', () => void invoke('sp_play', { channel: i.channel, index: 0, shuffle: false }).catch(fail));
    const together = btn(T('Играть вместе'), I.users);
    together.addEventListener('click', () => void this.playTogether(d));
    const copy = btn(T('Скопировать ссылку'), I.link);
    copy.addEventListener('click', async () => {
      await navigator.clipboard.writeText(d.link).catch(() => undefined);
      toast(T('Ссылка скопирована'));
    });
    const actions = h('div', { class: 'row-actions' }, play, together, copy);
    if (i.role === 'owner') {
      const access = btn(T('Доступ'), I.users);
      access.addEventListener('click', () => this.accessDialog(d));
      const rename = btn(T('Переименовать'), I.edit);
      rename.addEventListener('click', async () => {
        const v = prompt(T('Название'), i.title);
        if (v && v.trim()) await invoke('sp_title', { channel: i.channel, title: v.trim() }).catch(fail);
      });
      actions.append(access, rename);
    }
    if (i.can_edit) {
      const add = btn(T('Добавить трек'), I.plus);
      add.addEventListener('click', () =>
        openPalette(async (t) => {
          await addToShared(i.channel, i.title, t);
        }),
      );
      play.after(add);
      const look = btn(T('Оформление'), I.edit);
      look.addEventListener('click', () => void this.coverDialog(d));
      actions.append(look);
    }
    const leave = btn(T('Покинуть'), I.close);
    leave.addEventListener('click', async () => {
      if (!confirm(T('Убрать общий плейлист из списка? Другие участники его сохранят.'))) return;
      await invoke('sp_leave', { channel: i.channel }).catch(fail);
      this.open = null;
      await this.load();
    });
    actions.append(leave);

    const tracks = h('div', { class: 'rm-queue' });
    if (!d.tracks.length) tracks.append(emptyState(i.can_edit ? T('Пусто. Нажмите «Добавить трек» или правая кнопка по любому треку - «В общий плейлист».') : T('Пока пусто.')));
    d.tracks.forEach((t, n) => {
      const label = t.track ? `${t.track.title} - ${t.track.artist}` : T('Трек {0} (загружается)', t.track_id);
      const row = h('div', { class: 'rm-q' }, h('span', { class: 'muted small', text: String(n + 1) }), h('span', { dir: 'auto', style: 'flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap', text: label }), h('span', { class: 'muted small', dir: 'auto', text: t.added_by_name }));
      row.style.cursor = 'pointer';
      row.addEventListener('click', () => void invoke('sp_play', { channel: i.channel, index: n, shuffle: false }).catch(fail));
      if (i.can_edit) {
        const rm = h('button', { type: 'button', class: 'icon-btn', title: T('Убрать из плейлиста'), 'aria-label': T('Убрать из плейлиста') }, icon(I.close));
        rm.addEventListener('click', (e) => {
          e.stopPropagation();
          void invoke('sp_remove', { channel: i.channel, entry: t.entry }).catch(fail);
        });
        row.append(rm);
      }
      tracks.append(row);
    });

    const members = h('div', { class: 'rm-members' });
    for (const m of i.members) members.append(h('div', { class: 'rm-member' }, h('span', { class: 'rm-name', dir: 'auto', text: m.name }), h('span', { class: 'muted small', text: ROLE_TEXT[m.role] })));

    const history = h('div', { class: 'rm-queue' });
    for (const ev of d.history) {
      const who = ev.who || T('Кто-то');
      const what = ev.title || T('трек');
      history.append(h('div', { class: 'rm-q' }, h('span', { dir: 'auto', text: `${who} ${ev.removed ? T('убрал(а)') : T('добавил(а)')} «${what}»` })));
    }
    const out = [h('div', { class: 'row-actions sp-back' }, back), ...(strip ? [strip] : []), head, actions, sectionTitle(T('Треки ({0})', d.tracks.length)), tracks, sectionTitle(T('Участники')), members];
    if (d.history.length) out.push(sectionTitle(T('История')), history);
    return out;
  }

  /** A room with this playlist's tracks in its queue; the invitation link goes to the clipboard. */
  private async playTogether(d: SharedDetail): Promise<void> {
    const tracks = d.tracks.map((t) => t.track).filter((t): t is Track => !!t);
    if (!tracks.length) {
      toast(T('В плейлисте нет треков, которые можно включить'), 'error');
      return;
    }
    try {
      const r = await invoke<{ link: string }>('room_create', { room: { name: d.info.title, password: '', access: 'link', queue_everyone: true } });
      for (const t of tracks) await invoke('room_cmd', { cmd: { cmd: 'enqueue', track: t } });
      await navigator.clipboard.writeText(r.link).catch(() => undefined);
      toast(T('Комната создана, ссылка скопирована'));
      router.go({ name: 'rooms' });
    } catch (e) {
      fail(e);
    }
  }

  /** The look: a picture across the top, a colour, or nothing. */
  private async coverDialog(d: SharedDetail): Promise<void> {
    const i = d.info;
    const save = (color: string | null, image: string | null, avatar: string | null = i.cover.avatar ?? null) => invoke('sp_cover', { channel: i.channel, color, image, avatar }).catch(fail);
    const swatches = h('div', { class: 'sp-swatches' });
    for (const c of COVER_COLORS) {
      const b = h('button', { type: 'button', class: `sp-swatch${i.cover.color === c && !i.cover.image ? ' is-on' : ''}`, title: c, 'aria-label': c });
      b.style.background = c;
      b.addEventListener('click', async () => {
        await save(c, null);
        done();
      });
      swatches.append(b);
    }
    const file = h('input', { type: 'file', accept: 'image/*', hidden: 'hidden' });
    const pick = btn(T('Баннер…'), I.upload);
    const avatarFile = h('input', { type: 'file', accept: 'image/*', hidden: 'hidden' });
    const pickAvatar = btn(T('Аватарка…'), I.upload);
    pickAvatar.addEventListener('click', () => avatarFile.click());
    avatarFile.addEventListener('change', async () => {
      const f = avatarFile.files?.[0];
      if (!f) return;
      try {
        await save(i.cover.color, i.cover.image, await pictureToAvatar(f));
        done();
      } catch (e) {
        fail(e);
      }
    });
    const autoAvatar = btn(T('Аватарка по трекам'), I.playlist);
    autoAvatar.addEventListener('click', async () => {
      await save(i.cover.color, i.cover.image, null);
      done();
    });
    pick.addEventListener('click', () => file.click());
    file.addEventListener('change', async () => {
      const f = file.files?.[0];
      if (!f) return;
      try {
        await save(i.cover.color, await pictureToCover(f), i.cover.avatar ?? null);
        done();
      } catch (e) {
        fail(e);
      }
    });
    const clear = btn(T('Убрать оформление'), I.close);
    clear.addEventListener('click', async () => {
      await save(null, null, null);
      done();
    });
    const close = h('button', { type: 'button', class: 'btn btn-primary', text: T('Готово') });
    const box = h('div', { class: 'upd panel', role: 'dialog', 'aria-modal': 'true', 'aria-label': T('Оформление') }, h('div', { class: 'upd-title', text: T('Оформление') }), h('div', { class: 'muted small', text: T('Баннер - полоса сверху: своя картинка, цвет или ничего. Рекомендуем 1200 × 300 (4 : 1), картинка обрежется по центру. Аватарка - квадрат слева от названия, рекомендуем от 256 × 256; без неё берутся обложки первых треков. Всё видят все участники.') }), swatches, h('div', { class: 'row-actions' }, pick, clear), h('div', { class: 'row-actions' }, pickAvatar, autoAvatar), file, avatarFile, h('div', { class: 'upd-actions' }, close));
    const backdrop = h('div', { class: 'upd-backdrop' }, box);
    const done = () => {
      backdrop.remove();
      void this.load();
    };
    close.addEventListener('click', done);
    backdrop.addEventListener('pointerdown', (e) => e.target === backdrop && done());
    document.body.append(backdrop);
  }

  // ------------------------------------------------------------ access dialog

  private accessDialog(d: SharedDetail): void {
    const i = d.info;
    let access = i.access ?? 'link';
    const modes = h('div', { class: 'sp-modes' });
    const drawModes = () => {
      modes.replaceChildren(
        ...MODES.map(([key, name, hint]) => {
          const r = h('input', { type: 'radio', name: 'sp-access' });
          r.checked = access === key;
          r.addEventListener('change', () => {
            access = key;
            void invoke('sp_access', { channel: i.channel, access: key }).catch(fail);
          });
          return h('label', { class: 'rm-check', style: 'align-items:flex-start' }, r, h('span', {}, h('div', { class: 'strong', text: name }), h('div', { class: 'muted small', text: hint })));
        }),
      );
    };
    drawModes();

    const people = h('select', { class: 'input' }, h('option', { value: '', text: T('Пригласить друга…') }));
    const roleSel = h('select', { class: 'input' }, h('option', { value: 'editor', text: T('редактор') }), h('option', { value: 'viewer', text: T('зритель') }));
    let friends: { id: number; name: string }[] = [];
    void api.friendsList().then((v) => {
      friends = v.friends;
      for (const u of v.friends) people.append(h('option', { value: String(u.id), text: u.name }));
      if (!v.friends.length) people.firstElementChild!.textContent = T('Сначала добавьте друзей (Профиль - Друзья)');
    }, () => undefined);
    const invite = btn(T('Пригласить'), I.plus);
    invite.addEventListener('click', async () => {
      const u = friends.find((x) => String(x.id) === people.value);
      if (!u) return;
      try {
        await invoke('sp_invite', { channel: i.channel, userId: u.id, name: u.name, role: roleSel.value });
        // the friend gets the invitation in the app (with the link that opens the playlist)
        await api.friendInvite(u.id, 'playlist', d.link, i.title);
        toast(T('Приглашён: {0}', u.name));
      } catch (e) {
        fail(e);
      }
    });

    const close = h('button', { type: 'button', class: 'btn btn-primary', text: T('Готово') });
    const box = h('div', { class: 'upd panel', role: 'dialog', 'aria-modal': 'true', 'aria-label': T('Доступ') }, h('div', { class: 'upd-title', text: T('Доступ') }), modes, h('div', { class: 'rm-form' }, people, roleSel, invite), h('div', { class: 'upd-actions' }, close));
    const backdrop = h('div', { class: 'upd-backdrop' }, box);
    const done = () => {
      backdrop.remove();
      document.removeEventListener('keydown', onKey, true);
      void this.load();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') done();
    };
    document.addEventListener('keydown', onKey, true);
    close.addEventListener('click', done);
    backdrop.addEventListener('pointerdown', (e) => e.target === backdrop && done());
    document.body.append(backdrop);
  }
}
