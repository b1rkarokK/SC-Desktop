// "Добавить в плейлист": a second menu with the user's own playlists and
// "Новый плейлист…" (a small name dialog).
import { api, errorMessage, type Track } from './api';
import { openMenu, type MenuItem } from './context_menu';
import { h, toast } from './dom';
import { I } from './icons';
import { T } from './i18n';

function fail(e: unknown): void {
  toast(errorMessage(e), 'error');
}

export async function addToPlaylistMenu(e: MouseEvent, t: Track): Promise<void> {
  let own;
  try {
    own = (await api.libraryPlaylists(false)).filter((p) => p.own && !p.is_album);
  } catch (err) {
    fail(err);
    return;
  }
  const items: MenuItem[] = [
    { label: T('Новый плейлист…'), icon: I.plus, action: () => void createPlaylist(t) },
  ];
  if (own.length) items.push('separator');
  for (const p of own) {
    items.push({
      label: p.title,
      icon: I.playlist,
      action: () => {
        toast(T('Добавляю в «{0}»…', p.title));
        void api.playlistAddTrack(p.id, t.id).then(() => toast(T('Добавлен в «{0}»', p.title)), fail);
      },
    });
  }
  openMenu(e, items);
}

async function createPlaylist(t: Track | null): Promise<void> {
  const title = await askName();
  if (!title) return;
  try {
    await api.playlistCreate(title, t?.id ?? null);
    toast(T('Плейлист «{0}» создан', title));
  } catch (err) {
    fail(err);
  }
}

/** Name of the new playlist; null when cancelled. */
function askName(): Promise<string | null> {
  return new Promise((done) => {
    const input = h('input', { class: 'input', placeholder: T('Название'), maxlength: '100', spellcheck: 'false' });
    const cancel = h('button', { type: 'button', class: 'btn', text: T('Отмена') });
    const ok = h('button', { type: 'button', class: 'btn btn-primary', text: T('Создать') });
    const box = h(
      'div',
      { class: 'upd panel', role: 'dialog', 'aria-modal': 'true', 'aria-label': T('Новый плейлист') },
      h('div', { class: 'upd-title', text: T('Новый плейлист') }),
      h('div', { class: 'muted small', text: T('Будет закрытым, открыть его можно на SoundCloud.') }),
      input,
      h('div', { class: 'upd-actions' }, cancel, ok),
    );
    const backdrop = h('div', { class: 'upd-backdrop' }, box);
    const close = (v: string | null) => {
      backdrop.remove();
      done(v);
    };
    const submit = () => {
      const v = input.value.trim();
      if (v) close(v);
      else input.focus();
    };
    cancel.addEventListener('click', () => close(null));
    ok.addEventListener('click', submit);
    backdrop.addEventListener('pointerdown', (ev) => ev.target === backdrop && close(null));
    input.addEventListener('keydown', (ev) => {
      if (ev.key === 'Enter') submit();
      else if (ev.key === 'Escape') close(null);
    });
    document.body.append(backdrop);
    input.focus();
  });
}
