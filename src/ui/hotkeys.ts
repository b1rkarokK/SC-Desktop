// Own hotkeys. Combos are stored as "Ctrl+Alt+KeyP" (modifiers + KeyboardEvent.code).
// With «Работают в играх» on, the core registers them system-wide (hotkeys.rs);
// with it off this module listens inside the window.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { api } from './api';
import { clock } from './clock';
import { lyricsData } from './lyrics_data';
import { h, toast } from './dom';
import { T } from './i18n';
import { prefs, updatePrefs } from './prefs';
import { store } from './store';

export interface HotkeyAction {
  id: string;
  label: string;
  def: string;
}

export const HOTKEY_ACTIONS: HotkeyAction[] = [
  { id: 'toggle', label: T('Пауза / играть'), def: 'Ctrl+Alt+KeyP' },
  { id: 'next', label: T('Следующий трек'), def: 'Ctrl+Alt+ArrowRight' },
  { id: 'prev', label: T('Предыдущий трек'), def: 'Ctrl+Alt+ArrowLeft' },
  { id: 'like', label: T('Лайк'), def: 'Ctrl+Alt+KeyL' },
  { id: 'volUp', label: T('Громче'), def: 'Ctrl+Alt+ArrowUp' },
  { id: 'volDown', label: T('Тише'), def: 'Ctrl+Alt+ArrowDown' },
  { id: 'mini', label: T('Мини-плеер'), def: 'Ctrl+Alt+KeyM' },
  { id: 'nowPlaying', label: T('Что играет (плашка на 3 секунды, в играх тоже)'), def: 'Ctrl+Alt+KeyN' },
  { id: 'lyricsPeek', label: T('Строка текста (плашка на 3 секунды, в играх тоже)'), def: 'Ctrl+Alt+KeyT' },
  { id: 'lyricsSync', label: T('Совместить текст со звуком (нажмите, когда строка начинается)'), def: 'Ctrl+Alt+KeyY' },
  { id: 'miniClick', label: T('Мини-плеер: нажимать в игре (20 с, ещё раз - выключить)'), def: 'Ctrl+Alt+KeyK' },
  { id: 'slowed', label: T('Slowed (медленнее, ещё раз - обычная)'), def: 'Ctrl+Alt+Comma' },
  { id: 'sped', label: T('Sped up (быстрее, ещё раз - обычная)'), def: 'Ctrl+Alt+Period' },
];

export interface HotkeyPrefs {
  inGames: boolean;
  keys: Record<string, string>;
  /** how long the mini player takes clicks over a game after its key, seconds; 0 = until the key is pressed again */
  miniClickSecs?: number;
}

export function hotkeys(): HotkeyPrefs {
  const p = (prefs() as { hotkeys?: Partial<HotkeyPrefs> }).hotkeys ?? {};
  return { inGames: p.inGames ?? true, keys: p.keys ?? {}, miniClickSecs: p.miniClickSecs ?? 20 };
}

export function comboOf(id: string): string {
  const a = HOTKEY_ACTIONS.find((x) => x.id === id);
  return hotkeys().keys[id] ?? a?.def ?? '';
}

function save(next: HotkeyPrefs): void {
  updatePrefs((p) => ((p as { hotkeys?: HotkeyPrefs }).hotkeys = next));
  void api.hotkeysApply(next);
}

export function setInGames(on: boolean): void {
  save({ ...hotkeys(), inGames: on });
}

export function setMiniClickSecs(secs: number): void {
  save({ ...hotkeys(), miniClickSecs: secs });
}

/** '' clears the key. Returns false if another action already uses it. */
export function setCombo(id: string, combo: string): boolean {
  if (combo && HOTKEY_ACTIONS.some((a) => a.id !== id && comboOf(a.id) === combo)) return false;
  const cur = hotkeys();
  save({ ...cur, keys: { ...cur.keys, [id]: combo } });
  return true;
}

export function resetCombos(): void {
  save({ ...hotkeys(), keys: {} });
}

const NAMES: Record<string, string> = {
  Space: T('Пробел'),
  ArrowRight: '→',
  ArrowLeft: '←',
  ArrowUp: '↑',
  ArrowDown: '↓',
  Escape: 'Esc',
  Backspace: '⌫',
  Comma: ',',
  Period: '.',
  Slash: '/',
  Semicolon: ';',
  Quote: "'",
  BracketLeft: '[',
  BracketRight: ']',
  Backslash: '\\',
  Minus: '-',
  Equal: '=',
  Backquote: '`',
};

export function keyName(code: string): string {
  return NAMES[code] ?? code.replace(/^Key/, '').replace(/^Digit/, '').replace(/^Numpad/, 'Num ');
}

export function formatCombo(combo: string): string[] {
  return combo ? combo.split('+').map((p) => (['Ctrl', 'Alt', 'Shift', 'Meta'].includes(p) ? p : keyName(p))) : [];
}

