// Segmented tabs: one bordered block, 1px dividers, active = 6% tint + bold.
import { h } from './dom';

export interface SegTab<K extends string> {
  key: K;
  label: string;
  count?: string;
}

export class SegTabs<K extends string> {
  readonly el: HTMLDivElement;
  private buttons = new Map<K, { btn: HTMLButtonElement; count: HTMLSpanElement }>();

  constructor(tabs: SegTab<K>[], private active: K, private onChange: (k: K) => void) {
    this.el = h('div', { class: 'seg', role: 'tablist' });
    for (const t of tabs) {
      const count = h('span', { class: 'seg-count', text: t.count ?? '' });
      const btn = h('button', { type: 'button', class: 'seg-tab', role: 'tab' }, h('span', { text: t.label }), count);
      btn.addEventListener('click', () => this.select(t.key, true));
      this.buttons.set(t.key, { btn, count });
      this.el.append(btn);
    }
    this.render();
  }

  get value(): K {
    return this.active;
  }

  select(k: K, notify = false): void {
    if (this.active === k && !notify) return;
    const changed = this.active !== k;
    this.active = k;
    this.render();
    if (notify && changed) this.onChange(k);
  }

  setCount(k: K, count: string): void {
    const b = this.buttons.get(k);
    if (b) b.count.textContent = count;
  }

  private render(): void {
    for (const [k, { btn }] of this.buttons) {
      const on = k === this.active;
      btn.classList.toggle('is-active', on);
      btn.setAttribute('aria-selected', String(on));
    }
  }
}
