// Drag-to-reorder for a list: rows are dragged by their `.drag-handle`, the
// row follows the pointer and the others make room; the list scrolls by
// itself near its edges. `onMove(from, to)` gets row indices among the
// sortable rows once the row is dropped somewhere new.
// Pointer events (mouse, pen, touch); no HTML5 drag-and-drop.

export interface SortableOptions {
  /** rows that take part (others, like a "now playing" row, stay put) */
  rowSelector: string;
  onMove: (from: number, to: number) => void;
}

export function makeSortable(list: HTMLElement, opts: SortableOptions): void {
  list.addEventListener('pointerdown', (e) => {
    const handle = (e.target as HTMLElement).closest('.drag-handle');
    if (!handle || e.button !== 0) return;
    const row = handle.closest(opts.rowSelector) as HTMLElement | null;
    if (!row || !list.contains(row)) return;
    e.preventDefault();
    drag(list, row, e, opts);
  });
}

function rows(list: HTMLElement, sel: string): HTMLElement[] {
  return [...list.querySelectorAll<HTMLElement>(sel)];
}

function drag(list: HTMLElement, row: HTMLElement, start: PointerEvent, opts: SortableOptions): void {
  const scroller = scrollParent(list);
  const from = rows(list, opts.rowSelector).indexOf(row);
  const height = row.getBoundingClientRect().height;
  let lastY = start.clientY;
  let scrollTimer = 0;

  // the row lifts and follows the pointer; a gap keeps its place in the list
  const gap = document.createElement('div');
  gap.className = 'drag-gap';
  gap.style.height = `${height}px`;
  const r0 = row.getBoundingClientRect();
  const dy0 = start.clientY - r0.top;
  row.classList.add('is-row-dragging');
  row.style.width = `${r0.width}px`;
  row.style.left = `${r0.left}px`;
  row.style.top = `${r0.top}px`;
  row.after(gap);
  document.body.append(row);
  document.body.classList.add('is-sorting');

  const place = (y: number) => {
    row.style.top = `${y - dy0}px`;
    // the gap goes before the first row whose middle is below the pointer
    const others = rows(list, opts.rowSelector);
    const before = others.find((r) => {
      const b = r.getBoundingClientRect();
      return y < b.top + b.height / 2;
    });
    if (before) {
      if (gap.nextElementSibling !== before) before.before(gap);
    } else {
      const last = others[others.length - 1];
      if (last && gap.previousElementSibling !== last) last.after(gap);
    }
  };

  const autoScroll = () => {
    if (!scroller) return;
    const b = scroller.getBoundingClientRect();
    const edge = 40;
    const speed = lastY < b.top + edge ? -(b.top + edge - lastY) : lastY > b.bottom - edge ? lastY - (b.bottom - edge) : 0;
    if (speed) {
      scroller.scrollTop += speed / 3;
      place(lastY);
    }
    scrollTimer = requestAnimationFrame(autoScroll);
  };
  scrollTimer = requestAnimationFrame(autoScroll);

  const move = (e: PointerEvent) => {
    lastY = e.clientY;
    place(e.clientY);
  };
  const end = () => {
    cancelAnimationFrame(scrollTimer);
    window.removeEventListener('pointermove', move);
    window.removeEventListener('pointerup', end);
    window.removeEventListener('pointercancel', end);
    document.body.classList.remove('is-sorting');
    row.classList.remove('is-row-dragging');
    row.style.width = row.style.left = row.style.top = '';
    gap.replaceWith(row);
    const to = rows(list, opts.rowSelector).indexOf(row);
    if (from >= 0 && to >= 0 && from !== to) opts.onMove(from, to);
  };
  window.addEventListener('pointermove', move);
  window.addEventListener('pointerup', end);
  window.addEventListener('pointercancel', end);
}

function scrollParent(el: HTMLElement): HTMLElement | null {
  for (let p: HTMLElement | null = el; p; p = p.parentElement) {
    const o = getComputedStyle(p).overflowY;
    if ((o === 'auto' || o === 'scroll') && p.scrollHeight > p.clientHeight) return p;
  }
  return null;
}
