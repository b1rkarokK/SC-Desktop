// «Комнаты»: listen together, nobody speaks. Rooms are kept in «Мои комнаты» (they stay when
// everybody is away; whoever comes first becomes the host). Left: my rooms, friends' open rooms.
// Middle: what plays, the controls, the queue. Right: who is online and what they are doing.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { api, coverUrl, errorMessage, type Track, type User } from '../api';
import { clock } from '../clock';
import { h, toast } from '../dom';
import { T } from '../i18n';
import { icon, I } from '../icons';
import { store } from '../store';
import { btn, emptyState, viewHead, type View } from './common';

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
  host_id: string;
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

interface Known {
  id: string;
  name: string;
}

interface MineRoom {
  channel: string;
  name: string;
  has_password: boolean;
  online: number;
  active: boolean;
  queue_len: number;
  current: string | null;
  known: Known[];
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
    document.body.classList.toggle('in-room', room.active());
    listeners.forEach((cb) => cb());
  });
  // «Добавлено в очередь комнаты», «Музыкой управляет хост» …
  void listen<string>('room:notice', (e) => toast(e.payload));
  void invoke<RoomViewState>('room_view').then((v) => {
    room.view = v;
    document.body.classList.toggle('in-room', room.active());
    listeners.forEach((cb) => cb());
  });
}

/** «В комнату» for a whole list (a playlist page). */
export async function addTracksToRoom(tracks: Track[]): Promise<void> {
  try {
    for (const t of tracks.slice(0, 200)) await invoke('room_cmd', { cmd: { cmd: 'enqueue', track: t } });
    toast(T('Добавлено в очередь комнаты: {0}', Math.min(tracks.length, 200)));
  } catch (e) {
    toast(errorMessage(e), 'error');
  }
}

/** A button that shows only while we are in a room. */
export function roomButton(tracks: () => Track[]): HTMLButtonElement {
  const b = btn(T('В комнату'), I.users);
  b.classList.add('in-room-only');
  b.addEventListener('click', () => void addTracksToRoom(tracks()));
  return b;
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
  loading: T('грузит трек'),
  ready: T('на связи'),
  playing: T('слушает'),
  problem: T('проблема со связью'),
};

/** What a member is doing, in words; for yourself the verb is «вы»-form. */
function statusText(m: RoomMember): string {
  if (!m.you) return STATUS_TEXT[m.status];
  const own: Record<RoomMember['status'], string> = {
    connecting: T('подключаетесь'),
    loading: T('грузите трек'),
    ready: T('на связи'),
    playing: T('слушаете'),
    problem: T('проблема со связью'),
  };
  return own[m.status];
}

const fail = (e: unknown) => toast(errorMessage(e), 'error');
const cmd = (c: Record<string, unknown>) => void invoke('room_cmd', { cmd: c }).catch(fail);

