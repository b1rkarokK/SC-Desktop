// Equalizer popover above the player bar (mockup 9): 10 bands ±12 dB, presets,
// preamp; below them the effects: speed (slowed / sped up), reverb, loudness
// leveling, crossfade.
import { listen } from '@tauri-apps/api/event';
import { api, errorMessage, type EqConfig, type FxConfig } from './api';
import { h, toast } from './dom';
import { iconButton, I } from './icons';
import { Slider } from './slider';
import { store } from './store';
import { T } from './i18n';
import { onRoom, room } from './views/rooms';

const BANDS = ['32', '64', '125', '250', '500', '1k', '2k', '4k', '8k', '16k'];

const PRESETS: Record<string, [string, number[]]> = {
  flat: [T('Ровно'), [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]],
  bass: [T('Бас +'), [6, 5, 3, 1, 0, -1, 0, 1, 2, 2]],
  bass_cut: [T('Меньше баса'), [-6, -4, -2, 0, 0, 0, 0, 0, 0, 0]],
  treble: [T('Высокие +'), [0, 0, 0, 0, 0, 1, 2, 4, 5, 6]],
  vocal: [T('Вокал'), [-2, -2, -1, 1, 3, 4, 3, 1, 0, -1]],
  rock: [T('Рок'), [5, 3, 1, -1, -2, -1, 1, 3, 4, 5]],
  electronic: [T('Электроника'), [5, 4, 1, 0, -2, 1, 0, 2, 4, 5]],
  acoustic: [T('Акустика'), [3, 2, 1, 1, 2, 2, 3, 3, 2, 1]],
  night: [T('Ночь (тихо)'), [-3, -2, 0, 1, 2, 2, 1, 0, -2, -4]],
  custom: [T('Свой'), [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]],
};

const toDb = (v: number) => Math.round((v * 24 - 12) * 2) / 2;
const toPos = (db: number) => (db + 12) / 24;

export class EqPopover {
  readonly el: HTMLDivElement;
  private deviceBtn = h('button', { type: 'button', class: 'toggle', role: 'switch', 'aria-label': T('Свой для каждого устройства') }, h('span', { class: 'toggle-knob' }));
  private deviceName = h('span', { class: 'muted small' });
  private deviceRow = h('div', { class: 'eq-device small' });
  private note = h('div', { class: 'eq-note small', text: T('Этот трек играет в плеере SoundCloud: эквалайзер и эффекты на него не действуют.') });
  private eq: EqConfig = { enabled: false, preamp: 0, gains: Array(10).fill(0), preset: 'flat' };
  private sliders: Slider[] = [];
  private values: HTMLElement[] = [];
  private preset: HTMLSelectElement;
  private enabledBtn: HTMLButtonElement;
  private preampSlider: Slider;
  private preampValue = h('span', { class: 'eq-val' });
  private open = false;
  /** the line of the sound over the bands: how the equalizer bends it */
  private curve!: SVGPathElement;
  private curveFill!: SVGPathElement;
  private fx: FxConfig = { speed: 1, reverb: 0, normalize: true, crossfade: 0 };
  private speedSlider: Slider;
  private speedValue = h('span', { class: 'eq-val' });
  private speedChips: [number, HTMLButtonElement][] = [];
  private speedRow!: HTMLElement;
  private speedNote = h('div', { class: 'muted small', text: T('Скорость недоступна в комнате: у всех она обычная') });
  private reverbSlider: Slider;
  private reverbValue = h('span', { class: 'eq-val' });
  private crossSlider: Slider;
  private crossValue = h('span', { class: 'eq-val' });
  private normBtn: HTMLButtonElement;
  private onToggle?: (open: boolean, enabled: boolean) => void;

