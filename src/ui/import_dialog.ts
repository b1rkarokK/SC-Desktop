// «Импорт» (Лайки → Плейлисты): a playlist or album link from Yandex Music or
// Spotify, or pasted «Артист - Трек» lines → a new SoundCloud playlist.
// The link is read as soon as it is pasted: name and number of songs show up
// before anything is created.
import { listen } from '@tauri-apps/api/event';
import { api, errorMessage, plural } from './api';
import { h, toast } from './dom';
import { T } from './i18n';

let busy = false;

export function importDialog(onDone: () => void): void {
  if (busy) {
    toast(T('Импорт уже идёт'));
    return;
  }
  const input = h('textarea', {
    class: 'input imp-input',
    rows: '3',
    spellcheck: 'false',
    placeholder: T('Ссылка на плейлист или альбом Яндекс Музыки / Spotify, или строки «Артист - Трек»'),
  });
  const found = h('div', { class: 'muted small' });
  const title = h('input', { class: 'input', placeholder: T('Название плейлиста'), maxlength: '100', spellcheck: 'false' });
  let isPrivate = true;
  const access = h('div', { class: 'seg', role: 'radiogroup', 'aria-label': T('Доступ') });
  const accessBtns = ([[T('Закрытый'), true], [T('Открытый'), false]] as const).map(([label, value]) => {
    const b = h('button', { type: 'button', class: 'seg-tab', role: 'radio', text: label });
    b.addEventListener('click', () => {
      isPrivate = value;
      renderAccess();
    });
    return b;
  });
  access.append(...accessBtns);
  const renderAccess = () => accessBtns.forEach((b, i) => b.classList.toggle('is-active', (i === 0) === isPrivate));
  renderAccess();

  const stage = h('div', { class: 'muted small' });
  const fill = h('div', { class: 'upd-fill' });
  const progress = h('div', { class: 'upl-progress', hidden: true }, stage, h('div', { class: 'upd-bar' }, fill));
  const result = h('div', { class: 'imp-result', hidden: true });
  const cancel = h('button', { type: 'button', class: 'btn', text: T('Отмена') });
  const go = h('button', { type: 'button', class: 'btn btn-primary', text: T('Импортировать'), disabled: true });

  const box = h(
    'div',
    { class: 'upd panel upl', role: 'dialog', 'aria-modal': 'true', 'aria-label': T('Импорт плейлиста') },
    h('div', { class: 'upd-title', text: T('Импорт плейлиста') }),
    input,
    found,
    h(
      'div',
      { class: 'upl-row' },
      h('div', { class: 'upl-grow' }, h('label', { class: 'upl-label', text: T('Название') }), title),
      h('div', { class: 'upl-col' }, h('label', { class: 'upl-label', text: T('Доступ') }), access),
    ),
    progress,
    result,
    h('div', { class: 'upd-actions' }, cancel, go),
  );
  const backdrop = h('div', { class: 'upd-backdrop' }, box);
  const close = () => backdrop.remove();
  cancel.addEventListener('click', close);
  box.addEventListener('keydown', (ev) => ev.key === 'Escape' && !busy && close());

  // read the link while typing (debounced): name + number of songs
  let timer = 0;
  let seq = 0;
  input.addEventListener('input', () => {
    window.clearTimeout(timer);
    go.disabled = true;
    found.textContent = '';
    const value = input.value.trim();
    if (!value) return;
    timer = window.setTimeout(async () => {
      const mine = ++seq;
      found.textContent = T('Читаю…');
      try {
        const [name, n] = await api.importPreview(value);
        if (mine !== seq) return;
        found.textContent = T('{0} · {1} {2}', name, n, plural(n, 'песня', 'песни', 'песен'));
        if (!title.value.trim() || title.dataset.auto === '1') {
          title.value = name;
          title.dataset.auto = '1';
        }
        go.disabled = false;
      } catch (e) {
        if (mine === seq) found.textContent = errorMessage(e);
      }
    }, 500);
  });
  title.addEventListener('input', () => (title.dataset.auto = ''));

  go.addEventListener('click', async () => {
    busy = true;
    for (const el of [input, title, go, ...accessBtns]) el.disabled = true;
    cancel.textContent = T('Скрыть');
    progress.hidden = false;
    stage.textContent = T('Ищу песни на SoundCloud…');
    fill.style.width = '0%';
    const unlisten = await listen<{ done: number; total: number }>('import:progress', (e) => {
      const { done, total } = e.payload;
      stage.textContent = T('Ищу на SoundCloud: {0} из {1}', done, total);
      fill.style.width = `${Math.round((done / Math.max(1, total)) * 100)}%`;
    });
    try {
      const r = await api.importRun(input.value.trim(), title.value.trim(), isPrivate);
      onDone();
      toast(T('Плейлист «{0}» создан: {1} из {2}', r.title, r.found, r.total));
      if (!backdrop.isConnected) return;
      progress.hidden = true;
      result.hidden = false;
      result.replaceChildren(
        h('div', { text: T('Создан плейлист «{0}»: нашлось {1} из {2}.', r.title, r.found, r.total) }),
        ...(r.missing.length
          ? [
              h(
                'details',
                { class: 'details' },
                h('summary', { text: T('Не нашлось на SoundCloud: {0}', r.missing.length) }),
                h('div', { class: 'imp-missing muted small' }, ...r.missing.map((m) => h('div', { dir: 'auto', text: m }))),
              ),
            ]
          : []),
      );
      cancel.textContent = T('Готово');
      cancel.disabled = false;
    } catch (e) {
      if (backdrop.isConnected) {
        progress.hidden = true;
        found.textContent = errorMessage(e);
        for (const el of [input, title, go, ...accessBtns]) el.disabled = false;
        cancel.textContent = T('Отмена');
      } else {
        toast(T('Импорт не удался: {0}', errorMessage(e)), 'error');
      }
    } finally {
      busy = false;
      unlisten();
    }
  });

  document.body.append(backdrop);
  input.focus();
}