function fmt(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

function initial(name: string): string {
  return (name.trim()[0] ?? '?').toUpperCase();
}

export class RoomsView implements View {
  el: HTMLElement;
  private side = h('aside', { class: 'rm-side' });
  private main = h('div', { class: 'rm-main' });
  private roster = h('aside', { class: 'rm-roster' });
  private live: LiveRoom[] = [];
  private mine: MineRoom[] = [];
  private err = '';
  private timer = 0;
  private bar: HTMLElement | null = null;
  private time: HTMLElement | null = null;

  constructor() {
    this.el = h('section', { class: 'view' }, viewHead(T('Комнаты')), h('div', { class: 'rm-shell' }, this.side, this.main, this.roster));
    onRoom(() => this.isShown() && this.onRoomChange());
    store.on('player', () => this.isShown() && this.tickBar());
    window.setInterval(() => this.isShown() && this.tickBar(), 500);
  }

  private isShown(): boolean {
    return this.el.isConnected;
  }

  show(): void {
    void invoke<RoomViewState>('room_view').then((v) => {
      room.view = v;
      this.renderAll();
    });
    void this.refresh();
    window.clearInterval(this.timer);
    this.timer = window.setInterval(() => (this.isShown() ? void this.refresh() : window.clearInterval(this.timer)), 15_000);
  }

  private onRoomChange(): void {
    this.renderAll();
    // a room we just opened or left changes the list
    void this.loadMine();
  }

  // ------------------------------------------------------------------- data

  private async refresh(): Promise<void> {
    await Promise.all([this.loadMine(), this.loadLive()]);
  }

  private async loadMine(): Promise<void> {
    try {
      this.mine = await invoke<MineRoom[]>('rooms_mine');
    } catch {
      this.mine = [];
    }
    this.renderSide();
    this.renderRoster();
  }

  private async loadLive(): Promise<void> {
    try {
      const people: User[] = await api.libraryArtists(false);
      this.live = await invoke<LiveRoom[]>('rooms_live', { hostIds: people.map((u) => u.id) });
      this.err = '';
    } catch (e) {
      this.live = [];
      this.err = errorMessage(e);
    }
    this.renderSide();
  }

  // ----------------------------------------------------------------- render

  private renderAll(): void {
    this.renderSide();
    this.renderMain();
    this.renderRoster();
  }

  /** Left: my rooms (they stay), friends' rooms right now, join. */
  private renderSide(): void {
    const active = room.active() ? room.view!.channel : '';
    const create = h('button', { type: 'button', class: 'rm-plus', title: T('Создать комнату'), 'aria-label': T('Создать комнату') }, icon(I.plus, 14));
    create.addEventListener('click', () => void this.createDialog());
    const rows = h('div', { class: 'rm-items' });
    if (!this.mine.length) rows.append(h('p', { class: 'muted small rm-hint', text: T('Пока нет своих комнат. Создайте комнату или присоединитесь по ссылке.') }));
    for (const r of this.mine) {
      const live = r.channel === active ? room.view : null;
      const on = r.online > 0 || !!live;
      // «в сети» is just being in the room; the track shows only while it really plays (only known for the room we are in)
      const nowPlaying = !!live?.play?.playing && !!live.current;
      const sub = nowPlaying ? `${live!.current!.title} - ${live!.current!.artist}` : on ? T('в сети · {0}', Math.max(r.online, 1)) : r.queue_len ? T('пусто · очередь {0}', r.queue_len) : T('пусто');
      const forget = h('button', { type: 'button', class: 'rm-forget', title: T('Убрать из списка'), 'aria-label': T('Убрать из списка') }, icon(I.close, 12));
      const item = h(
        'div',
        { class: `rm-item${r.channel === active ? ' is-active' : ''}`, role: 'button', tabindex: '0' },
        h('span', { class: `rm-state${on ? ' is-on' : ''}` }),
        h('div', { class: 'rm-item-text' }, h('div', { class: 'rm-item-name', dir: 'auto', text: r.name || T('Комната') }), h('div', { class: `rm-item-sub${on ? ' is-on' : ''}`, text: sub })),
        forget,
      );
      const go = () => {
        if (r.channel !== active) void invoke('room_enter', { channel: r.channel }).catch(fail);
      };
      item.addEventListener('click', go);
      item.addEventListener('keydown', (e) => e.key === 'Enter' && go());
      forget.addEventListener('click', (e) => {
        e.stopPropagation();
        if (!confirm(T('Убрать комнату «{0}» из списка? Для других она останется.', r.name))) return;
        void invoke('room_forget', { channel: r.channel }).then(() => this.loadMine(), fail);
      });
      rows.append(item);
    }
    const friends = h('div', { class: 'rm-items' });
    for (const r of this.live.filter((l) => !this.mine.some((m) => m.channel === l.channel))) {
      const go = h('button', { type: 'button', class: 'rm-link', text: T('войти') });
      go.addEventListener('click', () => void this.enter(r));
      friends.append(h('div', { class: 'rm-friend' }, h('span', { class: 'rm-pav small', text: initial(r.host_name) }), h('div', { class: 'rm-item-text' }, h('div', { class: 'rm-item-name', dir: 'auto', text: r.name }), h('div', { class: 'rm-item-sub', text: `${r.host_name} · ${T('{0} в комнате', r.members)}` })), go));
    }
    const join = h('button', { type: 'button', class: 'rm-joinbtn' }, icon(I.link, 14), h('span', { text: T('Присоединиться') }));
    join.addEventListener('click', () => void this.joinDialog());
    this.side.replaceChildren(
      h('div', { class: 'rm-sidehead' }, h('span', { text: T('Мои комнаты') }), create),
      rows,
      ...(this.err ? [h('p', { class: 'muted small rm-hint', text: this.err })] : friends.childElementCount ? [h('div', { class: 'rm-sidehead' }, h('span', { text: T('У друзей сейчас') })), friends] : []),
      h('div', { class: 'spacer' }),
      join,
    );
  }

  /** Middle: what plays, controls, queue. */
  private renderMain(): void {
    const v = room.view;
    this.bar = null;
    this.time = null;
    if (!v?.active) {
      this.main.replaceChildren(
        h('div', { class: 'rm-welcome' }, icon(I.users, 30), h('div', { class: 'rm-title', text: T('Слушайте вместе') }), h('div', { class: 'muted', text: T('Все слушают один трек одновременно. Звук и переписка не передаются, только команды. Комната остаётся в списке слева, даже когда в ней никого нет: кто зайдёт первым, тот и хост.') }), h('div', { class: 'row-actions' }, (() => {
          const b = btn(T('Создать комнату'), I.plus, true);
          b.addEventListener('click', () => void this.createDialog());
          return b;
        })(), (() => {
          const b = btn(T('Присоединиться'), I.link);
          b.addEventListener('click', () => void this.joinDialog());
          return b;
        })())),
      );
      return;
    }
    if (v.ended) {
      const back = btn(T('К списку комнат'), null, true);
      back.addEventListener('click', async () => {
        await invoke('room_leave').catch(() => undefined);
        room.view = null;
        this.renderAll();
        void this.loadMine();
      });
      const text = v.ended === 'kicked' ? T('Вас убрали из комнаты.') : v.ended === 'no answer' ? T('Не удалось войти: проверьте ссылку и пароль.') : v.ended === 'closed' ? T('Вы вышли из комнаты.') : v.ended;
      this.main.replaceChildren(emptyState(text), h('div', { class: 'row-actions' }, back));
      return;
    }

    const online = v.members.filter((m) => m.status !== 'connecting' || m.you);
    const host = v.members.find((m) => m.id === v.host_id) ?? v.members.find((m) => m.host);
    const invite = btn(T('Пригласить'), I.link, true);
    invite.addEventListener('click', async () => {
      await navigator.clipboard.writeText(LINK + v.channel).catch(() => undefined);
      toast(T('Ссылка на комнату скопирована'));
    });
    const leave = btn(T('Выйти из комнаты'), null);
    leave.addEventListener('click', async () => {
      await invoke('room_leave').catch(() => undefined);
      room.view = null;
      this.renderAll();
      void this.loadMine();
    });
    const head = h(
      'div',
      { class: 'rm-roomhead' },
      h('div', { class: 'rm-roomtitle' }, h('div', { class: 'rm-title', dir: 'auto', text: v.name || T('Комната') }), h('div', { class: 'muted small', text: `${T('{0} слушают', online.length)}${host ? ' · ' + T('хост {0}', host.name) : ''}${v.connected ? '' : ' · ' + T('нет связи с сервером')}${v.has_password ? ' · ' + T('пароль') : ''}` })),
      invite,
      leave,
    );

    const names = new Map<string, string>();
    for (const m of v.members) names.set(m.id, m.name);
    for (const r of this.mine) if (r.channel === v.channel) for (const k of r.known) if (!names.has(k.id)) names.set(k.id, k.name);

    const cur = v.current;
    const bar = h('div', { class: 'rm-barfill' });
    const time = h('span', { class: 'muted small', text: '0:00' });
    this.bar = bar;
    this.time = time;
    const prev = h('button', { type: 'button', class: 'rm-round', title: T('Назад'), 'aria-label': T('Назад') }, icon(I.prev, 16));
    prev.addEventListener('click', () => cmd({ cmd: 'prev' }));
    const pp = h('button', { type: 'button', class: 'rm-round is-main', title: v.play?.playing ? T('Пауза') : T('Играть'), 'aria-label': T('Играть') }, icon(v.play?.playing ? I.pause : I.play, 16));
    pp.addEventListener('click', () => cmd({ cmd: 'play', playing: !v.play?.playing }));
    const next = h('button', { type: 'button', class: 'rm-round', title: T('Дальше'), 'aria-label': T('Дальше') }, icon(I.next, 16));
    next.addEventListener('click', () => cmd({ cmd: 'next' }));
    const loading = v.members.some((m) => m.status === 'loading');
    const state = cur ? (loading ? T('ждём, пока загрузится у всех') : v.play?.playing ? T('играет у всех · вместе') : T('на паузе')) : '';
    // the cover of what plays: the local player follows the room, so it knows the artwork
    const cover = h('div', { class: 'rm-cover' }, icon(I.queue, 30));
    const art = cur && store.snapshot?.track?.id === cur.track_id ? coverUrl(store.snapshot.track.artwork_url, 't500x500') : null;
    if (art) {
      cover.replaceChildren();
      cover.style.backgroundImage = `url("${art}")`;
      cover.style.backgroundSize = 'cover';
      cover.style.backgroundPosition = 'center';
    }
    const card = h(
      'div',
      { class: 'rm-card' },
      cover,
      h(
        'div',
        { class: 'rm-card-body' },
        cur ? h('div', {}, h('div', { class: 'strong', dir: 'auto', text: cur.title }), h('div', { class: 'muted', dir: 'auto', text: cur.artist })) : h('div', { class: 'muted', text: v.queue.length ? T('Очередь сохранена. Нажмите «Играть», чтобы продолжить.') : T('Пока ничего не играет. Добавьте трек: кнопка рядом с треком или «В конец очереди» в меню трека.') }),
        h('div', { class: 'rm-bar' }, bar),
        h('div', { class: 'rm-timerow' }, time, h('span', { class: `small ${loading ? 'rm-warn' : 'rm-ok'}`, text: state })),
        v.host ? h('div', { class: 'rm-ctl' }, prev, pp, next) : h('div', { class: 'muted small', text: T('Музыкой управляет хост') }),
      ),
    );

    const queue = h('div', { class: 'rm-queue' });
    if (!v.queue.length) queue.append(h('p', { class: 'muted small', text: T('Очередь пуста.') }));
    v.queue.forEach((q, i) => queue.append(h('div', { class: 'rm-qrow' }, h('span', { class: 'muted small rm-qn', text: String(i + 1) }), h('span', { class: 'rm-qt', dir: 'auto', text: `${q.title} - ${q.artist}` }), h('span', { class: 'muted small', dir: 'auto', text: names.get(q.added_by) ?? '' }))));

    const extra: HTMLElement[] = [];
    if (v.host) {
      const everyone = h('input', { type: 'checkbox' });
      everyone.checked = v.who_can_queue_all;
      everyone.addEventListener('change', () => cmd({ cmd: 'queue_everyone', on: everyone.checked }));
      extra.push(h('label', { class: 'rm-check', style: 'margin-top:12px' }, everyone, h('span', { text: T('Добавлять треки может каждый') })));
    }
    this.main.replaceChildren(head, card, h('div', { class: 'rm-sectiontitle', text: T('Дальше в очереди · {0}', v.queue.length) }), queue, ...extra);
    this.tickBar();
  }

  /** Right: who is online and what they are doing; who is away. */
  private renderRoster(): void {
    const v = room.view;
    if (!v?.active || v.ended) {
      this.roster.replaceChildren();
      this.roster.hidden = true;
      return;
    }
    this.roster.hidden = false;
    const online = v.members.filter((m) => m.status !== 'connecting' || m.you);
    // away: known to the room, but not here now (a member still «connecting» is not here either)
    const here = new Set(online.map((m) => m.id));
    const known = new Map<string, string>();
    for (const k of this.mine.find((r) => r.channel === v.channel)?.known ?? []) known.set(k.id, k.name);
    for (const m of v.members) if (!known.has(m.id)) known.set(m.id, m.name);
    const away = [...known].filter(([id]) => !here.has(id)).map(([id, name]) => ({ id, name }));
    const person = (m: RoomMember) => {
      const text = m.host ? `${T('хост')} · ${statusText(m)}` : statusText(m);
      const kick = v.host && !m.you ? h('button', { type: 'button', class: 'rm-forget', title: T('Убрать из комнаты'), 'aria-label': T('Убрать из комнаты') }, icon(I.close, 12)) : null;
      kick?.addEventListener('click', () => cmd({ cmd: 'kick', id: m.id }));
      return h(
        'div',
        { class: 'rm-person' },
        h('span', { class: 'rm-pav' }, initial(m.name), h('span', { class: `rm-pdot is-${m.status}` })),
        h('div', { class: 'rm-item-text' }, h('div', { class: 'rm-item-name', dir: 'auto' }, m.name, m.you ? h('span', { class: 'muted small', text: ' ' + T('вы') }) : null), h('div', { class: `rm-item-sub is-${m.status}${m.host ? ' is-host' : ''}`, title: m.problem ?? '', text }),),
        kick,
      );
    };
    this.roster.replaceChildren(
      h('div', { class: 'rm-sidehead' }, h('span', { text: T('В сети, {0}', online.length) })),
      ...online.map(person),
      ...(away.length ? [h('div', { class: 'rm-sidehead' }, h('span', { text: T('Не в сети, {0}', away.length) })), ...away.map((k) => h('div', { class: 'rm-person is-away' }, h('span', { class: 'rm-pav', text: initial(k.name) }), h('div', { class: 'rm-item-text' }, h('div', { class: 'rm-item-name', dir: 'auto', text: k.name }))))] : []),
      h('p', { class: 'muted small rm-hint', text: T('Хост передаётся сам: когда он выйдет, им станет следующий по списку.') }),
    );
  }

  /** The progress of the track from the local player (it follows the room). */
  private tickBar(): void {
    if (!this.bar || !this.time) return;
    const d = store.snapshot?.track?.duration_ms ?? 0;
    const pos = clock.now();
    this.bar.style.width = d > 0 ? `${Math.min(100, (pos / d) * 100)}%` : '0%';
    this.time.textContent = d > 0 ? `${fmt(pos)} / ${fmt(d)}` : '0:00';
  }

  // ---------------------------------------------------------------- dialogs

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
        // the first field is the main one (a name may stay empty: it gets one)
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
      fail(e);
    }
  }

  private async join(link: string, password: string): Promise<void> {
    try {
      await invoke('room_join', { link, password });
    } catch (e) {
      fail(e);
    }
  }
}