const MODS = new Set(['ControlLeft', 'ControlRight', 'AltLeft', 'AltRight', 'ShiftLeft', 'ShiftRight', 'MetaLeft', 'MetaRight']);

export function comboFromEvent(e: KeyboardEvent): string | null {
  if (MODS.has(e.code) || !e.code) return null;
  const parts: string[] = [];
  if (e.ctrlKey) parts.push('Ctrl');
  if (e.altKey) parts.push('Alt');
  if (e.shiftKey) parts.push('Shift');
  if (e.metaKey) parts.push('Meta');
  parts.push(e.code);
  return parts.join('+');
}

/** Fills `holder` with the combo as keycaps; a click records a new one. */
export function recorder(id: string): HTMLElement {
  const box = h('button', { type: 'button', class: 'keycaps' });
  const render = () => {
    const parts = formatCombo(comboOf(id));
    box.classList.remove('is-recording');
    box.replaceChildren(...(parts.length ? parts.map((p) => h('kbd', { text: p })) : [h('span', { class: 'muted', text: T('Не задано') })]));
  };
  render();
  box.addEventListener('click', () => {
    box.classList.add('is-recording');
    box.replaceChildren(h('span', { text: T('Нажми клавиши…') }));
    const done = () => {
      window.removeEventListener('keydown', onKey, true);
      box.removeEventListener('blur', onBlur);
      render();
    };
    const onBlur = () => done();
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.code === 'Escape') return done();
      if (e.code === 'Backspace' || e.code === 'Delete') {
        setCombo(id, '');
        return done();
      }
      const combo = comboFromEvent(e);
      if (!combo) return;
      if (!/^(Ctrl|Alt|Meta)\+|^(Shift\+)?F\d+$/.test(combo)) {
        toast(T('Добавь Ctrl или Alt, иначе клавиша будет срабатывать при наборе текста'), 'error');
        return;
      }
      if (!setCombo(id, combo)) toast(T('Эта комбинация уже занята другим действием'), 'error');
      done();
    };
    window.addEventListener('keydown', onKey, true);
    box.addEventListener('blur', onBlur);
  });
  return box;
}

// ------------------------------------------------------------ in-window

function run(id: string): void {
  switch (id) {
    case 'toggle':
      void api.toggle();
      break;
    case 'next':
      void api.next();
      break;
    case 'prev':
      void api.prev();
      break;
    case 'like': {
      const t = store.snapshot?.track;
      if (t) void store.setLiked(t, !store.liked.has(t.id));
      break;
    }
    case 'volUp':
    case 'volDown': {
      const v = Math.min(1, Math.max(0, (store.snapshot?.volume ?? 0.5) + (id === 'volUp' ? 0.05 : -0.05)));
      if (store.snapshot) store.snapshot.volume = v;
      void api.setVolume(v, true);
      store.emit('player');
      break;
    }
    case 'mini':
      void api.miniOpen();
      break;
    case 'miniClick':
      void invoke('mini_interact');
      break;
    case 'nowPlaying':
    case 'lyricsPeek':
      void invoke('np_show', { lyrics: id === 'lyricsPeek' });
      break;
    case 'slowed':
    case 'sped':
      void api.speedPreset(id);
      break;
    case 'lyricsSync':
      syncLyricsNow();
      break;
  }
}

/** The key pressed at the moment a line begins: the lyrics are moved so that the nearest line starts exactly now. */
function syncLyricsNow(): void {
  const s = lyricsData.state;
  const t = store.snapshot?.track;
  if (s.kind !== 'ready' || !t || !s.lyrics.found || !s.lyrics.synced) return void toast(T('У этого трека нет синхронного текста'));
  const now = clock.now();
  let best: number | null = null;
  for (const l of s.lyrics.lines) {
    if (l.start_ms === null || !l.text.trim()) continue;
    if (best === null || Math.abs(l.start_ms - now) < Math.abs(best - now)) best = l.start_ms;
  }
  if (best === null) return;
  const delta = Math.round(now - best);
  if (Math.abs(delta) > 8000) return void toast(T('Строка слишком далеко от этого места, сдвиньте текст кнопками − и +'));
  lyricsData.nudge(t.id, delta);
  toast(T('Текст сдвинут на {0} с', (delta / 1000).toFixed(2).replace('.', ',')));
}

/** Window-level listener, used while the keys are not global. */
export function initHotkeys(openSearch: () => void): void {
  // the core passes on keys that need the window (a hotkey while the keys are global)
  void listen<string>('hotkey:action', (e) => run(e.payload));
  window.addEventListener('keydown', (e) => {
    if (e.code === 'KeyK' && (e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey) {
      e.preventDefault();
      openSearch();
      return;
    }
    if (hotkeys().inGames) return; // the core already took them
    const combo = comboFromEvent(e);
    if (!combo) return;
    const a = HOTKEY_ACTIONS.find((x) => comboOf(x.id) === combo);
    if (a) {
      e.preventDefault();
      run(a.id);
    }
  });
}
