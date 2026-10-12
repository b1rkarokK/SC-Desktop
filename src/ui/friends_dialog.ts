// «Добавить друга»: search SoundCloud users by name and send a friend request. Friends are the app's own
// list (they accept, then see each other and can send invitations), not SoundCloud followers.
import { api, coverUrl, errorMessage, fmtCount, type FriendsView, type User } from './api';
import { h, toast } from './dom';
import { T } from './i18n';

export function findPeople(onChange: () => void): void {
  const input = h('input', { type: 'search', class: 'input', placeholder: T('Имя пользователя на SoundCloud'), 'aria-label': T('Добавить друга') });
  const list = h('div', { class: 'fd-list' });
  const close = h('button', { type: 'button', class: 'btn btn-primary', text: T('Готово') });
  const box = h('div', { class: 'upd panel fd', role: 'dialog', 'aria-modal': 'true', 'aria-label': T('Добавить друга') }, h('div', { class: 'upd-title', text: T('Добавить друга') }), input, h('p', { class: 'muted small', text: T('Человек получит запрос в программе и увидит его, как только откроет SC Desk. Дружба начнётся, когда он согласится.') }), list, h('div', { class: 'upd-actions' }, close));
  const backdrop = h('div', { class: 'upd-backdrop' }, box);
  let seq = 0;
  let changed = false;
  let mine: FriendsView = { friends: [], incoming: [], outgoing: [], invites: [] };

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
    const action = h('button', { type: 'button', class: 'btn' });
    const state = () => (mine.friends.some((f) => f.id === u.id) ? 'friend' : mine.outgoing.some((f) => f.id === u.id) ? 'sent' : mine.incoming.some((f) => f.id === u.id) ? 'incoming' : 'none');
    const paint = () => {
      const s = state();
      action.textContent = { friend: T('В друзьях'), sent: T('Запрос отправлен'), incoming: T('Принять запрос'), none: T('Добавить в друзья') }[s];
      action.disabled = s === 'friend' || s === 'sent';
      action.classList.toggle('btn-primary', s === 'none' || s === 'incoming');
    };
    paint();
    action.addEventListener('click', async () => {
      action.disabled = true;
      try {
        if (state() === 'incoming') await api.friendAccept(u.id);
        else await api.friendRequest(u);
        changed = true;
        mine = await api.friendsList();
      } catch (e) {
        toast(errorMessage(e), 'error');
      }
      paint();
    });
    const sub = u.followers_count != null ? T('{0} подписчиков', fmtCount(u.followers_count)) : '';
    return h('div', { class: 'fd-row' }, img, h('div', { class: 'row-main' }, h('div', { class: 'row-title', dir: 'auto', text: u.username }), h('div', { class: 'row-artist muted small', dir: 'auto', text: sub })), action);
  };

  const search = async () => {
    const q = input.value.trim();
    const my = ++seq;
    if (!q) {
      list.replaceChildren();
      return;
    }
    try {
      mine = await api.friendsList();
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
