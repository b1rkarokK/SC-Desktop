// Edit mode of an own playlist: drag rows by the handle, ✕ removes a track;
// «Сохранить» sends the new order to SoundCloud, «Отмена» puts it back.
import { api, coverUrl, errorMessage, type Track } from './api';
import { h, toast } from './dom';
import { I, icon, iconButton } from './icons';
import { T } from './i18n';
import { makeSortable } from './sortable';

/** `onDone(tracks)`: saved with this order; `onCancel`: left unchanged. */
export function playlistEditor(playlistId: number, original: Track[], onDone: (tracks: Track[]) => void, onCancel: () => void): HTMLElement {
  let tracks = [...original];
  const list = h('div', { class: 'pl-edit-list' });
  const save = h('button', { type: 'button', class: 'btn btn-primary', text: T('Сохранить') });
  const cancel = h('button', { type: 'button', class: 'btn', text: T('Отмена') });
  const note = h('span', { class: 'muted small' });

  const changed = () => tracks.length !== original.length || tracks.some((t, i) => t.id !== original[i]?.id);
  const render = () => {
    list.replaceChildren(
      ...tracks.map((t, i) => {
        const img = h('img', { class: 'q-cover', alt: '', loading: 'lazy' });
        const src = coverUrl(t.artwork_url, 't67x67');
        if (src) img.src = src;
        const rm = iconButton(I.close, T('Убрать из плейлиста'));
        rm.addEventListener('click', () => {
          tracks.splice(i, 1);
          render();
        });
        return h(
          'div',
          { class: 'q-row is-next pl-edit-row' },
          h('span', { class: 'drag-handle', title: T('Перетащите, чтобы изменить порядок') }, icon(I.grip)),
          img,
          h('div', { class: 'q-meta' }, h('div', { class: 'q-title', dir: 'auto', text: t.title }), h('div', { class: 'q-artist', dir: 'auto', text: t.artist })),
          rm,
        );
      }),
    );
    const removed = original.length - tracks.length;
    note.textContent = changed() ? (removed > 0 ? T('Изменено, убрано: {0}', removed) : T('Порядок изменён')) : '';
    save.disabled = !changed();
  };
  makeSortable(list, {
    rowSelector: '.pl-edit-row',
    onMove: (from, to) => {
      const [t] = tracks.splice(from, 1);
      if (t) tracks.splice(to, 0, t);
      render();
    },
  });
  save.addEventListener('click', async () => {
    save.disabled = cancel.disabled = true;
    save.textContent = T('Сохраняю…');
    try {
      await api.playlistSetTracks(playlistId, tracks.map((t) => t.id));
      toast(T('Плейлист сохранён'));
      onDone(tracks);
    } catch (e) {
      toast(errorMessage(e), 'error');
      save.disabled = cancel.disabled = false;
      save.textContent = T('Сохранить');
    }
  });
  cancel.addEventListener('click', onCancel);
  render();
  return h('div', { class: 'pl-edit' }, h('div', { class: 'pl-edit-bar' }, note, h('div', { class: 'spacer' }), cancel, save), list);
}
