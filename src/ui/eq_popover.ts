// Equalizer popover above the player bar (mockup 9): 10 bands ±12 dB, presets,
// preamp; below them the effects: speed (slowed / sped up), reverb, loudness
// leveling, crossfade.
import { api, errorMessage, type EqConfig, type FxConfig } from './api';
import { h, toast } from './dom';
import { iconButton, I } from './icons';
import { Slider } from './slider';
import { T } from './i18n';

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
  private eq: EqConfig = { enabled: false, preamp: 0, gains: Array(10).fill(0), preset: 'flat' };
  private sliders: Slider[] = [];
  private values: HTMLElement[] = [];
  private preset: HTMLSelectElement;
  private enabledBtn: HTMLButtonElement;
  private preampSlider: Slider;
  private preampValue = h('span', { class: 'eq-val' });
  private open = false;
  private fx: FxConfig = { speed: 1, reverb: 0, normalize: true, crossfade: 0 };
  private speedSlider: Slider;
  private speedValue = h('span', { class: 'eq-val' });
  private speedChips: [number, HTMLButtonElement][] = [];
  private reverbSlider: Slider;
  private reverbValue = h('span', { class: 'eq-val' });
  private crossSlider: Slider;
  private crossValue = h('span', { class: 'eq-val' });
  private normBtn: HTMLButtonElement;
  private onToggle?: (open: boolean, enabled: boolean) => void;

  constructor() {
    this.enabledBtn = h('button', { type: 'button', class: 'toggle', role: 'switch' }, h('span', { class: 'toggle-knob' }));
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

    const bands = h('div', { class: 'eq-bands' });
    BANDS.forEach((label, i) => {
      const val = h('span', { class: 'eq-val' });
      const s = new Slider(
        T('{0} Гц', label),
        (v) => this.setBand(i, v, false),
        (v) => this.setBand(i, v, true),
        { vertical: true, bipolar: true },
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
    this.normBtn = h('button', { type: 'button', class: 'toggle', role: 'switch' }, h('span', { class: 'toggle-knob' }));
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
      fxRow(T('Скорость'), this.speedSlider.el, this.speedValue),
      fxRow('Reverb', this.reverbSlider.el, this.reverbValue),
      fxRow(T('Кроссфейд'), this.crossSlider.el, this.crossValue),
      h('div', { class: 'fx-row fx-norm' }, h('span', { class: 'muted small', text: T('Выравнивать громкость треков') }), this.normBtn),
    );

    this.el = h(
      'div',
      { class: 'eq-pop panel', role: 'dialog', 'aria-label': T('Эквалайзер') },
      h('div', { class: 'eq-head' }, h('span', { class: 'strong', text: T('Эквалайзер') }), this.enabledBtn, h('div', { class: 'spacer' }), this.preset, close),
      h('div', { class: 'eq-scale' }, h('span', { text: '+12' }), h('span', { text: '0' }), h('span', { text: '−12' })),
      bands,
      h('div', { class: 'eq-pre' }, h('span', { class: 'muted small', text: T('Предусилитель') }), this.preampSlider.el, this.preampValue),
      effects,
    );
    this.el.hidden = true;
    document.addEventListener('keydown', (e) => e.key === 'Escape' && this.open && this.setOpen(false));
  }

  init(eq: EqConfig, fx: FxConfig | undefined, onToggle: (open: boolean, enabled: boolean) => void): void {
    this.eq = { ...eq, gains: [...eq.gains] };
    if (fx) this.fx = { ...fx };
    this.onToggle = onToggle;
    this.render();
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
    this.fx = { ...this.fx, ...patch };
    this.renderFx();
    api.fxSet(this.fx, persist).catch((e) => toast(errorMessage(e), 'error'));
  }

  private renderFx(): void {
    const f = this.fx;
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
    this.preampSlider.set((this.eq.preamp + 18) / 24);
    this.preampValue.textContent = T('{0}{1} дБ', this.eq.preamp > 0 ? '+' : '', String(this.eq.preamp).replace('.', ','));
    this.el.classList.toggle('is-off', !this.eq.enabled);
    this.renderFx();
  }
}
