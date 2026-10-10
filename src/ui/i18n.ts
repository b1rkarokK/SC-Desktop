// Interface language. The Russian text itself is the key: `T('Слушать')`
// reads naturally in the code and falls back to Russian when a translation
// is missing. `{0}`, `{1}` … are filled from the arguments.
//
// The language is known before any module renders (localStorage, then the
// system language), so module-level labels are translated too; changing it
// reloads the window.
import { EN, EN_PLURAL } from './i18n_en';

export type Lang = 'ru' | 'en';

const KEY = 'scdesk.lang';

function detect(): Lang {
  try {
    const saved = localStorage.getItem(KEY);
    if (saved === 'ru' || saved === 'en') return saved;
  } catch {
    // storage blocked: system language
  }
  const sys = (navigator.languages?.[0] ?? navigator.language ?? 'ru').toLowerCase();
  // Russian also for the neighbours who mostly read it
  return /^(ru|uk|be|kk|ky|uz|tg|hy|az|ka)\b/.test(sys) ? 'ru' : 'en';
}

let lang: Lang = detect();

export function currentLang(): Lang {
  return lang;
}

/** Saves the choice; the caller reloads the window. */
export function setLang(l: Lang): void {
  lang = l;
  try {
    localStorage.setItem(KEY, l);
  } catch {
    // not remembered: the system language next time
  }
  document.documentElement.lang = l;
}

document.documentElement.lang = lang;

function fill(s: string, args: unknown[]): string {
  return args.length ? s.replace(/\{(\d+)\}/g, (m, i) => (Number(i) < args.length ? String(args[Number(i)]) : m)) : s;
}

export function T(ru: string, ...args: unknown[]): string {
  return fill(lang === 'ru' ? ru : (EN[ru] ?? ru), args);
}

/** Russian plural forms (1 трек / 2 трека / 5 треков) or the English pair. */
export function plural(n: number, one: string, few: string, many: string): string {
  if (lang !== 'ru') {
    const en = EN_PLURAL[one];
    if (en) return Math.abs(n) === 1 ? en[0] : en[1];
  }
  const m10 = n % 10;
  const m100 = n % 100;
  if (m10 === 1 && m100 !== 11) return one;
  if (m10 >= 2 && m10 <= 4 && (m100 < 12 || m100 > 14)) return few;
  return many;
}

/** Patterns for messages that come from the core with a variable part. */
const literal = (ru: string) => ru.replace(/\{\d+\}/g, '').replace(/[^\p{L}]/gu, '');
const patterns: [RegExp, string][] = Object.entries(EN)
  // "{0} {1}" and the like would match anything: only patterns with own words
  .filter(([ru]) => ru.includes('{0}') && literal(ru).length >= 2)
  // the most specific first
  .sort(([a], [b]) => literal(b).length - literal(a).length)
  .map(([ru, en]) => {
    const rx = ru.replace(/[.*+?^$()|[\]\\]/g, '\\$&').replace(/\{(\d+)\}/g, '(.+?)');
    return [new RegExp(`^${rx}$`, 's'), en];
  });

/** Text from the core (errors, shelf names): exact entry, a pattern, or as is. */
export function tr(text: string): string {
  if (lang === 'ru' || !text) return text;
  const exact = EN[text];
  if (exact) return exact;
  for (const [rx, en] of patterns) {
    const m = rx.exec(text);
    if (m) return fill(en, m.slice(1).map((part) => tr(part)));
  }
  return text;
}
