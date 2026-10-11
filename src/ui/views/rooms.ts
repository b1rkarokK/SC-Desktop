// «Комнаты» (variant A): listen together, nobody speaks. Without a room: join by link / code,
// make your own, see the rooms your friends have open. In a room: who is there and how they are
// doing (the dot by the nick), what plays, the queue, invite, host controls.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { api, errorMessage, type Track, type User } from '../api';
import { h, toast } from '../dom';
import { T } from '../i18n';
import { icon, I } from '../icons';
import { store } from '../store';
import { btn, emptyState, sectionTitle, viewHead, type View } from './common';

export interface RoomMember {
  id: string;
  name: string;
  avatar_url: string | null;
  status: 'connecting' | 'loading' | 'ready' | 'playing' | 'problem';
  problem: string | null;
  host: boolean;
  you: boolean;
}

export interface RoomQueueItem {
  track_id: number;
  title: string;
  artist: string;
  added_by: string;
}

export interface RoomViewState {
  active: boolean;
  host: boolean;
  channel: string;
  name: string;
  has_password: boolean;
  members: RoomMember[];
  queue: RoomQueueItem[];
  current: RoomQueueItem | null;
  play: { at: number; pos_ms: number; playing: boolean } | null;
  who_can_queue_all: boolean;
  connected: boolean;
  ended: string | null;
}

interface LiveRoom {
  channel: string;
  name: string;
  has_password: boolean;
  members: number;
  host_name: string;
  host_id: number;
}

const LINK = 'scdesk://room/';

/** The room as the core last reported it; other parts of the UI read this. */
export const room = {
  view: null as RoomViewState | null,
  active(): boolean {
    return !!this.view?.active && !this.view.ended;
  },
};

const listeners = new Set<() => void>();
export function onRoom(cb: () => void): void {
  listeners.add(cb);
}

export function initRoomEvents(): void {
  void listen<RoomViewState>('room:view', (e) => {
    room.view = e.payload;
    listeners.forEach((cb) => cb());
  });
}

/** «Добавить в комнату» from a track menu. */
export async function addToRoom(t: Track): Promise<void> {
  try {
    await invoke('room_cmd', { cmd: { cmd: 'enqueue', track: t } });
    toast(T('Добавлено в очередь комнаты'));
  } catch (e) {
    toast(errorMessage(e), 'error');
  }
}

const STATUS_TEXT: Record<RoomMember['status'], string> = {
  connecting: T('подключается'),
  loading: T('грузится'),
  ready: T('готов'),
  playing: T('слушает'),
  problem: T('проблема с соединением'),
};

function dot(status: RoomMember['status']): HTMLElement {
  return h('span', { class: `rm-dot is-${status}` });
}

export class RoomsView implements View {
  el: HTMLElement;
  private body = h('div', { class: 'view-scroll pad' });
  private live: LiveRoom[] = [];
  private err = '';

  constructor() {
    this.el = h('section', { class: 'view' }, viewHead(T('Комнаты')), this.body);
    onRoom(() => this.isShown() && this.render());
  }

  private isShown(): boolean {
    return this.el.isConnected;
  }

  show(): void {
    void invoke<RoomViewState>('room_view').then((v) => {
      room.view = v;
      this.render();
    });
    void this.loadLive();
  }

  // ------------------------------------------------------------ friends' rooms

  private async loadLive(): Promise<void> {
    try {
      const people: User[] = await api.libraryArtists(false);
      this.live = await invoke<LiveRoom[]>('rooms_live', { hostIds: people.map((u) => u.id) });
      this.err = '';
    } catch (e) {
      this.live = [];
      this.err = errorMessage(e);
    }
    if (!room.active()) this.render();
  }

  // ------------------------------------------------------------------ render

  private render(): void {
    const v = room.view;
    if (v?.active) {
      this.body.replaceChildren(...this.inRoom(v));
    } else {
      this.body.replaceChildren(...this.lobby());
    }
  }

