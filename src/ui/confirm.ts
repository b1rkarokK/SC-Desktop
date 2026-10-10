// Small yes / no window in the app's own style (the update dialog's look).
import { h } from './dom';
import { T } from './i18n';

export function confirmDialog(title: string, text: string, yes: string): Promise<boolean> {
  return new Promise((done) => {
    const no = h('button', { type: 'button', class: 'btn', text: T('Отмена') });
    const ok = h('button', { type: 'button', class: 'btn btn-primary', text: yes });
    const box = h(
      'div',
      { class: 'upd panel', role: 'alertdialog', 'aria-modal': 'true', 'aria-label': title },
      h('div', { class: 'upd-title', dir: 'auto', text: title }),
      h('div', { class: 'muted small', text }),
      h('div', { class: 'upd-actions' }, no, ok),
    );
    const backdrop = h('div', { class: 'upd-backdrop' }, box);
    const close = (v: boolean) => {
      backdrop.remove();
      done(v);
    };
    no.addEventListener('click', () => close(false));
    ok.addEventListener('click', () => close(true));
    backdrop.addEventListener('pointerdown', (ev) => ev.target === backdrop && close(false));
    box.addEventListener('keydown', (ev) => ev.key === 'Escape' && close(false));
    document.body.append(backdrop);
    no.focus();
  });
}
