// Right lyrics panel (karaoke). Hidden completely with the «T» toggle.
import { api } from './api';
import { h } from './dom';
import { iconButton, I } from './icons';
import { LyricsView } from './lyrics_view';
import { lyricsData, SOURCE_NAMES, type LyricsState } from './lyrics_data';
import { onPrefs, prefs, updatePrefs } from './prefs';

export class LyricsPanel {
  private view = new LyricsView('ly-panel');
  private badge = h('span', { class: 'badge' });
  private footer = h('div', { class: 'ly-foot muted small' });
  private extBtn = iconButton(I.external, 'Открыть на Genius');
  private url: string | null = null;

  constructor(private host: HTMLElement, onFullscreen: () => void) {
    const fs = iconButton(I.fullscreen, 'Полный экран');
    fs.addEventListener('click', onFullscreen);
    this.extBtn.addEventListener('click', () => this.url && void api.openExternal(this.url));
    const hide = iconButton(I.lyrics, 'Скрыть текст');
    hide.classList.add('is-on');
    hide.addEventListener('click', () => updatePrefs((p) => (p.lyricsOpen = false)));
    host.append(
      h('header', { class: 'ly-head' }, h('span', { class: 'ly-label', text: 'Текст' }), this.badge, h('div', { class: 'spacer' }), fs, this.extBtn, hide),
      this.view.el,
      this.footer,
    );
    lyricsData.on((s) => this.render(s));
    onPrefs(() => this.applyPrefs());
    this.applyPrefs();
    this.render(lyricsData.state);
  }

  private applyPrefs(): void {
    const p = prefs();
    document.getElementById('app')?.classList.toggle('lyrics-closed', !p.lyricsOpen);
    this.view.setShown(p.lyricsOpen);
    const hl = !p.syncedLyrics ? 'none' : p.wordHighlight ? 'word' : 'line';
    if (hl !== this.view.highlight || p.fs.offset !== this.view.offset) {
      this.view.highlight = hl;
      this.view.offset = p.fs.offset;
      this.view.rerender();
    }
    lyricsData.setWanted(p.lyricsOpen || document.body.classList.contains('fs-open'));
  }

  private render(s: LyricsState): void {
    this.url = null;
    this.extBtn.hidden = true;
    this.badge.hidden = true;
    this.footer.textContent = '';
    switch (s.kind) {
      case 'idle':
        this.view.setMessage('Ничего не играет.');
        return;
      case 'loading':
        this.view.setMessage('Ищем текст…');
        return;
      case 'error':
        this.view.setMessage(s.message);
        return;
      case 'ready': {
        const l = s.lyrics;
        if (!l.found) {
          this.view.setMessage('Текст не найден ни в одном источнике.');
          return;
        }
        this.view.setLyrics(l);
        this.badge.hidden = false;
        this.badge.textContent = l.word_level ? 'по словам' : l.synced ? 'синхронный' : 'без таймингов';
        this.url = l.url;
        this.extBtn.hidden = !l.url;
        this.footer.textContent = `Источник: ${SOURCE_NAMES[l.source ?? ''] ?? l.source}${l.word_level ? ' · по словам' : l.synced ? ' · по строкам' : ''}`;
      }
    }
  }

  get element(): HTMLElement {
    return this.host;
  }
}
