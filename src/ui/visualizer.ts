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
  /** the rings of «Пульс» (born on a bass hit): radius 0..1, strength 0..1 */
  private rings: { r: number; a: number }[] = [];
  private bassBefore = 0;
  private lastRing = 0;
  /** the sparks of «Частицы» */
  private sparks: { x: number; y: number; vy: number; vx: number; life: number; size: number }[] = [];

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
      peak > 0.01 || this.mode === 'wave' || this.mode === 'aurora' || this.mode === 'pulse' || this.mode === 'particles' || this.mode === 'mountains'
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
      {
        // no cover on screen (text-only layout): the ring sits in the middle, behind the lyrics
        const shown = !!r && r.width > 0 && r.height > 0;
        const cx = shown ? (r.left + r.width / 2 - cv.left) * k : w / 2;
        const cy = shown ? (r.top + r.height / 2 - cv.top) * k : h * 0.46;
        const rad = shown ? (Math.max(r.width, r.height) / 2 + 14) * k : Math.min(w, h) * 0.3;
        const n = 64;
        ctx.lineWidth = 3 * k;
        ctx.lineCap = 'round';
        ctx.globalAlpha = shown ? 0.85 : 0.45;
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
    } else if (this.mode === 'aurora') {
      // slow ribbons of light, hung from the top: the bass swells them, the treble ripples their edges
      this.phase += 0.006;
      const bass = (this.level[1]! + this.level[2]! + this.level[3]!) / 3;
      const treble = (this.level[16]! + this.level[22]! + this.level[28]!) / 3;
      const ribbons = 4;
      for (let i = 0; i < ribbons; i++) {
        const top = h * (0.12 + i * 0.07);
        const depth = h * (0.18 + bass * 0.22 + i * 0.03);
        const grad = ctx.createLinearGradient(0, top - depth * 0.3, 0, top + depth);
        grad.addColorStop(0, 'rgba(0,0,0,0)');
        grad.addColorStop(0.35, accent);
        grad.addColorStop(1, 'rgba(0,0,0,0)');
        ctx.fillStyle = grad;
        ctx.globalAlpha = 0.16 + 0.1 * (1 - i / ribbons) + bass * 0.12;
        ctx.beginPath();
        const steps = 80;
        for (let s = 0; s <= steps; s++) {
          const x = (s / steps) * w;
          const lv = this.level[Math.min(BANDS - 1, Math.floor((s / steps) * BANDS))]!;
          const y = top + Math.sin(s / 11 + this.phase * (1.5 + i * 0.4) + i * 2) * h * 0.05 + Math.sin(s / 5 - this.phase * 4 + i) * h * 0.02 * (0.3 + treble) - lv * h * 0.05;
          if (s === 0) ctx.moveTo(x, y);
          else ctx.lineTo(x, y);
        }
        ctx.lineTo(w, top + depth);
        ctx.lineTo(0, top + depth);
        ctx.closePath();
        ctx.fill();
      }
      ctx.fillStyle = accent;
    } else if (this.mode === 'pulse') {
      // rings leave the middle on every bass hit and fade out
      const bass = (this.level[0]! + this.level[1]! + this.level[2]! + this.level[3]!) / 4;
      const now = performance.now();
      if (bass > 0.35 && bass > this.bassBefore * 1.18 && now - this.lastRing > 180) {
        this.rings.push({ r: 0.04, a: Math.min(1, 0.5 + bass) });
        this.lastRing = now;
      }
      this.bassBefore = this.bassBefore * 0.7 + bass * 0.3;
      const cx = w / 2;
      const cy = h * 0.46;
      const maxR = Math.hypot(w, h) * 0.5;
      ctx.lineWidth = 3 * k;
      this.rings = this.rings.filter((ring) => ring.a > 0.02);
      for (const ring of this.rings) {
        ring.r += 0.012 + bass * 0.006;
        ring.a *= 0.968;
        ctx.globalAlpha = ring.a * 0.8;
        ctx.beginPath();
        ctx.arc(cx, cy, ring.r * maxR, 0, Math.PI * 2);
        ctx.stroke();
      }
      // a soft core that breathes with the bass
      const core = Math.min(w, h) * (0.05 + bass * 0.08);
      const g = ctx.createRadialGradient(cx, cy, 0, cx, cy, core * 3);
      g.addColorStop(0, accent);
      g.addColorStop(1, 'rgba(0,0,0,0)');
      ctx.globalAlpha = 0.28 + bass * 0.3;
      ctx.fillStyle = g;
      ctx.beginPath();
      ctx.arc(cx, cy, core * 3, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillStyle = accent;
    } else if (this.mode === 'particles') {
      // sparks rise from the bottom: a louder band throws more of them, faster
      for (let i = 0; i < BANDS; i += 2) {
        const lv = this.level[i]!;
        if (lv > 0.15 && Math.random() < lv * 0.6 && this.sparks.length < 260) {
          this.sparks.push({ x: ((i + Math.random()) / BANDS) * w, y: h * (0.9 + Math.random() * 0.08), vy: -(1.2 + lv * 5) * k, vx: (Math.random() - 0.5) * 1.2 * k, life: 1, size: (1.5 + lv * 3.5) * k });
        }
      }
      this.sparks = this.sparks.filter((p) => p.life > 0.02 && p.y > -10);
      for (const p of this.sparks) {
        p.x += p.vx + Math.sin(p.y / 60) * 0.4 * k;
        p.y += p.vy;
        p.life *= 0.985;
        ctx.globalAlpha = Math.min(0.9, p.life);
        ctx.beginPath();
        ctx.arc(p.x, p.y, p.size, 0, Math.PI * 2);
        ctx.fill();
      }
    } else if (this.mode === 'mountains') {
      // three ranges one behind the other, the far ones calmer: the whole spectrum is the skyline
      this.phase += 0.004;
      for (let layer = 0; layer < 3; layer++) {
        const base = h * (0.62 + layer * 0.14);
        const amp = h * (0.28 - layer * 0.07);
        ctx.globalAlpha = 0.18 + layer * 0.14;
        ctx.beginPath();
        ctx.moveTo(0, h);
        const steps = 60;
        for (let s = 0; s <= steps; s++) {
          const f = s / steps;
          const lv = this.level[Math.min(BANDS - 1, Math.floor(f * (BANDS - 1)))]!;
          const calm = 1 - layer * 0.3;
          const y = base - (lv * amp * calm + Math.sin(f * 7 + this.phase * (1 + layer) + layer * 2) * h * 0.02);
          ctx.lineTo(f * w, y);
        }
        ctx.lineTo(w, h);
        ctx.closePath();
        ctx.fill();
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
