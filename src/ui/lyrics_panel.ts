// Right lyrics panel (karaoke). Hidden completely with the «T» toggle.
import { api } from './api';
import { h } from './dom';
import { iconButton, I } from './icons';
import { LyricsView } from './lyrics_view';
import { lyricsData, SOURCE_NAMES, type LyricsState } from './lyrics_data';
import { lyricsMenu } from './menus';
import { onPrefs, prefs, updatePrefs } from './prefs';
import { T } from './i18n';

export class LyricsPanel {
  private view = new LyricsView('ly-panel');
  private badge = h('span', { class: 'badge' });
  private footer = h('div', { class: 'ly-foot muted small' });
  private nudgeBox = h('div', { class: 'ly-nudge muted small' });
  private extBtn = iconButton(I.external, T('Открыть на Genius'));
  private url: string | null = null;

  constructor(private host: HTMLElement, onFullscreen: () => void) {
    const fs = iconButton(I.fullscreen, T('Полный экран'));
    fs.addEventListener('click', onFullscreen);
    this.extBtn.addEventListener('click', () => this.url && void api.openExternal(this.url));
    const hide = iconButton(I.lyrics, T('Скрыть текст'));
    hide.classList.add('is-on');
    hide.addEventListener('click', () => updatePrefs((p) => (p.lyricsOpen = false)));
    host.append(
      h('header', { class: 'ly-head' }, h('span', { class: 'ly-label', text: T('Текст') }), this.badge, h('div', { class: 'spacer' }), fs, this.extBtn, hide),
      this.view.el,
      this.nudgeBox,
      this.footer,
    );
    this.view.el.addEventListener('contextmenu', (e) => {
      const s = lyricsData.state;
      const l = s.kind === 'ready' && s.lyrics.found ? s.lyrics : null;
      lyricsMenu(e, l ? l.lines.map((x) => x.text).join('\n') : null, l?.url ?? null, () => lyricsData.reload());
    });
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
    this.nudgeBox.replaceChildren();
    switch (s.kind) {
      case 'idle':
        this.view.setMessage(T('Ничего не играет.'));
        return;
      case 'loading':
        this.view.setMessage(T('Ищем текст…'));
        return;
      case 'error':
        this.view.setMessage(s.message);
        return;
      case 'ready': {
        const l = s.lyrics;
        if (!l.found) {
          this.view.setMessage(T('Текст не найден ни в одном источнике.'));
          return;
        }
        this.view.setLyrics(l);
        this.badge.hidden = false;
        this.badge.textContent = l.word_level ? T('по словам') : l.synced ? T('синхронный') : T('без таймингов');
        this.url = l.url;
        this.extBtn.hidden = !l.url;
        this.footer.textContent = T('Источник: {0}{1}', SOURCE_NAMES[l.source ?? ''] ?? l.source, l.original ? T(' · текст оригинала') : l.word_level ? T(' · по словам') : l.synced ? T(' · по строкам') : '');
        if (l.synced && l.original && l.scaled_by) this.footer.textContent += ' · ' + T('тайминги оригинала пересчитаны под темп: ×{0}', l.scaled_by.toFixed(2).replace('.', ','));
        if (l.synced && !l.original && l.length_off_ms) this.footer.textContent += ' · ' + T('тайминги могут быть сдвинуты: версия отличается на {0} с', Math.round(l.length_off_ms / 1000));
        if (l.synced) this.renderNudge(s.track.id);
      }
    }
  }

  /** "Text is early / late": a nudge of a quarter of a second each press, kept for this track. */
  private renderNudge(id: number): void {
    const { auto, nudge } = lyricsData.info(id);
    const sec = (ms: number) => `${ms > 0 ? '+' : ms < 0 ? '−' : ''}${(Math.abs(ms) / 1000).toFixed(2).replace('.', ',')} ${T('с')}`;
    const mk = (label: string, title: string, delta: number) => {
      const b = h('button', { type: 'button', class: 'btn-icon ly-nudge-btn', title, 'aria-label': title, text: label });
      b.addEventListener('click', () => lyricsData.nudge(id, delta));
      return b;
    };
    const value = h('span', { class: 'ly-nudge-val', text: nudge ? sec(nudge) : T('сдвиг текста') });
    const reset = mk('↺', T('Сбросить сдвиг'), 0);
    reset.hidden = nudge === 0;
    const note = auto ? h('span', { class: 'ly-nudge-auto', title: T('Подстроено по звуку'), text: T('по звуку {0}', sec(auto)) }) : null;
    this.nudgeBox.replaceChildren(mk('−', T('Текст спешит: сдвинуть позже'), -250), value, mk('+', T('Текст отстаёт: сдвинуть раньше'), 250), reset, ...(note ? [note] : []));
  }

  get element(): HTMLElement {
    return this.host;
  }
}
