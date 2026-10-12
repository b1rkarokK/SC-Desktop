// «Поделиться треком»: a link, optionally starting from the current second.
import { api, errorMessage, fmtTime, type Track } from './api';
import { clock } from './clock';
import { h, toast } from './dom';
import { T } from './i18n';
import { store } from './store';

function stamp(ms: number): string {
  const s = Math.floor(ms / 1000);
  return `#t=${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

export function shareDialog(t: Track): void {
  if (!t.permalink_url) {
    toast(T('У этого трека нет ссылки'), 'error');
    return;
  }
  const url = t.permalink_url;
  // a timecode only makes sense for the track that plays now
  const playing = store.currentId() === t.id;
  const at = playing ? clock.now() : 0;
  let withTime = playing && at >= 1000;

  const sw = h('button', { type: 'button', class: 'toggle', role: 'switch', 'aria-label': T('Начать с {0}', fmtTime(at)) }, h('span', { class: 'toggle-knob' }));
  const field = h('input', { class: 'input', readOnly: true, 'aria-label': T('Ссылка') });
  const render = () => {
    sw.classList.toggle('is-on', withTime);
    sw.setAttribute('aria-checked', String(withTime));
    field.value = url + (withTime ? stamp(at) : '');
  };
  sw.addEventListener('click', () => {
    withTime = !withTime;
    render();
  });
  render();

  const copy = h('button', { type: 'button', class: 'btn btn-primary', text: T('Скопировать ссылку') });
  const open = h('button', { type: 'button', class: 'btn', text: T('Открыть') });
  const close = h('button', { type: 'button', class: 'btn', text: T('Закрыть') });
  const box = h(
    'div',
    { class: 'upd panel', role: 'dialog', 'aria-modal': 'true', 'aria-label': T('Поделиться треком') },
    h('div', { class: 'upd-title', text: T('Поделиться треком') }),
    h('div', { class: 'muted small', dir: 'auto', text: `${t.title} - ${t.artist}` }),
    playing && at >= 1000
      ? h('div', { class: 'set-row' }, h('span', { text: T('Начать с {0}', fmtTime(at)) }), sw)
      : null,
    field,
    h('div', { class: 'upd-actions' }, close, open, copy),
  );
  const backdrop = h('div', { class: 'upd-backdrop' }, box);
  const done = () => backdrop.remove();
  close.addEventListener('click', done);
  backdrop.addEventListener('pointerdown', (e) => e.target === backdrop && done());
  box.addEventListener('keydown', (e) => e.key === 'Escape' && done());
  open.addEventListener('click', () => void api.openExternal(field.value));
  copy.addEventListener('click', async () => {
    try {
      await navigator.clipboard.writeText(field.value);
      toast(T('Ссылка скопирована'));
      done();
    } catch (e) {
      toast(errorMessage(e), 'error');
    }
  });
  document.body.append(backdrop);
  copy.focus();
}
