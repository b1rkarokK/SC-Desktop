// Sleep timer, own time: minutes ("90") or a clock time ("2:30" → at 02:30).
import { h } from './dom';
import { T } from './i18n';

/** Minutes from now; "2:30" is the next 02:30 (today or tomorrow). */
export function parseSleep(input: string, now = new Date()): number | null {
  const s = input.trim().replace(/\s+/g, '');
  const clock = /^(\d{1,2})[:.](\d{2})$/.exec(s);
  if (clock) {
    const hh = Number(clock[1]);
    const mm = Number(clock[2]);
    if (hh > 23 || mm > 59) return null;
    const at = new Date(now);
    at.setHours(hh, mm, 0, 0);
    if (at.getTime() <= now.getTime()) at.setDate(at.getDate() + 1);
    return Math.max(1, Math.ceil((at.getTime() - now.getTime()) / 60_000));
  }
  if (/^\d{1,4}$/.test(s)) {
    const m = Number(s);
    return m >= 1 && m <= 24 * 60 ? m : null;
  }
  return null;
}

export function askSleepTime(): Promise<number | null> {
  return new Promise((done) => {
    const input = h('input', { class: 'input', placeholder: T('90 или 2:30'), maxlength: '5', spellcheck: 'false', inputmode: 'numeric' });
    const err = h('div', { class: 'small err', hidden: true });
    const cancel = h('button', { type: 'button', class: 'btn', text: T('Отмена') });
    const ok = h('button', { type: 'button', class: 'btn btn-primary', text: T('Поставить') });
    const box = h(
      'div',
      { class: 'upd panel', role: 'dialog', 'aria-modal': 'true', 'aria-label': T('Таймер сна') },
      h('div', { class: 'upd-title', text: T('Таймер сна') }),
      h('div', { class: 'muted small', text: T('Минуты (например, 90) или время на часах (например, 2:30). Пауза с плавным затуханием.') }),
      input,
      err,
      h('div', { class: 'upd-actions' }, cancel, ok),
    );
    const backdrop = h('div', { class: 'upd-backdrop' }, box);
    const close = (v: number | null) => {
      backdrop.remove();
      done(v);
    };
    const submit = () => {
      const m = parseSleep(input.value);
      if (m === null) {
        err.textContent = T('Введите минуты (1–1440) или время вида 2:30');
        err.hidden = false;
        input.focus();
        return;
      }
      close(m);
    };
    input.addEventListener('input', () => (err.hidden = true));
    input.addEventListener('keydown', (ev) => {
      if (ev.key === 'Enter') submit();
      else if (ev.key === 'Escape') close(null);
    });
    cancel.addEventListener('click', () => close(null));
    ok.addEventListener('click', submit);
    backdrop.addEventListener('pointerdown', (ev) => ev.target === backdrop && close(null));
    document.body.append(backdrop);
    input.focus();
  });
}
