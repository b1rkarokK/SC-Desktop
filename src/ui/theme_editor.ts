// «Своя тема»: colours, a background picture, shapes and, for those who want it, their own CSS, with a live
// preview. Saved in the preferences next to the built-in themes; the picture lives in the database.
// Everything is static: the picture is scaled once (and blurred once, if asked), so a theme costs the
// computer nothing while the app runs. A theme can be copied as a short code and pasted back.
import { api, errorMessage } from './api';
import { h, toast } from './dom';
import { T } from './i18n';
import { customThemes, previewTheme, safeCss, updatePrefs, type CustomTheme } from './prefs';

const HEX = /^#[0-9a-f]{6}$/i;

/** Colours a new theme starts from: the dark built-in one. */
const START: Omit<CustomTheme, 'id' | 'name'> = { base: 'dark', bg: '#1a1a1a', panel: '#222222', text: '#e0e0e0', accent: '#ff5500' };

function newId(): string {
  return Math.random().toString(36).slice(2, 10);
}

const num = (v: unknown, lo: number, hi: number): number | undefined => (typeof v === 'number' && Number.isFinite(v) ? Math.min(hi, Math.max(lo, v)) : undefined);
const hex = (v: unknown): string | undefined => (typeof v === 'string' && HEX.test(v) ? v : undefined);

/** A theme from pasted text (the code the editor copies); null if it is not one. */
export function parseThemeCode(text: string): CustomTheme | null {
  try {
    const o = JSON.parse(text.trim()) as Record<string, unknown>;
    if (!o || typeof o !== 'object') return null;
    const bg = hex(o.bg);
    const panel = hex(o.panel);
    const fg = hex(o.text);
    const accent = hex(o.accent);
    if (!bg || !panel || !fg || !accent) return null;
    const t: CustomTheme = { id: newId(), name: String(o.name ?? T('Моя тема')).slice(0, 30), base: o.base === 'light' ? 'light' : 'dark', bg, panel, text: fg, accent };
    const muted = hex(o.muted);
    const border = hex(o.border);
    const hover = hex(o.hover);
    if (muted) t.muted = muted;
    if (border) t.border = border;
    if (hover) t.hover = hover;
    const radius = num(o.radius, 0, 16);
    if (radius !== undefined) t.radius = radius;
    const dim = num(o.dim, 0, 0.85);
    if (dim !== undefined) t.dim = dim;
    const shell = num(o.shell, 40, 100);
    if (shell !== undefined) t.shell = shell;
    if (typeof o.css === 'string' && o.css.trim()) t.css = o.css.slice(0, 30_000);
    return t;
  } catch {
    return null;
  }
}

/** The theme as text (the picture is too big to travel in it). */
export function themeCode(t: CustomTheme): string {
  const { id: _id, image: _image, ...rest } = t;
  return JSON.stringify(rest);
}

export function saveTheme(t: CustomTheme, activate: boolean): void {
  updatePrefs((p) => {
    const list = [...(p.customThemes ?? [])];
    const i = list.findIndex((x) => x.id === t.id);
    if (i >= 0) list[i] = t;
    else list.push(t);
    p.customThemes = list;
    if (activate) p.theme = `custom:${t.id}`;
  });
}

export function deleteTheme(id: string): void {
  void api.themeBgDelete(id).catch(() => undefined);
  updatePrefs((p) => {
    p.customThemes = (p.customThemes ?? []).filter((x) => x.id !== id);
    if (p.theme === `custom:${id}`) p.theme = 'dark';
  });
}

// ------------------------------------------------------------------ the picture

const MAX_SIDE = 1600;

/** Scales a picture once to a size the screen needs and (if asked) blurs it once: what is stored and painted is
 *  a plain still image, nothing is filtered while the app runs. */
async function bake(source: ImageBitmap, blur: number): Promise<string> {
  for (const [side, quality] of [[MAX_SIDE, 0.8], [1280, 0.7], [960, 0.6]] as [number, number][]) {
    const k = Math.min(1, side / Math.max(source.width, source.height));
    const w = Math.max(1, Math.round(source.width * k));
    const hgt = Math.max(1, Math.round(source.height * k));
    const canvas = document.createElement('canvas');
    canvas.width = w;
    canvas.height = hgt;
    const ctx = canvas.getContext('2d');
    if (!ctx) throw new Error('canvas');
    if (blur > 0) {
      // drawn a little larger so the blurred edges do not fade into transparent
      ctx.filter = `blur(${blur}px)`;
      const pad = blur * 2;
      ctx.drawImage(source, -pad, -pad, w + pad * 2, hgt + pad * 2);
    } else {
      ctx.drawImage(source, 0, 0, w, hgt);
    }
    const url = canvas.toDataURL('image/webp', quality);
    if (url.length <= 1_500_000) return url;
  }
  throw new Error(T('Картинка слишком большая'));
}

async function bitmapOf(data: string): Promise<ImageBitmap> {
  return createImageBitmap(await (await fetch(data)).blob());
}

