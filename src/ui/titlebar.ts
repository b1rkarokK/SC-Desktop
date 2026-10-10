// Own title bar (window is undecorated): logo + name on the left, drag region
// in the middle, minimize / maximize / close on the right.
// "Close" hides to the tray (handled in Rust on CloseRequested).
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Copy, Minus, Square, X, type IconNode } from 'lucide';
import { h } from './dom';
import { icon, scLogo, setIcon } from './icons';
import { T } from './i18n';

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

export class TitleBar {
  constructor(host: HTMLElement) {
    host.append(
      h('div', { class: 'tb-brand', 'data-tauri-drag-region': true }, scLogo(24), h('span', { class: 'tb-name', 'data-tauri-drag-region': true, text: 'SC Desk' })),
      h('div', { class: 'tb-drag', 'data-tauri-drag-region': true }),
      windowControls(),
    );
  }
}
