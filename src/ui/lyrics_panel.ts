// Right lyrics panel (karaoke). Hidden completely with the «T» toggle.
import { api } from './api';
import { h } from './dom';
import { iconButton, I } from './icons';
import { LyricsView } from './lyrics_view';
import { lyricsData, SOURCE_NAMES, type LyricsState } from './lyrics_data';
import { lyricsMenu } from './menus';
import { onPrefs, prefs, updatePrefs } from './prefs';
import { T, currentLang } from './i18n';

export class LyricsPanel {
  private view = new LyricsView('ly-panel');
  private badge = h('span', { class: 'badge' });
  private footer = h('div', { class: 'ly-foot muted small' });
  private nudgeBox = h('div', { class: 'ly-nudge muted small' });
  private extBtn = iconButton(I.external, T('Открыть на Genius'));
  private url: string | null = null;
  private dragWidth = 0;
  private trBtn!: HTMLButtonElement;
  /** which track the shown translation is for (a stale answer is dropped) */
  private trFor = 0;
  private trOn = false;

  constructor(private host: HTMLElement, onFullscreen: () => void) {
    const fs = iconButton(I.fullscreen, T('Полный экран'));
    fs.addEventListener('click', onFullscreen);
    this.extBtn.addEventListener('click', () => this.url && void api.openExternal(this.url));
    const tr = iconButton(I.translate, T('Перевод строк'));
    tr.addEventListener('click', () => updatePrefs((p) => (p.lyricsTranslate = !p.lyricsTranslate)));
    this.trBtn = tr;
    const hide = iconButton(I.lyrics, T('Скрыть текст'));
    hide.classList.add('is-on');
    hide.addEventListener('click', () => updatePrefs((p) => (p.lyricsOpen = false)));
    const sizeBtn = (label: string, title: string, delta: number) => {
      const b = h('button', { type: 'button', class: 'btn-icon ly-size-btn', title, 'aria-label': title, text: label });
      b.addEventListener('click', () => updatePrefs((p) => (p.lyricsSize = Math.min(40, Math.max(11, (p.lyricsSize ?? 14) + delta)))));
      return b;
    };
    // the left edge drags: the panel grows to the left, the page gets narrower
    const grip = h('div', { class: 'ly-grip', title: T('Потяните, чтобы изменить ширину') });
    grip.addEventListener('pointerdown', (e) => {
      e.preventDefault();
      grip.setPointerCapture(e.pointerId);
      const move = (ev: PointerEvent) => {
        const w = Math.round(Math.min(window.innerWidth * 0.7, Math.max(240, window.innerWidth - ev.clientX)));
        document.documentElement.style.setProperty('--ly-w', w + 'px');
        this.dragWidth = w;
      };
      const up = () => {
        grip.removeEventListener('pointermove', move);
        grip.removeEventListener('pointerup', up);
        grip.removeEventListener('pointercancel', up);
        if (this.dragWidth) {
          const w = this.dragWidth;
          updatePrefs((p) => (p.lyricsWidth = w));
        }
      };
      grip.addEventListener('pointermove', move);
      grip.addEventListener('pointerup', up);
      grip.addEventListener('pointercancel', up);
    });
    grip.addEventListener('dblclick', () => updatePrefs((p) => (p.lyricsWidth = 320)));
    host.append(
      grip,
      h('header', { class: 'ly-head' }, h('span', { class: 'ly-label', text: T('Текст') }), this.badge, h('div', { class: 'spacer' }), sizeBtn('A−', T('Текст меньше'), -2), sizeBtn('A+', T('Текст больше'), 2), tr, fs, this.extBtn, hide),
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
    document.documentElement.style.setProperty('--ly-w', (p.lyricsWidth ?? 320) + 'px');
    this.view.el.style.fontSize = (p.lyricsSize ?? 14) + 'px';
    this.trBtn?.classList.toggle('is-on', !!p.lyricsTranslate);
    if (this.trBtn && !!p.lyricsTranslate !== this.trOn) {
      // only when the button was pressed (not on every other pref): show or drop the translation of what is on screen
      this.trOn = !!p.lyricsTranslate;
      const s = lyricsData.state;
      if (s.kind === 'ready' && s.lyrics.found) this.translate(s.track.id, s.lyrics.lines.map((x) => x.text));
    }
    this.view.setShown(p.lyricsOpen);
    const hl = !p.syncedLyrics ? 'none' : p.wordHighlight ? 'word' : 'line';
    if (hl !== this.view.highlight || p.fs.offset !== this.view.offset) {
      this.view.highlight = hl;
      this.view.offset = p.fs.offset;
      this.view.rerender();
    }
    lyricsData.setWanted(p.lyricsOpen || document.body.classList.contains('fs-open'));
  }

  /** The translation under the lines, if the button is on (kept per track by the core). */
  private translate(trackId: number, lines: string[]): void {
    if (!prefs().lyricsTranslate) {
      this.trFor = 0;
      this.view.setTranslations(null);
      return;
    }
    this.trFor = trackId;
    void api.lyricsTranslate(trackId, lines, currentLang()).then(
      (t) => this.trFor === trackId && this.view.setTranslations(t),
      () => this.trFor === trackId && this.view.setTranslations(null),
    );
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
      case 'hearing':
        this.view.setMessage(T('По названию текста нет. Слушаю песню, чтобы узнать её по словам…'));
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
        this.translate(s.track.id, l.lines.map((x) => x.text));
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
