// Equalizer popover above the player bar (mockup 9): 10 bands ±12 dB, presets, preamp.
import { api, errorMessage, type EqConfig } from './api';
import { h, toast } from './dom';
import { iconButton, I } from './icons';
import { Slider } from './slider';

const BANDS = ['32', '64', '125', '250', '500', '1k', '2k', '4k', '8k', '16k'];

const PRESETS: Record<string, [string, number[]]> = {
  flat: ['Ровно', [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]],
  bass: ['Бас +', [6, 5, 3, 1, 0, -1, 0, 1, 2, 2]],
  bass_cut: ['Меньше баса', [-6, -4, -2, 0, 0, 0, 0, 0, 0, 0]],
  treble: ['Высокие +', [0, 0, 0, 0, 0, 1, 2, 4, 5, 6]],
  vocal: ['Вокал', [-2, -2, -1, 1, 3, 4, 3, 1, 0, -1]],
  rock: ['Рок', [5, 3, 1, -1, -2, -1, 1, 3, 4, 5]],
  electronic: ['Электроника', [5, 4, 1, 0, -2, 1, 0, 2, 4, 5]],
  acoustic: ['Акустика', [3, 2, 1, 1, 2, 2, 3, 3, 2, 1]],
  night: ['Ночь (тихо)', [-3, -2, 0, 1, 2, 2, 1, 0, -2, -4]],
  custom: ['Свой', [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]],
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
    const close = iconButton(I.close, 'Закрыть');
    close.addEventListener('click', () => this.setOpen(false));

    const bands = h('div', { class: 'eq-bands' });
    BANDS.forEach((label, i) => {
      const val = h('span', { class: 'eq-val' });
      const s = new Slider(
        `${label} Гц`,
        (v) => this.setBand(i, v, false),
        (v) => this.setBand(i, v, true),
        { vertical: true, bipolar: true },
      );
      this.sliders.push(s);
      this.values.push(val);
      bands.append(h('div', { class: 'eq-band' }, s.el, h('span', { class: 'eq-freq', text: label }), val));
    });
    this.preampSlider = new Slider('Предусилитель', (v) => this.setPreamp(v, false), (v) => this.setPreamp(v, true));

    this.el = h(
      'div',
      { class: 'eq-pop panel', role: 'dialog', 'aria-label': 'Эквалайзер' },
      h('div', { class: 'eq-head' }, h('span', { class: 'strong', text: 'Эквалайзер' }), this.enabledBtn, h('div', { class: 'spacer' }), this.preset, close),
      h('div', { class: 'eq-scale' }, h('span', { text: '+12' }), h('span', { text: '0' }), h('span', { text: '−12' })),
      bands,
      h('div', { class: 'eq-pre' }, h('span', { class: 'muted small', text: 'Предусилитель' }), this.preampSlider.el, this.preampValue),
    );
    this.el.hidden = true;
    document.addEventListener('keydown', (e) => e.key === 'Escape' && this.open && this.setOpen(false));
  }

  init(eq: EqConfig, onToggle: (open: boolean, enabled: boolean) => void): void {
    this.eq = { ...eq, gains: [...eq.gains] };
    this.onToggle = onToggle;
    this.render();
  }

  toggle(): void {
    this.setOpen(!this.open);
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
    this.preampValue.textContent = `${this.eq.preamp > 0 ? '+' : ''}${String(this.eq.preamp).replace('.', ',')} дБ`;
    this.el.classList.toggle('is-off', !this.eq.enabled);
  }
}
