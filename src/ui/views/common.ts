// Shared bits for views: header, static track lists, empty/loading states.
import { api, errorMessage, type Track } from '../api';
import { h, toast } from '../dom';
import { icon, I } from '../icons';
import { router } from '../router';
import { store } from '../store';
import { ROW_HEIGHT, trackRowRenderer } from '../track_row';
import { T } from '../i18n';

export interface View {
  el: HTMLElement;
  show?(): void;
}

export function viewHead(title: string | null, ...right: (HTMLElement | null)[]): HTMLElement {
  const head = h('header', { class: 'view-head' });
  if (router.canGoBack) {
    const back = h('button', { type: 'button', class: 'back-btn' }, icon(I.back), h('span', { text: T('Назад') }));
    back.addEventListener('click', () => router.back());
    head.append(back);
  }
  if (title) head.append(h('h1', { text: title }));
  head.append(h('div', { class: 'spacer' }));
  for (const r of right) if (r) head.append(r);
  return head;
}

export function sectionTitle(text: string, right?: HTMLElement): HTMLElement {
  return h('div', { class: 'section-title' }, h('span', { text }), h('div', { class: 'spacer' }), right ?? null);
}

export function btn(label: string, ic: Parameters<typeof icon>[0] | null, primary = false): HTMLButtonElement {
  return h('button', { type: 'button', class: primary ? 'btn btn-primary' : 'btn' }, ic ? icon(ic) : null, h('span', { text: label }));
}

export function emptyState(text: string): HTMLElement {
  return h('div', { class: 'empty', text });
}

/**
 * Non-virtual track list (pages with ≤ a few hundred rows). Click / Enter
 * plays the list from that row; re-renders on player/like changes.
 */
export function staticTrackList(tracks: Track[], onPlay?: () => void): HTMLElement {
  const r = trackRowRenderer();
  const box = h('div', { class: 'track-list', role: 'list' });
  box.style.minHeight = `${tracks.length * ROW_HEIGHT}px`;
  const rows = tracks.map((t, i) => {
    const row = r.create();
    row.dataset.index = String(i);
    row.tabIndex = 0;
    row.setAttribute('role', 'listitem');
    r.update(row, t, i);
    box.append(row);
    return row;
  });
  const play = (i: number) =>
    api.playTracks(tracks, i).then(() => onPlay?.(), (e) => toast(errorMessage(e), 'error'));
  box.addEventListener('click', (e) => {
    const row = (e.target as HTMLElement).closest('[data-index]') as HTMLElement | null;
    if (row) void play(Number(row.dataset.index));
  });
  box.addEventListener('keydown', (e) => {
    const row = (e.target as HTMLElement).closest('[data-index]') as HTMLElement | null;
    if (row && e.key === 'Enter') void play(Number(row.dataset.index));
  });
  const refresh = () => {
    if (!box.isConnected) return;
    rows.forEach((row, i) => r.update(row, tracks[i], i));
  };
  store.on('player', refresh);
  store.on('likes', refresh);
  store.on('dislikes', refresh);
  return box;
}
