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
  private count = h('span', { class: 'muted small rm-count' });

  constructor() {
    const refresh = h('button', { type: 'button', class: 'icon-btn', title: T('Обновить'), 'aria-label': T('Обновить') }, icon(I.refresh));
    refresh.addEventListener('click', () => void this.loadLive());
    const join = btn(T('Присоединиться'), null);
    join.addEventListener('click', () => void this.joinDialog());
    const create = btn(T('Создать'), I.plus, true);
    create.addEventListener('click', () => void this.createDialog());
    const head = viewHead(T('Комнаты'), refresh, join, create);
    head.querySelector('h1')?.after(this.count);
    this.el = h('section', { class: 'view' }, head, this.body);
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
    this.count.textContent = this.live.length ? `· ${T('открыто у друзей: {0}', this.live.length)}` : '';
    if (this.err) return [h('p', { class: 'muted small', text: this.err })];
    if (!this.live.length) {
      return [
        h('div', { class: 'rm-empty' }, icon(I.users, 28), h('div', { class: 'strong', text: T('Сейчас ни у кого из подписок нет открытой комнаты') }), h('div', { class: 'muted small', text: T('Создайте свою или присоединитесь по ссылке: кнопки справа вверху.') })),
        h('p', { class: 'muted small', text: T('Все слушают один трек одновременно. Если у кого-то трек ещё грузится, остальные ждут. Звук и переписка не передаются, только команды.') }),
      ];
    }
    const rows = h('div', { class: 'rm-list' });
    for (const r of this.live) {
      const go = btn(T('Войти'), null);
      go.addEventListener('click', () => void this.enter(r));
      rows.append(
        h(
          'div',
          { class: 'rm-row' },
          h('span', { class: 'rm-dot is-playing' }),
          h('div', { class: 'rm-live-text' }, h('div', { class: 'strong', dir: 'auto', text: r.name }), h('div', { class: 'muted small', text: `${r.host_name} · ${T('{0} в комнате', r.members)}${r.has_password ? ' · ' + T('пароль') : ''}` })),
          go,
        ),
      );
    }
    return [rows];
  }

  /** Friend's room; a password is asked in a dialog only when it has one. */
  private async enter(r: LiveRoom): Promise<void> {
    let password = '';
    if (r.has_password) {
      const v = await this.dialog(T('Пароль комнаты'), r.name, [{ key: 'password', placeholder: T('Пароль'), type: 'password' }], T('Войти'));
      if (!v) return;
      password = v.password ?? '';
    }
    await this.join(r.channel, password);
  }

  /** A small dialog: text fields and one optional checkbox. null when cancelled. */
  private dialog(title: string, hint: string, fields: { key: string; placeholder: string; type?: string; maxlength?: string }[], okLabel: string, check?: { key: string; label: string }): Promise<Record<string, string> | null> {
    return new Promise((done) => {
      const inputs = fields.map((f) => h('input', { class: 'input', placeholder: f.placeholder, type: f.type ?? 'text', maxlength: f.maxlength ?? '400', spellcheck: 'false', autocomplete: 'off' }));
      const box = h('input', { type: 'checkbox' });
      box.checked = true;
      const cancel = h('button', { type: 'button', class: 'btn', text: T('Отмена') });
      const ok = h('button', { type: 'button', class: 'btn btn-primary', text: okLabel });
      const card = h(
        'div',
        { class: 'upd panel', role: 'dialog', 'aria-modal': 'true', 'aria-label': title },
        h('div', { class: 'upd-title', text: title }),
        hint ? h('div', { class: 'muted small', text: hint }) : null,
        ...inputs,
        check ? h('label', { class: 'rm-check' }, box, h('span', { text: check.label })) : null,
        h('div', { class: 'upd-actions' }, cancel, ok),
      );
      const backdrop = h('div', { class: 'upd-backdrop' }, card);
      const close = (v: Record<string, string> | null) => {
        backdrop.remove();
        done(v);
      };
      const submit = () => {
        const out: Record<string, string> = {};
        fields.forEach((f, i) => (out[f.key] = inputs[i]!.value.trim()));
        if (check) out[check.key] = box.checked ? '1' : '';
        // the first field is the main one
        if (fields[0] && fields[0].key !== 'password' && fields[0].key !== 'name' && !out[fields[0].key]) {
          inputs[0]!.focus();
          return;
        }
        close(out);
      };
      cancel.addEventListener('click', () => close(null));
      ok.addEventListener('click', submit);
      backdrop.addEventListener('pointerdown', (ev) => ev.target === backdrop && close(null));
      for (const i of inputs) {
        i.addEventListener('keydown', (ev) => {
          if (ev.key === 'Enter') submit();
          else if (ev.key === 'Escape') close(null);
        });
      }
      document.body.append(backdrop);
      inputs[0]?.focus();
    });
  }

  private async joinDialog(): Promise<void> {
    const v = await this.dialog(T('Присоединиться'), T('Ссылка или код комнаты, которую вам прислали.'), [{ key: 'link', placeholder: T('Ссылка или код комнаты') }, { key: 'password', placeholder: T('Пароль (если есть)'), type: 'password' }], T('Войти'));
    if (v) await this.join(v.link ?? '', v.password ?? '');
  }

  private async createDialog(): Promise<void> {
    const v = await this.dialog(
      T('Новая комната'),
      T('Все слушают один трек одновременно. Звук и переписка не передаются, только команды.'),
      [{ key: 'name', placeholder: T('Название комнаты'), maxlength: '40' }, { key: 'password', placeholder: T('Пароль (необязательно)'), type: 'password' }],
      T('Создать комнату'),
      { key: 'everyone', label: T('Добавлять треки может каждый') },
    );
    if (!v) return;
    try {
      const r = await invoke<{ link: string }>('room_create', { room: { name: v.name ?? '', password: v.password ?? '', access: 'link', queue_everyone: !!v.everyone } });
      await navigator.clipboard.writeText(r.link).catch(() => undefined);
      toast(T('Комната создана, ссылка скопирована'));
    } catch (e) {
      toast(errorMessage(e), 'error');
    }
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
