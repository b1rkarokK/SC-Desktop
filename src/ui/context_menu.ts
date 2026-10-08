// Right-click menu: flat panel at the cursor, flips at screen edges,
// closes on Esc / outside click / scroll / resize; arrow keys + Enter work.
import type { IconNode } from 'lucide';
import { h } from './dom';
import { icon } from './icons';

export type MenuItem =
  | { label: string; icon?: IconNode; action: () => void; disabled?: boolean; on?: boolean }
  | 'separator';

let current: HTMLDivElement | null = null;

export function closeMenu(): void {
  current?.remove();
  current = null;
}

export function openMenu(e: MouseEvent, items: MenuItem[]): void {
  e.preventDefault();
  e.stopPropagation();
  closeMenu();
  const visible = items.filter((it, i, all) => it !== 'separator' || (i > 0 && all[i - 1] !== 'separator' && i < all.length - 1));
  const menu = h('div', { class: 'ctx panel', role: 'menu' });
  for (const it of visible) {
    if (it === 'separator') {
      menu.append(h('div', { class: 'ctx-sep', role: 'separator' }));
      continue;
    }
    const b = h(
      'button',
      { type: 'button', class: `ctx-item${it.on ? ' is-on' : ''}`, role: 'menuitem', disabled: it.disabled },
      it.icon ? icon(it.icon) : h('span', { class: 'ctx-noicon' }),
      h('span', { text: it.label }),
    );
    b.addEventListener('click', () => {
      closeMenu();
      it.action();
    });
    menu.append(b);
  }
  document.body.append(menu);
  current = menu;

  // position at the cursor, flipped to stay on screen
  const r = menu.getBoundingClientRect();
  const x = e.clientX + r.width > window.innerWidth - 4 ? Math.max(4, e.clientX - r.width) : e.clientX;
  const y = e.clientY + r.height > window.innerHeight - 4 ? Math.max(4, e.clientY - r.height) : e.clientY;
  menu.style.left = `${x}px`;
  menu.style.top = `${y}px`;
  (menu.querySelector('.ctx-item:not(:disabled)') as HTMLElement | null)?.focus();

  menu.addEventListener('keydown', (ev) => {
    const list = [...menu.querySelectorAll<HTMLButtonElement>('.ctx-item:not(:disabled)')];
    const idx = list.indexOf(document.activeElement as HTMLButtonElement);
    if (ev.key === 'ArrowDown') list[(idx + 1) % list.length]?.focus();
    else if (ev.key === 'ArrowUp') list[(idx - 1 + list.length) % list.length]?.focus();
    else return;
    ev.preventDefault();
  });
}

document.addEventListener('pointerdown', (e) => {
  if (current && !current.contains(e.target as Node)) closeMenu();
}, true);
document.addEventListener('keydown', (e) => e.key === 'Escape' && closeMenu());
window.addEventListener('blur', closeMenu);
window.addEventListener('resize', closeMenu);
document.addEventListener('scroll', closeMenu, true);