  constructor() {
    this.enabledBtn = h('button', { type: 'button', class: 'toggle', role: 'switch', 'aria-label': T('Эквалайзер') }, h('span', { class: 'toggle-knob' }));
    this.enabledBtn.addEventListener('click', () => {
      this.eq.enabled = !this.eq.enabled;
      this.commit(true);
    });
    this.preset = h('select', { class: 'input select' });
    for (const [key, [label]] of Object.entries(PRESETS)) this.preset.append(h('option', { value: key, text: label }));
    this.preset.addEventListener('change', () => {
      const p = PRESETS[this.preset.value];
      if (!p) return;
      this.eq.preset = this.preset.value;
      if (this.preset.value !== 'custom') this.eq.gains = [...p[1]];
      this.eq.enabled = true;
      this.commit(true);
    });
    const close = iconButton(I.close, T('Закрыть'));
    close.addEventListener('click', () => this.setOpen(false));
    // «Сбросить»: every band and the preamp to zero
    const reset = h('button', { type: 'button', class: 'chip-btn', text: T('Сбросить'), title: T('Все полосы и предусилитель на ноль') });
    reset.addEventListener('click', () => {
      this.eq.gains = Array(10).fill(0);
      this.eq.preamp = 0;
      this.eq.preset = 'flat';
      this.commit(true);
    });
    const NS = 'http://www.w3.org/2000/svg';
    const curveSvg = document.createElementNS(NS, 'svg');
    curveSvg.setAttribute('viewBox', '0 0 100 40');
    curveSvg.setAttribute('preserveAspectRatio', 'none');
    curveSvg.setAttribute('aria-hidden', 'true');
    const zero = document.createElementNS(NS, 'line');
    zero.setAttribute('x1', '0');
    zero.setAttribute('x2', '100');
    zero.setAttribute('y1', '20');
    zero.setAttribute('y2', '20');
    zero.setAttribute('class', 'eq-curve-zero');
    this.curveFill = document.createElementNS(NS, 'path');
    this.curveFill.setAttribute('class', 'eq-curve-fill');
    this.curve = document.createElementNS(NS, 'path');
    this.curve.setAttribute('class', 'eq-curve-line');
    curveSvg.append(zero, this.curveFill, this.curve);
    const curveBox = h('div', { class: 'eq-curve' });
    curveBox.append(curveSvg);

    const bands = h('div', { class: 'eq-bands' });
    BANDS.forEach((label, i) => {
      const val = h('span', { class: 'eq-val' });
      const s = new Slider(
        T('{0} Гц', label),
        (v) => this.setBand(i, v, false),
        (v) => this.setBand(i, v, true),
        { vertical: true, bipolar: true },
      );
      // a double click puts the band back to 0 dB, the wheel moves it by half a decibel
      s.el.addEventListener('dblclick', () => this.setBand(i, toPos(0), true));
      s.el.addEventListener(
        'wheel',
        (e) => {
          e.preventDefault();
          const db = Math.max(-12, Math.min(12, (this.eq.gains[i] ?? 0) + (e.deltaY < 0 ? 0.5 : -0.5)));
          this.setBand(i, toPos(db), true);
        },
        { passive: false },
      );
      this.sliders.push(s);
      this.values.push(val);
      bands.append(h('div', { class: 'eq-band' }, s.el, h('span', { class: 'eq-freq', text: label }), val));
    });
    this.preampSlider = new Slider(T('Предусилитель'), (v) => this.setPreamp(v, false), (v) => this.setPreamp(v, true));

    // effects: speed 0.70 … 1.30, reverb 0 … 100 %, crossfade 0 … 12 s
    const speedOf = (v: number) => Math.round((0.7 + v * 0.6) * 100) / 100;
    this.speedSlider = new Slider(T('Скорость'), (v) => this.setFx({ speed: speedOf(v) }, false), (v) => this.setFx({ speed: speedOf(v) }, true));
    this.reverbSlider = new Slider('Reverb', (v) => this.setFx({ reverb: Math.round(v * 100) / 100 }, false), (v) => this.setFx({ reverb: Math.round(v * 100) / 100 }, true));
    this.crossSlider = new Slider(T('Кроссфейд'), (v) => this.setFx({ crossfade: Math.round(v * 12) }, false), (v) => this.setFx({ crossfade: Math.round(v * 12) }, true));
    this.normBtn = h('button', { type: 'button', class: 'toggle', role: 'switch', 'aria-label': T('Выравнивать громкость треков') }, h('span', { class: 'toggle-knob' }));
    this.normBtn.addEventListener('click', () => this.setFx({ normalize: !this.fx.normalize }, true));
    const chips = h('div', { class: 'fx-chips' });
    for (const [label, speed] of [['Slowed', 0.85], [T('Обычно'), 1], ['Sped up', 1.2]] as [string, number][]) {
      const b = h('button', { type: 'button', class: 'chip-btn', text: label });
      b.addEventListener('click', () => this.setFx({ speed }, true));
      this.speedChips.push([speed, b]);
      chips.append(b);
    }
    const fxRow = (label: string, control: HTMLElement, value: HTMLElement | null) =>
      h('div', { class: 'fx-row' }, h('span', { class: 'muted small', text: label }), control, value ?? h('span', { class: 'eq-val' }));
    const effects = h(
      'div',
      { class: 'fx' },
      h('div', { class: 'fx-head' }, h('span', { class: 'strong', text: T('Эффекты') }), chips),
      (this.speedRow = fxRow(T('Скорость'), this.speedSlider.el, this.speedValue)),
      this.speedNote,
      fxRow('Reverb', this.reverbSlider.el, this.reverbValue),
      fxRow(T('Кроссфейд'), this.crossSlider.el, this.crossValue),
      h('div', { class: 'fx-row fx-norm' }, h('span', { class: 'muted small', text: T('Выравнивать громкость треков') }), this.normBtn),
    );

    this.el = h(
      'div',
      { class: 'eq-pop panel', role: 'dialog', 'aria-label': T('Эквалайзер') },
      h('div', { class: 'eq-head' }, h('span', { class: 'strong', text: T('Эквалайзер') }), this.enabledBtn, h('div', { class: 'spacer' }), reset, this.preset, close),
      this.deviceRow,
      this.note,
      curveBox,
      h('div', { class: 'eq-scale' }, h('span', { text: '+12' }), h('span', { text: '0' }), h('span', { text: '−12' })),
      bands,
      h('div', { class: 'eq-pre' }, h('span', { class: 'muted small', text: T('Предусилитель') }), this.preampSlider.el, this.preampValue),
      effects,
    );
    this.el.hidden = true;
    this.note.hidden = true;
    // an equalizer of its own for every output device (speakers, headphones…), switched with it
    this.deviceRow.append(this.deviceName, h('span', { class: 'spacer' }), h('span', { class: 'muted small', text: T('Свой для каждого устройства') }), this.deviceBtn);
    this.deviceBtn.addEventListener('click', async () => {
      const on = this.deviceBtn.getAttribute('aria-checked') !== 'true';
      await api.eqPerDeviceSet(on).catch((e) => toast(errorMessage(e), 'error'));
      void this.refreshDevice();
    });
    void listen<[string, EqConfig]>('eq:changed', (e) => {
      // another device took over and brought its own settings
      this.eq = { ...e.payload[1], gains: [...e.payload[1].gains] };
      this.render();
      this.onToggle?.(this.open, this.eq.enabled);
      void this.refreshDevice();
    });
    void listen('eq:device', () => void this.refreshDevice());
    // a speed hotkey changed the effects behind our back
    void listen<FxConfig>('fx:changed', (e) => {
      this.fx = { ...e.payload };
      this.renderFx();
    });
    void this.refreshDevice();
    store.on('player', () => (this.note.hidden = !store.snapshot?.widget));
    document.addEventListener('keydown', (e) => e.key === 'Escape' && this.open && this.setOpen(false));
    onRoom(() => this.renderFx());
  }

