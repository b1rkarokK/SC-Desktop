// «Загрузить трек» (Профиль → Мои треки): one window over the app with the
// file, cover, title, genre and access; progress in the same window. Closing
// it during the upload only hides it: the upload goes on and ends with a toast.
import { listen } from '@tauri-apps/api/event';
import { api, errorMessage, type PickedFile, type Track } from './api';
import { h, toast } from './dom';
import { I, icon } from './icons';
import { T } from './i18n';

const GENRES = [
  'Alternative Rock', 'Ambient', 'Classical', 'Country', 'Dance & EDM', 'Dancehall', 'Deep House', 'Disco',
  'Drum & Bass', 'Dubstep', 'Electronic', 'Folk & Singer-Songwriter', 'Hip-hop & Rap', 'House', 'Indie',
  'Jazz & Blues', 'Latin', 'Metal', 'Phonk', 'Piano', 'Pop', 'R&B & Soul', 'Reggae', 'Reggaeton', 'Rock',
  'Soundtrack', 'Techno', 'Trance', 'Trap', 'Triphop', 'World',
];

const STAGE: Record<string, string> = {
  upload: T('Загрузка'),
  transcode: T('SoundCloud обрабатывает файл'),
  save: T('Сохраняю трек'),
};

function fmtSize(bytes: number): string {
  return bytes >= 1024 * 1024 ? T('{0} МБ', (bytes / 1024 / 1024).toFixed(1).replace('.', ',')) : T('{0} КБ', Math.max(1, Math.round(bytes / 1024)));
}

let busy = false;

/** Picks the audio file first; the window opens only if one was chosen. */
export async function uploadTrack(onDone: (t: Track) => void): Promise<void> {
  if (busy) {
    toast(T('Загрузка уже идёт'));
    return;
  }
  let file: PickedFile | null;
  try {
    file = await api.uploadPick(false);
  } catch (e) {
    toast(errorMessage(e), 'error');
    return;
  }
  if (file) uploadWindow(file, onDone);
}

