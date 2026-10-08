// Flat slider built from divs (no gradients, no native range styling quirks).
// Horizontal or vertical; vertical can be bipolar (fill grows from the middle, for EQ).

export interface SliderOpts {
  vertical?: boolean;
  bipolar?: boolean;
}

export class Slider {
  readonly el: HTMLDivElement;
  private fill: HTMLDivElement;
  private knob: HTMLDivElement;
  private value = 0;
  private dragging = false;

  /** onInput: while dragging; onCommit: on release / key press. Values 0..1. */
  constructor(
    label: string,
    private onInput: (v: number) => void,
    private onCommit: (v: number) => void,
    private opts: SliderOpts = {},
  ) {
    this.el = document.createElement('div');
    this.el.className = `slider${opts.vertical ? ' slider-v' : ''}`;
    this.el.tabIndex = 0;
    this.el.setAttribute('role', 'slider');
    this.el.setAttribute('aria-label', label);
    this.el.setAttribute('aria-orientation', opts.vertical ? 'vertical' : 'horizontal');
    const track = document.createElement('div');
    track.className = 'slider-track';
    this.fill = document.createElement('div');
    this.fill.className = 'slider-fill';
    this.knob = document.createElement('div');
    this.knob.className = 'slider-knob';
    track.append(this.fill, this.knob);
    this.el.append(track);

    this.el.addEventListener('pointerdown', (e) => {
      if (e.button !== 0) return;
      this.dragging = true;
      this.el.setPointerCapture(e.pointerId);
      this.el.classList.add('is-dragging');
      this.fromPointer(e);
    });
    this.el.addEventListener('pointermove', (e) => {
      if (this.dragging) this.fromPointer(e);
    });
    const end = (e: PointerEvent) => {
      if (!this.dragging) return;
      this.dragging = false;
      this.el.classList.remove('is-dragging');
      this.el.releasePointerCapture(e.pointerId);
      this.onCommit(this.value);
    };
    this.el.addEventListener('pointerup', end);
    this.el.addEventListener('pointercancel', end);
    this.el.addEventListener('dblclick', () => {
      if (this.opts.bipolar) {
        this.render(0.5);
        this.onCommit(0.5);
      }
    });
    this.el.addEventListener('keydown', (e) => {
      const step = e.shiftKey ? 0.1 : 0.02;
      if (e.key === 'ArrowRight' || e.key === 'ArrowUp') this.nudge(step, e);
      else if (e.key === 'ArrowLeft' || e.key === 'ArrowDown') this.nudge(-step, e);
    });
  }

  get isDragging(): boolean {
    return this.dragging;
  }

  /** External update; ignored while the user is dragging. */
  set(v: number): void {
    if (this.dragging) return;
    this.render(v);
  }

  private nudge(delta: number, e: KeyboardEvent): void {
    e.preventDefault();
    e.stopPropagation();
    this.render(this.value + delta);
    this.onCommit(this.value);
  }

  private fromPointer(e: PointerEvent): void {
    const r = this.el.getBoundingClientRect();
    const v = this.opts.vertical ? 1 - (e.clientY - r.top) / Math.max(1, r.height) : (e.clientX - r.left) / Math.max(1, r.width);
    this.render(v);
    this.onInput(this.value);
  }

  private render(v: number): void {
    this.value = Math.min(1, Math.max(0, Number.isFinite(v) ? v : 0));
    const pct = this.value * 100;
    if (this.opts.vertical) {
      if (this.opts.bipolar) {
        const lo = Math.min(pct, 50);
        const hi = Math.max(pct, 50);
        this.fill.style.bottom = `${lo}%`;
        this.fill.style.height = `${hi - lo}%`;
      } else {
        this.fill.style.bottom = '0';
        this.fill.style.height = `${pct}%`;
      }
      this.knob.style.bottom = `${pct}%`;
    } else {
      this.fill.style.width = `${pct.toFixed(2)}%`;
      this.knob.style.left = `${pct.toFixed(2)}%`;
    }
    this.el.setAttribute('aria-valuenow', String(Math.round(pct)));
  }
}
