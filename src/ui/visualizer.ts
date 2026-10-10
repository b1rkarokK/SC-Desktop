// Full-screen visualizer: «Столбики» (bars above the controls) and «Волна»
// (soft waves behind the lyrics). The core sends 32 bands (0-255) ~30 times a
// second while it is switched on; nothing runs when the mode is off.
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { VizColor, VizMode } from './prefs';

const BANDS = 32;

export class Visualizer {
  readonly el = document.createElement('canvas');
  private ctx = this.el.getContext('2d')!;
  private target = new Float32Array(BANDS);
  private level = new Float32Array(BANDS);
  private mode: VizMode = 'off';
  private active = false;
  private raf = 0;
  private unlisten: UnlistenFn | null = null;
  private phase = 0;

  private colorMode: VizColor = 'accent';
  private cover: string | null = null;

  constructor(private bottomOf: () => number, private coverRect: () => DOMRect | null) {
    this.el.className = 'fs-viz';
    new ResizeObserver(() => this.resize()).observe(this.el);
  }

  setMode(mode: VizMode): void {
    this.mode = mode;
    this.el.hidden = mode === 'off';
    this.sync();
  }

  setColor(mode: VizColor, cover: string | null): void {
    this.colorMode = mode;
    this.cover = cover;
  }

  /** Shown + playing + not hidden behind another window. */
  setActive(active: boolean): void {
    this.active = active;
    this.sync();
  }

  private sync(): void {
    const run = this.active && this.mode !== 'off';
    if (run && !this.unlisten) {
      this.unlisten = () => {};
      void listen<number[]>('viz:bands', (e) => {
        for (let i = 0; i < BANDS; i++) this.target[i] = (e.payload[i] ?? 0) / 255;
      }).then((un) => {
        if (this.unlisten) this.unlisten = un;
        else un();
      });
      void invoke('viz_set', { on: true });
    } else if (!run && this.unlisten) {
      this.unlisten();
      this.unlisten = null;
      this.target.fill(0);
      void invoke('viz_set', { on: false });
    }
    if (run && !this.raf) this.raf = requestAnimationFrame(() => this.frame());
  }

  private resize(): void {
    const r = this.el.getBoundingClientRect();
    const k = Math.min(window.devicePixelRatio || 1, 2);
    this.el.width = Math.max(1, Math.round(r.width * k));
    this.el.height = Math.max(1, Math.round(r.height * k));
  }

  private frame(): void {
    this.raf = 0;
    if (!this.unlisten) return;
    let peak = 0;
    for (let i = 0; i < BANDS; i++) {
      const t = this.target[i]!;
      // rises fast, falls slowly
      this.level[i] = t > this.level[i]! ? this.level[i]! + (t - this.level[i]!) * 0.6 : this.level[i]! * 0.88;
      peak = Math.max(peak, this.level[i]!, t);
    }
    this.draw();
    // standing still (paused): look again rarely until sound comes back
    this.raf =
      peak > 0.01 || this.mode === 'wave'
        ? requestAnimationFrame(() => this.frame())
        : (window.setTimeout(() => this.frame(), 250) as unknown as number);
  }