  init(eq: EqConfig, fx: FxConfig | undefined, onToggle: (open: boolean, enabled: boolean) => void): void {
    this.eq = { ...eq, gains: [...eq.gains] };
    if (fx) this.fx = { ...fx };
    this.onToggle = onToggle;
    this.render();
    // the button lights up at once when the equalizer was on at the last exit
    onToggle(this.open, this.eq.enabled);
  }

  toggle(): void {
    this.setOpen(!this.open);
  }

  close(): void {
    if (this.open) this.setOpen(false);
  }

  private setOpen(open: boolean): void {
    this.open = open;
    this.el.hidden = !open;
    // what the window shows is what the core plays: opening it re-sends the settings
    if (open) {
      void api.eqSet(this.eq, false).catch(() => undefined);
      void this.refreshDevice();
    }
    this.onToggle?.(open, this.eq.enabled);
  }

  private setBand(i: number, v: number, persist: boolean): void {
    this.eq.gains[i] = toDb(v);
    this.eq.enabled = true;
    this.eq.preset = 'custom';
    this.commit(persist);
  }

  private setPreamp(v: number, persist: boolean): void {
    this.eq.preamp = Math.round((v * 24 - 18) * 2) / 2; // −18…+6 dB
    this.commit(persist);
  }

  private setFx(patch: Partial<FxConfig>, persist: boolean): void {
    // in a room the speed stays normal for everybody
    if (room.active() && 'speed' in patch) return;
    this.fx = { ...this.fx, ...patch };
    this.renderFx();
    api.fxSet(this.fx, persist).catch((e) => toast(errorMessage(e), 'error'));
  }

