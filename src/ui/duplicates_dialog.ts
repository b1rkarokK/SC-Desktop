// «Дубли» (Лайки → Треки): one song liked several times (re-uploads by different people). Per group the most
// played upload is kept, the rest are ticked to be unliked: untick what should stay, then remove.
import { api, coverUrl, errorMessage, fmtTime, type Track } from './api';
import { h, toast } from './dom';
import { T } from './i18n';
import { store } from './store';

let busy = false;

export async function duplicatesDialog(): Promise<void> {
  if (busy) return;
  busy = true;
  let groups: Track[][];
  try {
    groups = await api.likesDuplicates();
  } catch (e) {
    busy = false;
    toast(errorMessage(e), 'error');
    return;
  }
  const close = h('button', { type: 'button', class: 'btn', text: T('Закрыть') });
  const go = h('button', { type: 'button', class: 'btn btn-primary', text: T('Убрать выбранные') });
  const checks = new Map<number, HTMLInputElement>();
  const list = h('div', { class: 'dup-list' });
  if (!groups.length) list.append(h('div', { class: 'muted', text: T('Дублей в лайках не нашлось.') }));
  for (const g of groups) {
    const rows = g.map((t, i) => {
      const img = h('img', { class: 'pal-art', alt: '' });
      const src = coverUrl(t.artwork_url, 't67x67');
      if (src) img.src = src;
      const box = h('input', { type: 'checkbox' });
      box.checked = i > 0; // the first (most played) stays
      checks.set(t.id, box);
      box.addEventListener('change', update);
      return h(
        'label',
        { class: 'dup-row' },
        box,
        img,
        h('div', { class: 'pal-text' }, h('div', { class: 'pal-title', dir: 'auto', text: t.title }), h('div', { class: 'muted small', dir: 'auto', text: `${t.artist} · ${fmtTime(t.duration_ms)}${i === 0 ? ' · ' + T('оставить') : ''}` })),
      );
    });
    list.append(h('div', { class: 'dup-group' }, ...rows));
  }
  function update(): void {
    const n = [...checks.values()].filter((c) => c.checked).length;
    go.textContent = n ? T('Убрать выбранные ({0})', n) : T('Убрать выбранные');
    go.disabled = n === 0;
  }
  update();
  const box = h(
    'div',
    { class: 'upd panel upl', role: 'dialog', 'aria-modal': 'true', 'aria-label': T('Дубли в лайках') },
    h('div', { class: 'upd-title', text: T('Дубли в лайках') }),
    h('div', { class: 'muted small', text: groups.length ? T('Одна песня, залитая разными людьми. Отмеченные потеряют лайк, самая прослушанная версия остаётся.') : '' }),
    list,
    h('div', { class: 'upd-actions' }, close, go),
  );
  const backdrop = h('div', { class: 'upd-backdrop' }, box);
  const done = () => {
    backdrop.remove();
    busy = false;
  };
  close.addEventListener('click', done);
  backdrop.addEventListener('pointerdown', (e) => e.target === backdrop && done());
  go.addEventListener('click', async () => {
    const ids = [...checks.entries()].filter(([, c]) => c.checked).map(([id]) => id);
    go.disabled = true;
    const byId = new Map(groups.flat().map((t) => [t.id, t]));
    let n = 0;
    for (const id of ids) {
      const t = byId.get(id);
      if (t) {
        await store.setLiked(t, false);
        n++;
      }
    }
    toast(T('Убрано лайков: {0}', n));
    done();
  });
  document.body.append(backdrop);
}
