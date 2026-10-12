// «Своя тема»: four colours with a live preview, saved in the preferences next to the built-in themes.
// A theme can be copied as a short code and pasted back (to move it to another computer or send to a friend).
import { h, toast } from './dom';
import { T } from './i18n';
import { customThemes, previewTheme, updatePrefs, type CustomTheme } from './prefs';

const HEX = /^#[0-9a-f]{6}$/i;

/** Colours a new theme starts from: the dark built-in one. */
const START: Omit<CustomTheme, 'id' | 'name'> = { base: 'dark', bg: '#1a1a1a', panel: '#222222', text: '#e0e0e0', accent: '#ff5500' };

function newId(): string {
  return Math.random().toString(36).slice(2, 10);
}

/** A theme from pasted text (the code the editor copies); null if it is not one. */
export function parseThemeCode(text: string): CustomTheme | null {
  try {
    const o = JSON.parse(text.trim()) as Partial<CustomTheme>;
    if (!o || typeof o !== 'object') return null;
    const ok = (c: unknown): c is string => typeof c === 'string' && HEX.test(c);
    if (!ok(o.bg) || !ok(o.panel) || !ok(o.text) || !ok(o.accent)) return null;
    return { id: newId(), name: String(o.name ?? T('Моя тема')).slice(0, 30), base: o.base === 'light' ? 'light' : 'dark', bg: o.bg, panel: o.panel, text: o.text, accent: o.accent };
  } catch {
    return null;
  }
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
  updatePrefs((p) => {
    p.customThemes = (p.customThemes ?? []).filter((x) => x.id !== id);
    if (p.theme === `custom:${id}`) p.theme = 'dark';
  });
}

/** Editor dialog. `existing`: change this theme; otherwise a new one is made. */
export function editTheme(existing?: CustomTheme): void {
  const t: CustomTheme = existing ? { ...existing } : { id: newId(), name: T('Моя тема'), ...START };
  const name = h('input', { class: 'input', maxlength: '30', value: t.name, spellcheck: 'false', 'aria-label': T('Название') });

  const colour = (key: 'bg' | 'panel' | 'text' | 'accent', label: string) => {
    const input = h('input', { type: 'color', class: 'te-colour', value: t[key], 'aria-label': label });
    input.addEventListener('input', () => {
      t[key] = input.value;
      previewTheme(t);
    });
    return h('label', { class: 'te-row' }, h('span', { text: label }), input);
  };

  const base = h('div', { class: 'seg', role: 'radiogroup', 'aria-label': T('Основа') });
  const renderBase = () =>
    base.replaceChildren(
      ...(['dark', 'light'] as const).map((b) => {
        const el = h('button', { type: 'button', class: `seg-tab${t.base === b ? ' is-active' : ''}`, role: 'radio', 'aria-checked': String(t.base === b), text: b === 'dark' ? T('Тёмная') : T('Светлая') });
        el.addEventListener('click', () => {
          t.base = b;
          renderBase();
          previewTheme(t);
        });
        return el;
      }),
    );
  renderBase();

  const copy = h('button', { type: 'button', class: 'btn btn-quiet', text: T('Скопировать код темы') });
  copy.addEventListener('click', async () => {
    t.name = name.value.trim() || t.name;
    await navigator.clipboard.writeText(JSON.stringify({ name: t.name, base: t.base, bg: t.bg, panel: t.panel, text: t.text, accent: t.accent })).catch(() => undefined);
    toast(T('Код темы скопирован'));
  });

  const cancel = h('button', { type: 'button', class: 'btn', text: T('Отмена') });
  const save = h('button', { type: 'button', class: 'btn btn-primary', text: T('Сохранить') });
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
    h('p', { class: 'muted small', text: T('Остальные оттенки подбираются сами. Изменения видны сразу.') }),
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
  save.addEventListener('click', () => {
    t.name = name.value.trim() || T('Моя тема');
    if (![t.bg, t.panel, t.text, t.accent].every((c) => HEX.test(c))) return;
    document.removeEventListener('keydown', onKey, true);
    backdrop.remove();
    saveTheme(t, true);
  });
  document.addEventListener('keydown', onKey, true);
  document.body.append(backdrop);
  previewTheme(t);
  name.focus();
}

/** Import: asks for the pasted code. Returns true when a theme was added. */
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