  private renderFx(): void {
    const f = this.fx;
    const locked = room.active();
    this.speedRow?.classList.toggle('is-locked', locked);
    for (const [, b] of this.speedChips) b.classList.toggle('is-locked', locked);
    this.speedNote.hidden = !locked;
    this.speedSlider.set((f.speed - 0.7) / 0.6);
    this.speedValue.textContent = `${f.speed.toFixed(2).replace('.', ',')}×`;
    for (const [speed, b] of this.speedChips) b.classList.toggle('is-active', Math.abs(f.speed - speed) < 0.005);
    this.reverbSlider.set(f.reverb);
    this.reverbValue.textContent = `${Math.round(f.reverb * 100)}%`;
    this.crossSlider.set(f.crossfade / 12);
    this.crossValue.textContent = f.crossfade ? T('{0} с', f.crossfade) : T('выкл');
    this.normBtn.classList.toggle('is-on', f.normalize);
    this.normBtn.setAttribute('aria-checked', String(f.normalize));
  }

  private async refreshDevice(): Promise<void> {
    try {
      const d = await api.eqDeviceGet();
      this.deviceRow.hidden = !d.device;
      this.deviceName.textContent = d.device ?? '';
      this.deviceBtn.classList.toggle('is-on', d.per_device);
      this.deviceBtn.setAttribute('aria-checked', String(d.per_device));
    } catch {
      this.deviceRow.hidden = true;
    }
  }

  /** The smooth line through the ten bands (the preamp lifts or lowers all of it). */
  private drawCurve(): void {
    const pts = this.eq.gains.map((g, i) => {
      const db = Math.max(-12, Math.min(12, g + this.eq.preamp));
      return [(i / 9) * 100, 20 - (db / 12) * 18] as const;
    });
    // Catmull-Rom through the points, as cubic pieces
    let d = `M ${pts[0]![0]} ${pts[0]![1].toFixed(2)}`;
    for (let i = 0; i < pts.length - 1; i++) {
      const p0 = pts[Math.max(0, i - 1)]!;
      const p1 = pts[i]!;
      const p2 = pts[i + 1]!;
      const p3 = pts[Math.min(pts.length - 1, i + 2)]!;
      const c1 = [p1[0] + (p2[0] - p0[0]) / 6, p1[1] + (p2[1] - p0[1]) / 6];
      const c2 = [p2[0] - (p3[0] - p1[0]) / 6, p2[1] - (p3[1] - p1[1]) / 6];
      d += ` C ${c1[0]!.toFixed(2)} ${c1[1]!.toFixed(2)}, ${c2[0]!.toFixed(2)} ${c2[1]!.toFixed(2)}, ${p2[0]} ${p2[1].toFixed(2)}`;
    }
    this.curve.setAttribute('d', d);
    this.curveFill.setAttribute('d', `${d} L ${pts[pts.length - 1]![0]} 20 L ${pts[0]![0]} 20 Z`);
  }

  private commit(persist: boolean): void {
    this.render();
    api.eqSet(this.eq, persist).catch((e) => toast(errorMessage(e), 'error'));
    this.onToggle?.(this.open, this.eq.enabled);
  }

  private render(): void {
    this.enabledBtn.classList.toggle('is-on', this.eq.enabled);
    this.enabledBtn.setAttribute('aria-checked', String(this.eq.enabled));
    this.preset.value = PRESETS[this.eq.preset] ? this.eq.preset : 'custom';
    this.eq.gains.forEach((g, i) => {
      this.sliders[i]!.set(toPos(g));
      this.values[i]!.textContent = (g > 0 ? '+' : '') + String(g).replace('.', ',');
    });
    this.drawCurve();
    this.preampSlider.set((this.eq.preamp + 18) / 24);
    this.preampValue.textContent = T('{0}{1} дБ', this.eq.preamp > 0 ? '+' : '', String(this.eq.preamp).replace('.', ','));
    this.el.classList.toggle('is-off', !this.eq.enabled);
    this.renderFx();
  }
}
