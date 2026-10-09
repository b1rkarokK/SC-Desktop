// «Профиль» (avatar at the bottom of the nav): downloaded files and likes
// waiting to reach SoundCloud.
import { listen } from '@tauri-apps/api/event';
import { api, coverUrl, errorMessage, fmtCount, plural, type Track, type User } from '../api';
import { h, toast } from '../dom';
import { I, iconButton } from '../icons';
import type { ProfileTab } from '../router';
import { SegTabs } from '../seg_tabs';
import { store } from '../store';
import { btn, emptyState, viewHead, type View } from './common';

function row(t: Track, note: string | null, ...actions: HTMLElement[]): HTMLElement {
  return line(t.artwork_url, t.title, t.artist, note, ...actions);
}

function userRow(u: User, note: string): HTMLElement {
  const r = line(u.avatar_url, u.username, 'артист', note);
  r.querySelector('.row-cover')?.classList.add('is-round');
  return r;
}

function line(cover: string | null | undefined, title: string, sub: string, note: string | null, ...actions: HTMLElement[]): HTMLElement {
  const img = h('img', { class: 'row-cover', alt: '' });
  const src = coverUrl(cover, 't67x67');
  if (src) img.src = src;
  return h(
    'div',
    { class: 'profile-row' },
    img,
    h('div', { class: 'row-main' }, h('div', { class: 'row-title', dir: 'auto', text: title }), h('div', { class: 'row-artist', dir: 'auto', text: sub })),
    note ? h('span', { class: 'muted small', text: note }) : h('span'),
    h('div', { class: 'profile-actions' }, ...actions),
  );
}

export class ProfileView implements View {
  el: HTMLElement;
  private tabs: SegTabs<ProfileTab>;
  private body = h('div', { class: 'view-scroll pad' });
  private action = h('div', { class: 'row-actions' });

  constructor() {
    this.tabs = new SegTabs<ProfileTab>(
      [
        { key: 'downloads', label: 'Скачанные' },
        { key: 'queue', label: 'В очереди' },
      ],
      'downloads',
      () => void this.render(),
    );
    this.el = h('section', { class: 'view' }, viewHead('Профиль', this.action), h('div', { class: 'tabs-row' }, this.tabs.el), this.body);
    const refresh = () => {
      void this.counts();
      if (this.el.isConnected) void this.render();
    };
    void listen('likes:queue', refresh);
    void listen('downloads:changed', refresh);
  }

  show(tab?: ProfileTab): void {
    if (tab) this.tabs.select(tab);
    void this.counts();
    void this.render();
  }

  private async counts(): Promise<void> {
    const [d, q, f] = await Promise.all([
      api.downloads().catch(() => []),
      api.likesPending().catch(() => []),
      api.followsPending().catch(() => []),
    ]);
    const n = q.length + f.length;
    this.tabs.setCount('downloads', d.length ? fmtCount(d.length) : '');
    this.tabs.setCount('queue', n ? fmtCount(n) : '');
  }

  private async render(): Promise<void> {
    if (this.tabs.value === 'downloads') await this.renderDownloads();
    else await this.renderQueue();
  }

  private async renderDownloads(): Promise<void> {
    const open = btn('Открыть папку', I.folder);
    open.addEventListener('click', () => void api.downloadsOpen().catch((e) => toast(errorMessage(e), 'error')));
    this.action.replaceChildren(open);
    let list;
    let dir = '';
    try {
      [list, dir] = await Promise.all([api.downloads(), api.downloadsDir()]);
    } catch (e) {
      toast(errorMessage(e), 'error');
      return;
    }
    if (this.tabs.value !== 'downloads') return;
    const change = h('button', { type: 'button', class: 'btn btn-quiet', text: 'Изменить' });
    change.addEventListener('click', async () => {
      try {
        await api.downloadsPickDir();
        void this.render();
      } catch (e) {
        toast(errorMessage(e), 'error');
      }
    });
    const folderLine = h(
      'div',
      { class: 'profile-folder' },
      h('span', { class: 'muted small', text: 'Сохранять в:' }),
      h('span', { class: 'small profile-path', title: dir, text: dir }),
      change,
    );
    if (!list.length) {
      this.body.replaceChildren(folderLine, emptyState('Скачанных треков пока нет. Нажмите правой кнопкой на трек и выберите «Скачать». Скачанные треки играют без интернета.'));
      return;
    }
    const tracks = list.map((d) => d.track);
    this.body.replaceChildren(
      folderLine,
      ...list.map((d, i) => {
        const show = iconButton(I.folder, 'Показать в папке');
        show.addEventListener('click', (e) => {
          e.stopPropagation();
          void api.downloadsOpen(d.path).catch((err) => toast(errorMessage(err), 'error'));
        });
        const rm = iconButton(I.close, 'Удалить файл');
        rm.addEventListener('click', async (e) => {
          e.stopPropagation();
          await api.downloadRemove(d.track.id).catch((err) => toast(errorMessage(err), 'error'));
        });
        const r = row(d.track, d.path.toLowerCase().endsWith('.m4a') ? 'M4A' : 'MP3', show, rm);
        r.classList.toggle('is-current', store.currentId() === d.track.id);
        r.addEventListener('click', () => void api.playTracks(tracks, i).catch((err) => toast(errorMessage(err), 'error')));
        return r;
      }),
    );
  }

  private async renderQueue(): Promise<void> {
    const send = btn('Отправить сейчас', I.refresh);
    send.addEventListener('click', async () => {
      send.disabled = true;
      try {
        const left = await api.likesPendingFlush();
        toast(left ? `SoundCloud пока не принял: ${left}. Попробую позже сам.` : 'Все лайки отправлены');
      } catch (e) {
        toast(errorMessage(e), 'error');
      } finally {
        send.disabled = false;
      }
    });
    let list, follows;
    try {
      [list, follows] = await Promise.all([api.likesPending(), api.followsPending()]);
    } catch (e) {
      toast(errorMessage(e), 'error');
      return;
    }
    if (this.tabs.value !== 'queue') return;
    const n = list.length + follows.length;
    this.action.replaceChildren(n ? send : h('span'));
    if (!n) {
      this.body.replaceChildren(emptyState('Очередь пуста: все лайки и подписки уже на SoundCloud.'));
      return;
    }
    this.body.replaceChildren(
      h('p', {
        class: 'muted small profile-note',
        text: `SoundCloud временно не принимает ${n} ${plural(n, 'действие', 'действия', 'действий')}. В программе они уже применены, а на сайт уйдут сами, даже после перезапуска.`,
      }),
      ...list.map((p) => row(p.track, p.liked ? 'лайк' : 'убрать лайк')),
      ...follows.map((f) => userRow(f.user, f.follow ? 'подписка' : 'отписка')),
    );
  }
}
