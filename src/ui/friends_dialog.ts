// «Найти людей»: search SoundCloud users and follow / unfollow them without leaving the app.
// The people you follow are the "friends" of the app: you invite them to rooms and shared playlists
// and see their open rooms.
import { api, coverUrl, errorMessage, fmtCount, type User } from './api';
import { h, toast } from './dom';
import { T } from './i18n';
import { store } from './store';

export function findPeople(onChange: () => void): void {
  const input = h('input', { type: 'search', class: 'input', placeholder: T('Имя пользователя на SoundCloud'), 'aria-label': T('Найти людей') });
  const list = h('div', { class: 'fd-list' });
  const close = h('button', { type: 'button', class: 'btn btn-primary', text: T('Готово') });
  const box = h('div', { class: 'upd panel fd', role: 'dialog', 'aria-modal': 'true', 'aria-label': T('Найти людей') }, h('div', { class: 'upd-title', text: T('Найти людей') }), input, list, h('div', { class: 'upd-actions' }, close));
  const backdrop = h('div', { class: 'upd-backdrop' }, box);
  let seq = 0;
  let changed = false;

  const done = () => {
    document.removeEventListener('keydown', onKey, true);
    backdrop.remove();
    if (changed) onChange();
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'Escape') done();
  };

  const row = (u: User) => {
    const img = h('img', { class: 'row-cover is-round', alt: '' });
    const src = coverUrl(u.avatar_url, 't67x67');
    if (src) img.src = src;
    const follow = h('button', { type: 'button', class: 'btn' });
    const paint = () => {
      const on = store.following.has(u.id);
      follow.textContent = on ? T('Отписаться') : T('Подписаться');
      follow.classList.toggle('btn-primary', !on);
    };
    paint();
    follow.addEventListener('click', async () => {
      follow.disabled = true;
      await store.setFollow(u, !store.following.has(u.id));
      changed = true;
      follow.disabled = false;
      paint();
    });
    const sub = u.followers_count != null ? T('{0} подписчиков', fmtCount(u.followers_count)) : '';
    return h('div', { class: 'fd-row' }, img, h('div', { class: 'row-main' }, h('div', { class: 'row-title', dir: 'auto', text: u.username }), h('div', { class: 'row-artist muted small', dir: 'auto', text: sub })), follow);
  };

  const search = async () => {
    const q = input.value.trim();
    const my = ++seq;
    if (!q) {
      list.replaceChildren();
      return;
    }
    try {
      const found = (await api.searchUsers(q)).slice(0, 20);
      if (my !== seq) return;
      list.replaceChildren(...(found.length ? found.map(row) : [h('p', { class: 'muted small', text: T('Ничего не нашлось') })]));
    } catch (e) {
      if (my === seq) toast(errorMessage(e), 'error');
    }
  };

  let timer = 0;
  input.addEventListener('input', () => {
    window.clearTimeout(timer);
    timer = window.setTimeout(() => void search(), 300);
  });
  close.addEventListener('click', done);
  backdrop.addEventListener('pointerdown', (e) => e.target === backdrop && done());
  document.addEventListener('keydown', onKey, true);
  document.body.append(backdrop);
  input.focus();
}
