// Hold the left button on a track row for a moment, then drag it to another place in the list. Works for a
// virtual list too (only the rows near the screen exist): the place is read from the row under the pointer.
// A plain click still plays the track; the click that ends a drag is swallowed.

export interface HoldReorderOptions {
  /** the scrolling element of the list (default: the nearest one that scrolls) */
  scroller?: HTMLElement;
  /** rows carry `data-index` (their place in the whole list) */
  onMove: (from: number, to: number) => void;
  /** whether reordering is allowed right now (a filtered or sorted view, for one, must not) */
  enabled?: () => boolean;
}

const HOLD_MS = 320;
const SLOP_PX = 6;

export function holdReorder(list: HTMLElement, opts: HoldReorderOptions): void {
  list.addEventListener('pointerdown', (e) => {
    if (e.button !== 0 || (opts.enabled && !opts.enabled())) return;
    const row = (e.target as HTMLElement).closest('[data-index]') as HTMLElement | null;
    if (!row || !list.contains(row)) return;
    const from = Number(row.dataset.index);
    if (!Number.isFinite(from)) return;
    const x0 = e.clientX;
    const y0 = e.clientY;
    let started = false;
    const timer = window.setTimeout(() => {
      started = true;
      begin(row, from, e.pointerId, y0);
    }, HOLD_MS);
    const cancel = (ev: PointerEvent) => {
      if (!started && Math.hypot(ev.clientX - x0, ev.clientY - y0) > SLOP_PX) stop();
    };
    const stop = () => {
      window.clearTimeout(timer);
      window.removeEventListener('pointermove', cancel);
      window.removeEventListener('pointerup', stop);
    };
    window.addEventListener('pointermove', cancel);
    window.addEventListener('pointerup', stop);
  });

  function scrollerOf(): HTMLElement {
    if (opts.scroller) return opts.scroller;
    let el: HTMLElement | null = list;
    while (el && !(el.scrollHeight > el.clientHeight && /auto|scroll/.test(getComputedStyle(el).overflowY))) el = el.parentElement;
    return el ?? document.documentElement;
  }

  function begin(row: HTMLElement, from: number, pointerId: number, startY: number): void {
    const scroller = scrollerOf();
    const line = document.createElement('div');
    line.className = 'reorder-line';
    document.body.append(line);
    row.classList.add('is-dragging');
    document.body.classList.add('is-reordering');
    try {
      list.setPointerCapture(pointerId);
    } catch {
      // the pointer already went up
    }
    let to = from;
    let lastY = startY;
    let scrollTimer = 0;

    const place = (y: number) => {
      lastY = y;
      const box = scroller.getBoundingClientRect();
      const x = box.left + box.width / 2;
      const under = document.elementFromPoint(x, Math.min(Math.max(y, box.top + 2), box.bottom - 2));
      const r = under?.closest('[data-index]') as HTMLElement | null;
      if (r && list.contains(r)) {
        const rb = r.getBoundingClientRect();
        const i = Number(r.dataset.index);
        const after = y > rb.top + rb.height / 2;
        to = after ? i + 1 : i;
        line.style.cssText = `left:${rb.left}px;width:${rb.width}px;top:${(after ? rb.bottom : rb.top) - 1}px`;
      }
    };
    // near the top or bottom edge the list scrolls by itself
    const autoscroll = () => {
      const box = scroller.getBoundingClientRect();
      const edge = 48;
      const speed = lastY < box.top + edge ? -(box.top + edge - lastY) : lastY > box.bottom - edge ? lastY - (box.bottom - edge) : 0;
      if (speed) {
        scroller.scrollTop += speed / 3;
        place(lastY);
      }
      scrollTimer = window.requestAnimationFrame(autoscroll);
    };
    const move = (ev: PointerEvent) => place(ev.clientY);
    const up = () => {
      window.cancelAnimationFrame(scrollTimer);
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
      line.remove();
      row.classList.remove('is-dragging');
      document.body.classList.remove('is-reordering');
      // the click that ends the drag must not play the track
      window.addEventListener('click', (c) => c.stopPropagation(), { capture: true, once: true });
      const target = to > from ? to - 1 : to;
      if (target !== from) opts.onMove(from, target);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
    scrollTimer = window.requestAnimationFrame(autoscroll);
  }
}