  private lobby(): HTMLElement[] {
    const link = h('input', { class: 'input', placeholder: T('Ссылка или код комнаты'), spellcheck: 'false' });
    const pw = h('input', { class: 'input', type: 'password', placeholder: T('Пароль (если есть)'), autocomplete: 'off' });
    const join = btn(T('Войти'), null, true);
    join.addEventListener('click', () => void this.join(link.value, pw.value));
    link.addEventListener('keydown', (e) => e.key === 'Enter' && join.click());

    const name = h('input', { class: 'input', placeholder: T('Название комнаты'), maxlength: '40' });
    const pass = h('input', { class: 'input', type: 'password', placeholder: T('Пароль (необязательно)'), autocomplete: 'off' });
    const everyone = h('input', { type: 'checkbox' });
    everyone.checked = true;
    const create = btn(T('Создать комнату'), I.plus);
    create.addEventListener('click', async () => {
      try {
        const r = await invoke<{ link: string }>('room_create', { room: { name: name.value, password: pass.value, access: 'link', queue_everyone: everyone.checked } });
        await navigator.clipboard.writeText(r.link).catch(() => undefined);
        toast(T('Комната создана, ссылка скопирована'));
      } catch (e) {
        toast(errorMessage(e), 'error');
      }
    });

    const friends = h('div', { class: 'rm-list' });
    if (this.err) friends.append(h('p', { class: 'muted small', text: this.err }));
    else if (!this.live.length) friends.append(h('p', { class: 'muted small', text: T('Сейчас ни у кого из тех, на кого вы подписаны, нет открытой комнаты.') }));
    for (const r of this.live) {
      const row = h(
        'div',
        { class: 'rm-live' },
        h('div', { class: 'rm-live-text' }, h('div', { class: 'strong', dir: 'auto', text: r.name }), h('div', { class: 'muted small', text: `${r.host_name} · ${T('{0} в комнате', r.members)}${r.has_password ? ' · ' + T('пароль') : ''}` })),
        (() => {
          const b = btn(T('Войти'), null);
          b.addEventListener('click', () => void this.join(r.channel, r.has_password ? (prompt(T('Пароль комнаты')) ?? '') : ''));
          return b;
        })(),
      );
      friends.append(row);
    }
    const refresh = btn(T('Обновить'), I.refresh);
    refresh.addEventListener('click', () => void this.loadLive());

    return [
      sectionTitle(T('Войти в комнату')),
      h('div', { class: 'rm-form' }, link, pw, join),
      sectionTitle(T('Открытые комнаты друзей'), refresh),
      friends,
      sectionTitle(T('Своя комната')),
      h('div', { class: 'rm-form' }, name, pass, h('label', { class: 'rm-check' }, everyone, h('span', { text: T('Добавлять треки может каждый') })), create),
      h('p', { class: 'muted small', text: T('Все слушают один трек одновременно. Если у кого-то трек ещё грузится, остальные ждут. Звук и переписка не передаются, только команды.') }),
    ];
  }

  private async join(link: string, password: string): Promise<void> {
    try {
      await invoke('room_join', { link, password });
    } catch (e) {
      toast(errorMessage(e), 'error');
    }
  }

