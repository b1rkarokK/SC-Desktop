// «Общие плейлисты» (variant A): one list of access modes (closed / by link / by invitation /
// open), members with roles, every track shows who added it. Edits made while someone is
// offline reach them later through the relay.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { api, errorMessage, type Track, type User } from '../api';
import { h, toast } from '../dom';
import { T } from '../i18n';
import { icon, I } from '../icons';
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
}

interface SharedTrack {
  entry: string;
  track_id: number;
  track: Track | null;
  added_by: number;
  added_by_name: string;
}

interface SharedDetail {
  info: SharedInfo;
  tracks: SharedTrack[];
  link: string;
}

const MODES: [Access, string, string][] = [
  ['closed', T('Закрытый'), T('Видят и меняют только приглашённые по имени')],
  ['link', T('По ссылке'), T('Смотреть может любой со ссылкой, менять - только приглашённые')],
  ['invite', T('По приглашению'), T('Ссылка сама по себе ничего не открывает, нужно приглашение')],
  ['open', T('Открытый'), T('Любой со ссылкой смотрит и добавляет треки')],
];

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
    this.el = h('section', { class: 'view' }, viewHead(T('Общие плейлисты')), this.body);
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
    const link = h('input', { class: 'input', placeholder: T('Ссылка на общий плейлист'), spellcheck: 'false' });
    const join = btn(T('Присоединиться'), null, true);
    const doJoin = async () => {
      try {
        const channel = await invoke<string>('sp_join', { link: link.value.trim() });
        link.value = '';
        this.open = channel;
        await this.load();
      } catch (e) {
        fail(e);
      }
    };
    join.addEventListener('click', () => void doJoin());
    link.addEventListener('keydown', (e) => e.key === 'Enter' && join.click());

    const title = h('input', { class: 'input', placeholder: T('Название'), maxlength: '100' });
    const create = btn(T('Создать'), I.plus);
    create.addEventListener('click', async () => {
      try {
        const r = await invoke<{ info: SharedInfo; link: string }>('sp_create', { title: title.value, access: 'link' });
        await navigator.clipboard.writeText(r.link).catch(() => undefined);
        toast(T('Плейлист создан, ссылка скопирована'));
        title.value = '';
        this.open = r.info.channel;
        await this.load();
      } catch (e) {
        fail(e);
      }
    });

    const rows = h('div', { class: 'rm-list' });
    if (!this.list.length) rows.append(h('p', { class: 'muted small', text: T('Пока нет общих плейлистов.') }));
    for (const p of this.list) {
      const row = h(
        'div',
        { class: 'rm-live', role: 'button', tabindex: '0' },
        h('div', { class: 'rm-live-text' }, h('div', { class: 'strong', dir: 'auto', text: p.title || T('Без названия') }), h('div', { class: 'muted small', text: `${T('общий')} · ${T('{0} треков', p.count)} · ${T('{0} человек', p.members.length)}` })),
        p.unseen ? h('span', { class: 'rm-tag', text: T('новых: {0}', p.unseen) }) : null,
      );
      row.addEventListener('click', () => {
        this.open = p.channel;
        void this.load();
      });
      rows.append(row);
    }
    return [sectionTitle(T('Присоединиться по ссылке')), h('div', { class: 'rm-form' }, link, join), sectionTitle(T('Мои общие плейлисты')), rows, sectionTitle(T('Новый общий плейлист')), h('div', { class: 'rm-form' }, title, create), h('p', { class: 'muted small', text: T('Все меняют один и тот же список. Если кто-то не в сети, он получит правки позже. Данные шифруются, сервер их не читает.') })];
  }

  // -------------------------------------------------------------------- page

  private page(d: SharedDetail): HTMLElement[] {
    const i = d.info;
    const back = btn(T('К списку'), I.back);
    back.addEventListener('click', () => {
      this.open = null;
      void this.load();
    });
    const head = h('div', { class: 'rm-head' }, h('div', {}, h('div', { class: 'rm-title', dir: 'auto', text: i.title }), h('div', { class: 'muted small', text: `${T('общий')} · ${T('{0} человек', i.members.length)}${i.role ? ' · ' + ROLE_TEXT[i.role] : ''}` })));

    const play = btn(T('Играть'), I.play, true);
    play.addEventListener('click', () => void invoke('sp_play', { channel: i.channel, index: 0, shuffle: false }).catch(fail));
    const copy = btn(T('Скопировать ссылку'), I.link);
    copy.addEventListener('click', async () => {
      await navigator.clipboard.writeText(d.link).catch(() => undefined);
      toast(T('Ссылка скопирована'));
    });
    const actions = h('div', { class: 'row-actions' }, play, copy);
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
    const leave = btn(T('Покинуть'), I.close);
    leave.addEventListener('click', async () => {
      if (!confirm(T('Убрать общий плейлист из списка? Другие участники его сохранят.'))) return;
      await invoke('sp_leave', { channel: i.channel }).catch(fail);
      this.open = null;
      await this.load();
    });
    actions.append(leave);

    const tracks = h('div', { class: 'rm-queue' });
    if (!d.tracks.length) tracks.append(emptyState(i.can_edit ? T('Пусто. Правая кнопка по треку - «Добавить в общий плейлист».') : T('Пока пусто.')));
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

    return [h('div', { class: 'row-actions' }, back), head, actions, sectionTitle(T('Треки ({0})', d.tracks.length)), tracks, sectionTitle(T('Участники')), members];
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

    const people = h('select', { class: 'input' }, h('option', { value: '', text: T('Пригласить из подписок…') }));
    const roleSel = h('select', { class: 'input' }, h('option', { value: 'editor', text: T('редактор') }), h('option', { value: 'viewer', text: T('зритель') }));
    let artists: User[] = [];
    void api.libraryArtists(false).then((a) => {
      artists = a;
      for (const u of a) people.append(h('option', { value: String(u.id), text: u.username }));
    }, () => undefined);
    const invite = btn(T('Пригласить'), I.plus);
    invite.addEventListener('click', async () => {
      const u = artists.find((x) => String(x.id) === people.value);
      if (!u) return;
      await invoke('sp_invite', { channel: i.channel, userId: u.id, name: u.username, role: roleSel.value }).catch(fail);
      toast(T('Приглашён: {0}', u.username));
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
