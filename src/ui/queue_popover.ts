// «Очередь» (variant A): a window above the player bar like the equalizer.
// Now playing, then what follows in play order: drag by the handle to
// reorder, ✕ to remove, click to play, «Очистить» drops all that follows.
// While open it follows the player; closed, it costs nothing.
import { listen } from '@tauri-apps/api/event';
import { api, coverUrl, errorMessage, type QueueView } from './api';
import { h, toast } from './dom';
import { I, icon, iconButton } from './icons';
import { T } from './i18n';
import { makeSortable } from './sortable';

const LIMIT = 200;

export class QueuePopover {
  readonly el: HTMLDivElement;
  private body = h('div', { class: 'q-body' });
  private count = h('span', { class: 'muted small' });
  private clearBtn = h('button', { type: 'button', class: 'btn btn-quiet', text: T('Очистить') });
  private open = false;
  private view: QueueView | null = null;
  /** the movable rows, in the order shown */
  private next: QueueView['items'] = [];
  private refreshTimer = 0;
  private onToggle?: (open: boolean) => void;

  constructor() {
    const close = iconButton(I.close, T('Закрыть'));
    close.addEventListener('click', () => this.setOpen(false));
    this.clearBtn.addEventListener('click', () => void api.queueClear().catch((e) => toast(errorMessage(e), 'error')));
    this.el = h(
      'div',
      { class: 'q-pop panel', role: 'dialog', 'aria-label': T('Очередь') },
      h('div', { class: 'q-head' }, h('span', { class: 'strong', text: T('Очередь') }), this.count, h('div', { class: 'spacer' }), this.clearBtn, close),
      this.body,
    );
    this.el.hidden = true;
    makeSortable(this.body, {
      rowSelector: '.q-row.is-next',
      onMove: (from, to) => {
        const a = this.next[from]?.at;
        const b = this.next[to]?.at;
        if (a !== undefined && b !== undefined) void api.queueMove(a, b).catch((e) => toast(errorMessage(e), 'error'));
      },
    });
    const later = () => {
      if (!this.open) return;
      window.clearTimeout(this.refreshTimer);
      this.refreshTimer = window.setTimeout(() => void this.refresh(), 150);
    };
    void listen('player:queue', later);
    void listen('player:state', later);
    document.addEventListener('keydown', (e) => e.key === 'Escape' && this.open && this.setOpen(false));
  }

  init(onToggle: (open: boolean) => void): void {
    this.onToggle = onToggle;
  }

  toggle(): void {
    this.setOpen(!this.open);
  }

  setOpen(open: boolean): void {
    if (this.open === open) return;
    this.open = open;
    this.el.hidden = !open;
    this.onToggle?.(open);
    if (open) void this.refresh();
    else this.body.replaceChildren(); // nothing kept while closed
  }

  get isOpen(): boolean {
    return this.open;
  }

  private async refresh(): Promise<void> {
    if (document.body.classList.contains('is-sorting')) return; // not under a dragged row
    try {
      this.view = await api.queueGet(LIMIT);
    } catch (e) {
      toast(errorMessage(e), 'error');
      return;
    }
    if (!this.open) return;
    this.render(this.view);
  }

  private render(v: QueueView): void {
    const now = v.current === null ? undefined : v.items[0];
    const next = v.current === null ? v.items : v.items.slice(1);
    this.next = next;
    this.count.textContent = v.upcoming ? T('далее {0}', v.upcoming) : '';
    this.clearBtn.hidden = !v.upcoming;
    const scroll = this.body.scrollTop;
    const parts: HTMLElement[] = [];
    if (now) {
      parts.push(h('div', { class: 'q-label', text: T('Сейчас') }), this.row(now, false));
    }
    parts.push(h('div', { class: 'q-label', text: next.length ? T('Далее') : '' }));
    if (!next.length) {
      parts.push(h('div', { class: 'empty small', text: v.source === 'wave' ? T('Волна подберёт следующие треки сама.') : T('Дальше ничего нет. Добавьте треки: правой кнопкой → «Играть следующим».') }));
    }
    parts.push(...next.map((it) => this.row(it, true)));
    if (v.upcoming > next.length) parts.push(h('div', { class: 'muted small q-more', text: T('и ещё {0}', v.upcoming - next.length) }));
    this.body.replaceChildren(...parts);
    this.body.scrollTop = scroll;
  }

  private row(it: QueueView['items'][number], movable: boolean): HTMLElement {
    const t = it.track;
    const img = h('img', { class: 'q-cover', alt: '', loading: 'lazy' });
    const src = coverUrl(t.artwork_url, 't67x67');
    if (src) img.src = src;
    const handle = movable ? h('span', { class: 'drag-handle', title: T('Перетащите, чтобы изменить порядок') }, icon(I.grip)) : h('span', { class: 'q-now-mark' }, icon(I.play));
    const meta = h('div', { class: 'q-meta' }, h('div', { class: 'q-title', dir: 'auto', text: t.title }), h('div', { class: 'q-artist', dir: 'auto', text: t.artist }));
    const row = h('div', { class: `q-row${movable ? ' is-next' : ' is-now'}` }, handle, img, meta);
    if (it.recommended) row.append(h('span', { class: 'q-rec', title: T('Рекомендация умного перемешивания') }, icon(I.smartShuffle)));
    if (movable) {
      const rm = iconButton(I.close, T('Убрать из очереди'));
      rm.addEventListener('click', (e) => {
        e.stopPropagation();
        void api.queueRemove(it.at).catch((err) => toast(errorMessage(err), 'error'));
      });
      row.append(rm);
      row.addEventListener('click', (e) => {
        if ((e.target as HTMLElement).closest('.drag-handle')) return;
        void api.queuePlay(it.at).catch((err) => toast(errorMessage(err), 'error'));
      });
    }
    return row;
  }
}