  private inRoom(v: RoomViewState): HTMLElement[] {
    if (v.ended) {
      const back = btn(T('К списку комнат'), null, true);
      back.addEventListener('click', async () => {
        await invoke('room_leave');
        room.view = null;
        this.show();
      });
      const text =
        v.ended === 'kicked' ? T('Вас убрали из комнаты.') : v.ended === 'no answer' ? T('Комната не ответила: проверьте ссылку и пароль, возможно, хозяин вышел.') : v.ended === 'closed' ? T('Вы вышли из комнаты.') : v.ended;
      return [emptyState(text), h('div', { class: 'row-actions' }, back)];
    }
    const head = h(
      'div',
      { class: 'rm-head' },
      h('div', {}, h('div', { class: 'rm-title', dir: 'auto', text: v.name || T('Комната') }), h('div', { class: 'muted small', text: `${T('{0} в комнате', v.members.length)}${v.connected ? '' : ' · ' + T('нет связи с сервером')}${v.has_password ? ' · ' + T('пароль') : ''}` })),
    );
    const invite = btn(T('Пригласить'), I.link, true);
    invite.addEventListener('click', async () => {
      await navigator.clipboard.writeText(LINK + v.channel).catch(() => undefined);
      toast(T('Ссылка на комнату скопирована'));
    });
    const leave = btn(T('Покинуть комнату'), I.close);
    leave.addEventListener('click', async () => {
      await invoke('room_leave').catch(() => undefined);
      room.view = null;
      this.show();
    });
    const actions = h('div', { class: 'row-actions' }, invite, leave);
    if (v.host) {
      const pp = btn(v.play?.playing ? T('Пауза') : T('Играть'), v.play?.playing ? I.pause : I.play);
      pp.addEventListener('click', () => void invoke('room_cmd', { cmd: { cmd: 'play', playing: !v.play?.playing } }).catch((e) => toast(errorMessage(e), 'error')));
      const next = btn(T('Дальше'), I.next);
      next.addEventListener('click', () => void invoke('room_cmd', { cmd: { cmd: 'next' } }).catch((e) => toast(errorMessage(e), 'error')));
      actions.prepend(pp, next);
    }

    const now = h(
      'div',
      { class: 'rm-now' },
      v.current
        ? h('div', {}, h('div', { class: 'strong', dir: 'auto', text: v.current.title }), h('div', { class: 'muted small', dir: 'auto', text: v.current.artist }))
        : h('div', { class: 'muted', text: T('Пока ничего не играет. Добавьте трек в очередь: правая кнопка по треку - «Добавить в комнату».') }),
      v.current && v.members.some((m) => m.status === 'loading') ? h('span', { class: 'rm-tag', text: T('ждём, пока загрузится у всех') }) : v.play?.playing ? h('span', { class: 'rm-tag', text: T('играет у всех') }) : null,
    );

    const members = h('div', { class: 'rm-members' });
    for (const m of v.members) {
      const row = h(
        'div',
        { class: 'rm-member' },
        dot(m.status),
        h('span', { class: 'rm-name', dir: 'auto', text: m.name + (m.you ? ' (' + T('вы') + ')' : '') + (m.host ? ' · ' + T('хозяин') : '') }),
        h('span', { class: 'muted small', title: m.problem ?? '', text: STATUS_TEXT[m.status] }),
      );
      if (v.host && !m.you) {
        const kick = h('button', { type: 'button', class: 'icon-btn', title: T('Убрать из комнаты'), 'aria-label': T('Убрать из комнаты') }, icon(I.close));
        kick.addEventListener('click', () => void invoke('room_cmd', { cmd: { cmd: 'kick', id: m.id } }).catch((e) => toast(errorMessage(e), 'error')));
        row.append(kick);
      }
      members.append(row);
    }

    const queue = h('div', { class: 'rm-queue' });
    if (!v.queue.length) queue.append(h('p', { class: 'muted small', text: T('Очередь пуста.') }));
    v.queue.forEach((q, i) => queue.append(h('div', { class: 'rm-q' }, h('span', { class: 'muted small', text: String(i + 1) }), h('span', { dir: 'auto', text: `${q.title} - ${q.artist}` }))));

    const settings: HTMLElement[] = [];
    if (v.host) {
      const everyone = h('input', { type: 'checkbox' });
      everyone.checked = v.who_can_queue_all;
      everyone.addEventListener('change', () => void invoke('room_cmd', { cmd: { cmd: 'queue_everyone', on: everyone.checked } }).catch((e) => toast(errorMessage(e), 'error')));
      settings.push(h('label', { class: 'rm-check' }, everyone, h('span', { text: T('Добавлять треки может каждый') })));
    }
    void store;
    return [head, now, actions, sectionTitle(T('Участники')), members, sectionTitle(T('Очередь ({0})', v.queue.length)), queue, ...settings];
  }
}
