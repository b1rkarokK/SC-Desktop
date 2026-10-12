// Own title bar (window is undecorated): logo + name on the left, drag region
// in the middle, minimize / maximize / close on the right.
// "Close" hides to the tray (handled in Rust on CloseRequested).
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Copy, Minus, Square, X, type IconNode } from 'lucide';
import { h } from './dom';
import { icon, scLogo, setIcon } from './icons';
import { T } from './i18n';
import { invoke } from '@tauri-apps/api/core';
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

export class TitleBar {
  constructor(host: HTMLElement) {
    host.append(
      h('div', { class: 'tb-brand', 'data-tauri-drag-region': true }, scLogo(24), h('span', { class: 'tb-name', 'data-tauri-drag-region': true, text: 'SC Desk' })),
      h('div', { class: 'tb-drag', 'data-tauri-drag-region': true }),
      roomPill(),
      windowControls(),
    );
  }
}