function pickFile(): Promise<File | null> {
  return new Promise((done) => {
    const input = h('input', { type: 'file', accept: 'image/*' });
    input.addEventListener('change', () => done(input.files?.[0] ?? null));
    input.addEventListener('cancel', () => done(null));
    input.click();
  });
}

// ------------------------------------------------------------------ the editor

/** Editor dialog. `existing`: change this theme; otherwise a new one is made. */
export function editTheme(existing?: CustomTheme): void {
  const t: CustomTheme = existing ? { ...existing } : { id: newId(), name: T('Моя тема'), ...START };
  const name = h('input', { class: 'input', maxlength: '30', value: t.name, spellcheck: 'false', 'aria-label': T('Название') });
  /** undefined: the saved picture is kept; null: removed; text: a new one */
  let image: string | null | undefined;
  let source: ImageBitmap | null = null;
  let blur = 0;

  const repaint = () => previewTheme(t, image);

  const colour = (key: 'bg' | 'panel' | 'text' | 'accent', label: string) => {
    const input = h('input', { type: 'color', class: 'te-colour', value: t[key], 'aria-label': label });
    input.addEventListener('input', () => {
      t[key] = input.value;
      repaint();
    });
    return h('label', { class: 'te-row' }, h('span', { text: label }), input);
  };

  /** An optional shade: the automatic one until a colour is chosen. */
  const shade = (key: 'muted' | 'border' | 'hover', label: string) => {
    const input = h('input', { type: 'color', class: 'te-colour', value: t[key] ?? '#808080', 'aria-label': label });
    const auto = h('button', { type: 'button', class: 'btn btn-quiet', text: T('Авто') });
    const paint = () => {
      auto.hidden = t[key] === undefined;
      input.style.opacity = t[key] === undefined ? '0.45' : '1';
    };
    input.addEventListener('input', () => {
      t[key] = input.value;
      paint();
      repaint();
    });
    auto.addEventListener('click', () => {
      delete t[key];
      paint();
      repaint();
    });
    paint();
    return h('label', { class: 'te-row' }, h('span', { text: label }), h('span', { class: 'te-inline' }, auto, input));
  };

  const range = (label: string, min: number, max: number, step: number, value: number, show: (v: number) => string, set: (v: number) => void) => {
    const out = h('span', { class: 'muted small te-val', text: show(value) });
    const input = h('input', { type: 'range', class: 'te-range', min: String(min), max: String(max), step: String(step), value: String(value), 'aria-label': label });
    input.addEventListener('input', () => {
      const v = Number(input.value);
      out.textContent = show(v);
      set(v);
    });
    return { row: h('label', { class: 'te-row' }, h('span', { text: label }), h('span', { class: 'te-inline' }, input, out)), input };
  };

  const base = h('div', { class: 'seg', role: 'radiogroup', 'aria-label': T('Основа') });
  const renderBase = () =>
    base.replaceChildren(
      ...(['dark', 'light'] as const).map((b) => {
        const el = h('button', { type: 'button', class: `seg-tab${t.base === b ? ' is-active' : ''}`, role: 'radio', 'aria-checked': String(t.base === b), text: b === 'dark' ? T('Тёмная') : T('Светлая') });
        el.addEventListener('click', () => {
          t.base = b;
          renderBase();
          repaint();
        });
        return el;
      }),
    );
  renderBase();

  // the picture
  const pickBtn = h('button', { type: 'button', class: 'btn', text: T('Выбрать картинку…') });
  const dropBtn = h('button', { type: 'button', class: 'btn btn-quiet', text: T('Убрать') });
  const hasPicture = () => (image === undefined ? !!t.image : image !== null);
  const picState = h('span', { class: 'muted small' });
  const dim = range(T('Затемнение картинки'), 0, 85, 5, Math.round((t.dim ?? 0.35) * 100), (v) => `${v}%`, (v) => {
    t.dim = v / 100;
    repaint();
  });
  const shell = range(T('Непрозрачность панелей'), 40, 100, 5, t.shell ?? 80, (v) => `${v}%`, (v) => {
    t.shell = v;
    repaint();
  });
  let blurTimer = 0;
  const blurRange = range(T('Размытие картинки'), 0, 20, 1, 0, (v) => (v ? `${v} px` : T('нет')), (v) => {
    blur = v;
    window.clearTimeout(blurTimer);
    blurTimer = window.setTimeout(async () => {
      try {
        if (!source) {
          const saved = existing?.image && image === undefined ? await api.themeBgGet(existing.id) : null;
          if (!saved) return;
          source = await bitmapOf(saved);
        }
        image = await bake(source, blur);
        repaint();
      } catch (e) {
        toast(errorMessage(e), 'error');
      }
    }, 250);
  });
  const syncPicture = () => {
    const on = hasPicture();
    dropBtn.hidden = !on;
    picState.textContent = on ? T('Картинка выбрана') : T('Без картинки');
    for (const r of [dim.row, shell.row, blurRange.row]) r.hidden = !on;
  };
  pickBtn.addEventListener('click', async () => {
    const file = await pickFile();
    if (!file) return;
    try {
      source = await createImageBitmap(file);
      image = await bake(source, blur);
      syncPicture();
      repaint();
    } catch (e) {
      toast(errorMessage(e), 'error');
    }
  });
  dropBtn.addEventListener('click', () => {
    image = null;
    source = null;
    syncPicture();
    repaint();
  });
  syncPicture();

  const radius = range(T('Скругление углов'), 0, 16, 1, t.radius ?? 4, (v) => `${v} px`, (v) => {
    t.radius = v;
    repaint();
  });

  // the user's own CSS
  const css = h('textarea', { class: 'input te-css', rows: '6', spellcheck: 'false', placeholder: ':root { --accent: #ff3366; }\n.btn { border-radius: 999px; }', 'aria-label': T('Свой CSS') });
  css.value = t.css ?? '';
  const cssNote = h('p', { class: 'muted small', text: T('Применяется поверх темы. Внешние ссылки и @import не работают: тема остаётся лёгкой.') });
  let cssTimer = 0;
  css.addEventListener('input', () => {
    t.css = css.value;
    window.clearTimeout(cssTimer);
    cssTimer = window.setTimeout(() => {
      cssNote.textContent = safeCss(css.value).length < css.value.length ? T('Часть CSS убрана: внешние ссылки и @import не работают.') : T('Применяется поверх темы. Внешние ссылки и @import не работают: тема остаётся лёгкой.');
      repaint();
    }, 250);
  });

  const copy = h('button', { type: 'button', class: 'btn btn-quiet', text: T('Скопировать код темы') });
  copy.addEventListener('click', async () => {
    t.name = name.value.trim() || t.name;
    await navigator.clipboard.writeText(themeCode(t)).catch(() => undefined);
    toast(T('Код темы скопирован (без картинки)'));
  });

  const cancel = h('button', { type: 'button', class: 'btn', text: T('Отмена') });
  const save = h('button', { type: 'button', class: 'btn btn-primary', text: T('Сохранить') });
  const advanced = h('details', { class: 'te-more' }, h('summary', { text: T('Ещё цвета') }), shade('muted', T('Приглушённый текст')), shade('border', T('Границы')), shade('hover', T('Наведение')));
  const cssBox = h('details', { class: 'te-more' }, h('summary', { text: T('Свой CSS') }), css, cssNote);
  if (t.css) cssBox.open = true;
  const box = h(
    'div',
    { class: 'upd panel te', role: 'dialog', 'aria-modal': 'true', 'aria-label': T('Своя тема') },
    h('div', { class: 'upd-title', text: existing ? T('Изменить тему') : T('Новая тема') }),
    name,
    colour('bg', T('Фон')),
    colour('panel', T('Панели')),
    colour('text', T('Текст')),
    colour('accent', T('Акцент')),
    h('div', { class: 'te-row' }, h('span', { text: T('Основа') }), base),
    advanced,
    h('div', { class: 'te-row' }, h('span', { text: T('Фоновая картинка') }), h('span', { class: 'te-inline' }, picState, dropBtn, pickBtn)),
    dim.row,
    shell.row,
    blurRange.row,
    radius.row,
    cssBox,
    h('p', { class: 'muted small', text: T('Остальные оттенки подбираются сами. Изменения видны сразу. Картинка уменьшается и сохраняется один раз, на скорость программы она не влияет.') }),
    copy,
    h('div', { class: 'upd-actions' }, cancel, save),
  );
  const backdrop = h('div', { class: 'upd-backdrop te-backdrop' }, box);
  const close = () => {
    document.removeEventListener('keydown', onKey, true);
    backdrop.remove();
    previewTheme(null); // back to the saved look
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'Escape') close();
  };
  cancel.addEventListener('click', close);
  save.addEventListener('click', async () => {
    t.name = name.value.trim() || T('Моя тема');
    if (![t.bg, t.panel, t.text, t.accent].every((c) => HEX.test(c))) return;
    try {
      if (typeof image === 'string') {
        await api.themeBgSet(t.id, image);
        t.image = true;
      } else if (image === null) {
        await api.themeBgDelete(t.id);
        t.image = false;
      }
    } catch (e) {
      toast(errorMessage(e), 'error');
      return;
    }
    if (!t.css?.trim()) delete t.css;
    document.removeEventListener('keydown', onKey, true);
    backdrop.remove();
    saveTheme(t, true);
  });
  document.addEventListener('keydown', onKey, true);
  document.body.append(backdrop);
  repaint();
  name.focus();
}

/** Import: reads the theme code from the clipboard. Returns true when a theme was added. */
export async function importTheme(): Promise<boolean> {
  let text = '';
  try {
    text = await navigator.clipboard.readText();
  } catch {
    /* no clipboard access: the message below says what to do */
  }
  const t = parseThemeCode(text);
  if (!t) {
    toast(T('В буфере нет кода темы. Скопируйте его в редакторе темы и попробуйте снова.'), 'error');
    return false;
  }
  saveTheme(t, true);
  toast(T('Тема «{0}» добавлена', t.name));
  return true;
}

export { customThemes };