  private draw(): void {
    const { ctx, el } = this;
    const w = el.width;
    const h = el.height;
    ctx.clearRect(0, 0, w, h);
    const accent = this.colorMode === 'white' ? '#ffffff' : this.colorMode === 'cover' && this.cover ? this.cover : getComputedStyle(el).getPropertyValue('--accent').trim() || '#ff5500';
    ctx.fillStyle = accent;
    ctx.strokeStyle = accent;
    const k = h / Math.max(1, el.clientHeight);
    if (this.mode === 'mirror') {
      const mid = h - this.bottomOf() * k - 8 * k - 48 * k;
      const gap = 5 * k;
      const bw = (Math.min(w, 960 * k) - gap * (BANDS - 1)) / BANDS;
      const x0 = (w - (bw * BANDS + gap * (BANDS - 1))) / 2;
      ctx.globalAlpha = 0.9;
      for (let i = 0; i < BANDS; i++) {
        const bh = Math.max(2 * k, this.level[i]! * 48 * k);
        ctx.fillRect(x0 + i * (bw + gap), mid - bh, bw, bh * 2);
      }
    } else if (this.mode === 'line') {
      const base = h - this.bottomOf() * k - 8 * k;
      const maxH = Math.min(96 * k, base);
      const width = Math.min(w, 960 * k);
      const x0 = (w - width) / 2;
      const pt = (i: number): [number, number] => [x0 + (i / (BANDS - 1)) * width, base - Math.max(2 * k, this.level[i]! * maxH)];
      ctx.beginPath();
      ctx.moveTo(x0, base);
      ctx.lineTo(...pt(0));
      for (let i = 1; i < BANDS; i++) {
        const [px, py] = pt(i - 1);
        const [cx, cy] = pt(i);
        ctx.quadraticCurveTo(px, py, (px + cx) / 2, (py + cy) / 2);
      }
      ctx.lineTo(x0 + width, base);
      ctx.closePath();
      ctx.globalAlpha = 0.35;
      ctx.fill();
      ctx.globalAlpha = 0.9;
      ctx.lineWidth = 2 * k;
      ctx.stroke();
    } else if (this.mode === 'ring') {
      const r = this.coverRect();
      const cv = this.el.getBoundingClientRect();
      if (r) {
        const cx = (r.left + r.width / 2 - cv.left) * k;
        const cy = (r.top + r.height / 2 - cv.top) * k;
        const rad = (Math.max(r.width, r.height) / 2 + 14) * k;
        const n = 64;
        ctx.lineWidth = 3 * k;
        ctx.lineCap = 'round';
        ctx.globalAlpha = 0.85;
        for (let i = 0; i < n; i++) {
          // symmetric: the low end at the top, mirrored left and right
          const f = i < n / 2 ? i / (n / 2) : (n - i) / (n / 2);
          const lv = this.level[Math.min(BANDS - 1, Math.floor(f * (BANDS - 1)))]!;
          const a = (i / n) * Math.PI * 2 - Math.PI / 2;
          const len = (3 + lv * 44) * k;
          ctx.beginPath();
          ctx.moveTo(cx + Math.cos(a) * rad, cy + Math.sin(a) * rad);
          ctx.lineTo(cx + Math.cos(a) * (rad + len), cy + Math.sin(a) * (rad + len));
          ctx.stroke();
        }
      }
    } else if (this.mode === 'bars') {
      const base = h - this.bottomOf() * k - 8 * k;
      const maxH = Math.min(96 * k, base);
      const gap = 5 * k;
      const bw = (Math.min(w, 960 * k) - gap * (BANDS - 1)) / BANDS;
      const x0 = (w - (bw * BANDS + gap * (BANDS - 1))) / 2;
      ctx.globalAlpha = 0.9;
      for (let i = 0; i < BANDS; i++) {
        const bh = Math.max(2 * k, this.level[i]! * maxH);
        ctx.fillRect(x0 + i * (bw + gap), base - bh, bw, bh);
      }
    } else {
      this.phase += 0.012;
      const mid = h * 0.55;
      const amp = h * 0.16;
      const bass = (this.level[1]! + this.level[2]! + this.level[3]! + this.level[4]!) / 4;
      const treble = (this.level[18]! + this.level[22]! + this.level[26]!) / 3;
      const lines: [number, number, number, number][] = [
        [0, 0.5, 2.5, bass],
        [1, 0.3, 2, treble],
        [2, 0.18, 1.5, (bass + treble) / 2],
      ];
      for (const [i, alpha, lw, mix] of lines) {
        ctx.globalAlpha = alpha;
        ctx.lineWidth = lw * k;
        ctx.beginPath();
        const steps = 120;
        for (let s = 0; s <= steps; s++) {
          const x = (s / steps) * w;
          const band = this.level[Math.min(BANDS - 1, Math.floor((s / steps) * BANDS))]!;
          const y = mid + Math.sin(s / 9 + this.phase * (2 + i) + i * 1.7) * amp * (0.15 + mix * 0.9) + Math.sin(s / 4 - this.phase * 3) * amp * band * 0.5;
          if (s === 0) ctx.moveTo(x, y);
          else ctx.lineTo(x, y);
        }
        ctx.stroke();
      }
    }
    ctx.globalAlpha = 1;
  }
}
