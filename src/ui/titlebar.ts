// Own title bar (window is undecorated): logo + name on the left, drag region
// in the middle, minimize / maximize / close on the right.
// "Close" hides to the tray (handled in Rust on CloseRequested).
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Copy, Minus, Square, X, type IconNode } from 'lucide';
import { h } from './dom';
import { icon, scLogo, setIcon } from './icons';
import { T } from './i18n';
import { api, errorMessage, type AudioDevices } from './api';
import { I } from './icons';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { onRoom, room } from './views/rooms';
import { router } from './router';
import { toast } from './dom';

/** Minimize / maximize / close; used by the title bar and the full-screen view. */
export function windowControls(): HTMLElement {
  const win = getCurrentWindow();
  const winBtn = (node: IconNode, label: string, cls = '') =>
    h('button', { type: 'button', class: `tb-btn ${cls}`, title: label, 'aria-label': label }, icon(node));
  const min = winBtn(Minus, T('Свернуть'));
  const maxBtn = winBtn(Square, T('Развернуть'));
  const close = winBtn(X, T('Закрыть (в трей)'), 'tb-close');
  min.addEventListener('click', () => void win.minimize());
  maxBtn.addEventListener('click', () => void win.toggleMaximize());
  close.addEventListener('click', () => void win.close());
  const syncMax = async () => {
    const max = await win.isMaximized();
    setIcon(maxBtn, max ? Copy : Square);
    maxBtn.title = max ? T('Восстановить') : T('Развернуть');
  };
  void win.onResized(() => void syncMax());
  void syncMax();
  return h('div', { class: 'tb-controls' }, min, maxBtn, close);
}

/** «Вы в комнате» on every tab; hover for the room's actions. */
function roomPill(): HTMLElement {
  const name = h('span', { class: 'room-pill-name' });
  const card = h('div', { class: 'room-pop-card' });
  const pill = h('div', { class: 'room-pill', tabindex: '0', role: 'button', 'aria-label': T('Вы в комнате') }, h('span', { class: 'room-pill-dot' }), name, h('div', { class: 'room-pop' }, card));
  const render = () => {
    const v = room.view;
    if (!room.active() || !v) return;
    const online = v.members.filter((m) => m.status !== 'connecting' || m.you).length;
    name.textContent = v.name;
    const open = h('button', { type: 'button', class: 'btn', text: T('Открыть комнату') });
    open.addEventListener('click', () => router.go({ name: 'rooms' }));
    const invite = h('button', { type: 'button', class: 'btn', text: T('Пригласить') });
    invite.addEventListener('click', async () => {
      await navigator.clipboard.writeText('scdesk://room/' + v.channel).catch(() => undefined);
      toast(T('Ссылка на комнату скопирована'));
    });
    const leave = h('button', { type: 'button', class: 'btn', text: T('Выйти из комнаты') });
    leave.addEventListener('click', () => void invoke('room_leave').catch(() => undefined));
    card.replaceChildren(
      h('div', { class: 'strong', dir: 'auto', text: v.name }),
      h('div', { class: 'muted small', text: `${T('{0} слушают', online)}${v.current && v.play?.playing ? ' · ' + v.current.title + ' - ' + v.current.artist : ''}` }),
      h('div', { class: 'room-pop-actions' }, open, invite, leave),
    );
  };
  onRoom(render);
  render();
  return pill;
}

/** What kind of device a name sounds like, for the little icon. */
function deviceIcon(name: string): IconNode {
  const n = name.toLowerCase();
  if (/науш|headphone|headset|гарнитур|airpods|buds|bluetooth/.test(n)) return I.headphones;
  if (/hdmi|монитор|monitor|display|tv|телев/.test(n)) return I.monitor;
  return I.speaker;
}

/** Where the sound goes: the device in a capsule next to the window buttons, a list to pick another. */
function deviceButton(): HTMLElement {
  const iconBox = h('span', { class: 'dev-icon' });
  const label = h('span', { class: 'dev-name', text: '' });
  const list = h('div', { class: 'dev-pop panel', role: 'menu' });
  const btn = h('button', { type: 'button', class: 'dev-btn', 'aria-haspopup': 'menu', title: T('Куда идёт звук') }, iconBox, label, icon(I.chevronDown, 12));
  const root = h('div', { class: 'dev-wrap' }, btn, list);
  list.hidden = true;
  let data: AudioDevices | null = null;
  const show = (d: AudioDevices) => {
    data = d;
    // the picked device if it is there, else where the sound goes now
    const now = (d.picked && d.devices.includes(d.picked) ? d.picked : d.current ?? d.system) ?? '';
    label.textContent = now || T('Звук');
    label.title = d.picked ? now : T('Как в системе: {0}', now);
    iconBox.replaceChildren(icon(deviceIcon(now), 15));
    root.hidden = !now && !d.devices.length;
  };
  const refresh = async () => {
    try {
      show(await api.audioDevices());
    } catch {
      /* no device list: the button stays as it was */
    }
  };
  const pick = async (name: string | null) => {
    list.hidden = true;
    try {
      await api.audioDeviceSet(name);
      setTimeout(() => void refresh(), 500);
    } catch (e) {
      toast(errorMessage(e), 'error');
    }
  };
  const row = (text: string, sub: string | null, on: boolean, name: string | null) => {
    const r = h('button', { type: 'button', class: `dev-row${on ? ' is-on' : ''}`, role: 'menuitem' }, h('span', { class: 'dev-check' }, on ? icon(I.check, 14) : null), h('span', { class: 'dev-text' }, h('span', { class: 'dev-title', dir: 'auto', text }), sub ? h('span', { class: 'muted small', dir: 'auto', text: sub }) : null));
    r.addEventListener('click', () => void pick(name));
    return r;
  };
  const draw = () => {
    if (!data) return;
    const d = data;
    list.replaceChildren(
      h('div', { class: 'dev-head', text: T('Вывод звука') }),
      row(T('Как в системе'), d.system ? T('сейчас: {0}', d.system) : null, !d.picked || !d.devices.includes(d.picked), null),
      ...d.devices.map((n) => row(n, null, d.picked === n && d.devices.includes(n), n)),
      h('div', { class: 'dev-note muted small', text: T('Треки в плеере SoundCloud (защищённые) идут на устройство по умолчанию.') }),
    );
  };
  btn.addEventListener('click', () => {
    if (!list.hidden) {
      list.hidden = true;
      return;
    }
    // opens at once with what we know; the fresh list replaces it a moment later
    if (data) draw();
    else list.replaceChildren(h('div', { class: 'dev-head', text: T('Загрузка…') }));
    list.hidden = false;
    void refresh().then(() => !list.hidden && draw());
  });
  document.addEventListener('pointerdown', (e) => {
    if (!list.hidden && !root.contains(e.target as Node)) list.hidden = true;
  }, true);
  document.addEventListener('keydown', (e) => e.key === 'Escape' && (list.hidden = true));
  void listen('audio:device', () => void refresh());
  void refresh();
  return root;
}

export class TitleBar {
  constructor(host: HTMLElement) {
    host.append(
      h('div', { class: 'tb-brand', 'data-tauri-drag-region': true }, scLogo(24), h('span', { class: 'tb-name', 'data-tauri-drag-region': true, text: 'SC Desk' })),
      h('div', { class: 'tb-drag', 'data-tauri-drag-region': true }),
      roomPill(),
      deviceButton(),
      h('div', { class: 'tb-sep' }),
      windowControls(),
    );
  }
}
