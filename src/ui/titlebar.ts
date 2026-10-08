// Own title bar (window is undecorated): brand + nav collapse on the left,
// drag region in the middle, minimize / maximize / close on the right.
// "Close" hides to the tray (handled in Rust on CloseRequested).
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Copy, Minus, Square, X, type IconNode } from 'lucide';
import { h } from './dom';
import { icon, I, scLogo, setIcon } from './icons';
import { onPrefs, prefs, updatePrefs } from './prefs';

export class TitleBar {
  private maxBtn: HTMLButtonElement;
  private collapseBtn: HTMLButtonElement;

  constructor(host: HTMLElement) {
    const win = getCurrentWindow();
    const winBtn = (node: IconNode, label: string, cls = '') => {
      const b = h('button', { type: 'button', class: `tb-btn ${cls}`, title: label, 'aria-label': label }, icon(node));
      return b;
    };
    const min = winBtn(Minus, 'Свернуть');
    this.maxBtn = winBtn(Square, 'Развернуть');
    const close = winBtn(X, 'Закрыть (в трей)', 'tb-close');
    min.addEventListener('click', () => void win.minimize());
    this.maxBtn.addEventListener('click', () => void win.toggleMaximize());
    close.addEventListener('click', () => void win.close());

    this.collapseBtn = h('button', { type: 'button', class: 'btn-icon tb-collapse' }, icon(I.navCollapse));
    this.collapseBtn.addEventListener('click', () => updatePrefs((p) => (p.navCollapsed = !p.navCollapsed)));

    host.append(
      h(
        'div',
        { class: 'tb-brand', 'data-tauri-drag-region': true },
        scLogo(24),
        h('span', { class: 'tb-name', 'data-tauri-drag-region': true, text: 'SC Desk' }),
        this.collapseBtn,
      ),
      h('div', { class: 'tb-drag', 'data-tauri-drag-region': true }),
      h('div', { class: 'tb-controls' }, min, this.maxBtn, close),
    );

    const syncMax = async () => {
      const max = await win.isMaximized();
      setIcon(this.maxBtn, max ? Copy : Square);
      this.maxBtn.title = max ? 'Восстановить' : 'Развернуть';
      document.documentElement.classList.toggle('is-maximized', max);
    };
    void win.onResized(() => void syncMax());
    void syncMax();
    onPrefs(() => this.syncCollapse());
    window.addEventListener('resize', () => this.syncCollapse());
    this.syncCollapse();
  }

  private syncCollapse(): void {
    const collapsed = document.getElementById('app')?.classList.contains('nav-collapsed') ?? prefs().navCollapsed;
    setIcon(this.collapseBtn, collapsed ? I.navExpand : I.navCollapse);
    this.collapseBtn.title = collapsed ? 'Развернуть панель' : 'Свернуть панель';
  }
}