/** The window for an already chosen file. */
export function uploadWindow(first: PickedFile, onDone: (t: Track) => void): void {
  let file = first;
  let cover: PickedFile | null = null;
  let isPrivate = false;

  const fileName = h('span', { class: 'upl-file-name', dir: 'auto' });
  const showFile = () => (fileName.textContent = `${file.name} · ${fmtSize(file.size)}`);
  showFile();
  const changeFile = h('button', { type: 'button', class: 'btn btn-quiet', text: T('Другой файл') });

  const title = h('input', { class: 'input', placeholder: T('Название'), maxlength: '100', spellcheck: 'false', value: first.title });
  const genre = h('input', { class: 'input', placeholder: T('Например, Hip-hop & Rap'), list: 'upl-genres', maxlength: '60', spellcheck: 'false' });
  const genres = h('datalist', { id: 'upl-genres' }, ...GENRES.map((g) => h('option', { value: g })));

  const coverBox = h('button', { type: 'button', class: 'upl-cover', title: T('Выбрать обложку') });
  const showCover = () =>
    coverBox.replaceChildren(
      icon(cover ? I.check : I.image, 20),
      h('span', { dir: 'auto', text: cover ? cover.name : T('Обложка') }),
    );
  showCover();

  const access = h('div', { class: 'seg', role: 'radiogroup', 'aria-label': T('Доступ') });
  const accessBtns = ([[T('Открытый'), false], [T('Закрытый'), true]] as const).map(([label, value]) => {
    const b = h('button', { type: 'button', class: 'seg-tab', role: 'radio', text: label });
    b.addEventListener('click', () => {
      isPrivate = value;
      renderAccess();
    });
    return b;
  });
  access.append(...accessBtns);
  const renderAccess = () =>
    accessBtns.forEach((b, i) => {
      const on = (i === 1) === isPrivate;
      b.classList.toggle('is-active', on);
      b.setAttribute('aria-checked', String(on));
    });
  renderAccess();

  const stage = h('div', { class: 'muted small' });
  const fill = h('div', { class: 'upd-fill' });
  const progress = h('div', { class: 'upl-progress', hidden: true }, stage, h('div', { class: 'upd-bar' }, fill));
  const err = h('div', { class: 'small err', hidden: true });

  const cancel = h('button', { type: 'button', class: 'btn', text: T('Отмена') });
  const submit = h('button', { type: 'button', class: 'btn btn-primary', text: T('Загрузить') });

  const box = h(
    'div',
    { class: 'upd panel upl', role: 'dialog', 'aria-modal': 'true', 'aria-label': T('Загрузить трек') },
    h('div', { class: 'upd-title', text: T('Загрузить трек') }),
    h(
      'div',
      { class: 'upl-top' },
      coverBox,
      h(
        'div',
        { class: 'upl-fields' },
        h('div', { class: 'upl-file' }, icon(I.track), fileName, changeFile),
        h('label', { class: 'upl-label', text: T('Название') }),
        title,
      ),
    ),
    h(
      'div',
      { class: 'upl-row' },
      h('div', { class: 'upl-grow' }, h('label', { class: 'upl-label', text: T('Жанр') }), genre, genres),
      h('div', { class: 'upl-col' }, h('label', { class: 'upl-label', text: T('Доступ') }), access),
    ),
    progress,
    err,
    h('div', { class: 'upd-actions' }, cancel, submit),
  );
  const backdrop = h('div', { class: 'upd-backdrop' }, box);
  const close = () => backdrop.remove();

  changeFile.addEventListener('click', async () => {
    const f = await api.uploadPick(false).catch((e) => (toast(errorMessage(e), 'error'), null));
    if (!f) return;
    // a title still equal to the old file's name follows the new file
    if (!title.value.trim() || title.value === file.title) title.value = f.title;
    file = f;
    showFile();
  });
  coverBox.addEventListener('click', async () => {
    const f = await api.uploadPick(true).catch((e) => (toast(errorMessage(e), 'error'), null));
    if (!f) return;
    cover = f;
    showCover();
  });
  cancel.addEventListener('click', close);
  backdrop.addEventListener('pointerdown', (ev) => ev.target === backdrop && !busy && close());
  box.addEventListener('keydown', (ev) => ev.key === 'Escape' && close());
  title.addEventListener('input', () => (err.hidden = true));

  submit.addEventListener('click', async () => {
    if (!title.value.trim()) {
      err.textContent = T('Введите название трека');
      err.hidden = false;
      title.focus();
      return;
    }
    busy = true;
    err.hidden = true;
    for (const el of [title, genre, coverBox, changeFile, submit, ...accessBtns]) el.disabled = true;
    cancel.textContent = T('Скрыть');
    progress.hidden = false;
    stage.textContent = T('Подготовка…');
    fill.style.width = '0%';
    const unlisten = await listen<{ stage: string; percent: number }>('upload:progress', (e) => {
      const { stage: s, percent } = e.payload;
      stage.textContent = s === 'save' ? `${STAGE.save}…` : `${STAGE[s] ?? ''} ${percent}%`;
      fill.style.width = `${s === 'save' ? 100 : percent}%`;
    });
    const unwarn = await listen<string>('upload:warning', (e) => toast(e.payload));
    try {
      const t = await api.uploadTrack({
        path: file.path,
        title: title.value.trim(),
        genre: genre.value.trim() || null,
        tags: null,
        private: isPrivate,
        artwork: cover?.path ?? null,
      });
      toast(T('«{0}» загружен{1}', t.title, isPrivate ? T(' закрытым') : ''));
      close();
      onDone(t);
    } catch (e) {
      // still open: show the reason in place; hidden: as a toast
      if (backdrop.isConnected) {
        err.textContent = errorMessage(e);
        err.hidden = false;
        progress.hidden = true;
        for (const el of [title, genre, coverBox, changeFile, submit, ...accessBtns]) el.disabled = false;
        cancel.textContent = T('Отмена');
        submit.textContent = T('Повторить');
      } else {
        toast(T('Загрузка не удалась: {0}', errorMessage(e)), 'error');
      }
    } finally {
      busy = false;
      unlisten();
      unwarn();
    }
  });

  document.body.append(backdrop);
  title.focus();
  title.select();
}
