// "Вышло обновление" dialog: version, date, changelog, «Позже» / «Обновить».
// Update downloads inside the dialog with progress, then the app restarts.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { errorMessage } from './api';
import { h } from './dom';

interface UpdateInfo {
  version: string;
  current_version: string;
  date: string | null;
  notes: string | null;
}

export class UpdateDialog {
  private backdrop: HTMLDivElement;
  private box: HTMLDivElement;
  private shownVersion: string | null = null;
  private busy = false;

  constructor() {
    this.box = h('div', { class: 'upd panel', role: 'dialog', 'aria-modal': 'true', 'aria-label': 'Обновление' });
    this.backdrop = h('div', { class: 'upd-backdrop' }, this.box);
    this.backdrop.hidden = true;
    document.body.append(this.backdrop);
    document.addEventListener('keydown', (e) => {
      if (e.key === 'Escape' && !this.backdrop.hidden && !this.busy) this.close();
    });
  }

  /**
   * The Rust core checks on start and hourly (also in the tray). The window
   * shows a waiting update right away and any update found while it's open.
   */
  async listen(): Promise<void> {
    await listen<UpdateInfo>('update:available', (e) => this.show(e.payload));
    const pending = await invoke<UpdateInfo | null>('update_pending').catch(() => null);
    if (pending) this.show(pending);
  }

  /** Manual check from Settings — reports errors to the caller. */
  async check(): Promise<'none' | 'shown'> {
    try {
      const info = await invoke<UpdateInfo | null>('update_check');
      if (!info) return 'none';
      this.shownVersion = null;
      this.show(info);
      return 'shown';
    } catch (e) {
      throw new Error(errorMessage(e));
    }
  }

  private show(info: UpdateInfo): void {
    if (this.busy || (this.shownVersion === info.version && !this.backdrop.hidden)) return;
    this.shownVersion = info.version;
    const later = h('button', { type: 'button', class: 'btn', text: 'Позже' });
    const update = h('button', { type: 'button', class: 'btn btn-primary', text: 'Обновить' });
    later.addEventListener('click', () => {
      void invoke('update_dismiss', { version: info.version });
      this.close();
    });
    update.addEventListener('click', () => void this.install(info));
    const date = info.date ? new Date(info.date) : null;
    const when = date && !Number.isNaN(date.getTime())
      ? date.toLocaleDateString('ru-RU', { day: 'numeric', month: 'long', year: 'numeric' })
      : null;
    this.box.replaceChildren(
      h('div', { class: 'upd-title', text: `Вышло обновление ${info.version}` }),
      h('div', { class: 'muted small', text: [when ? `от ${when}` : null, `у вас ${info.current_version}`].filter(Boolean).join(' · ') }),
      h('div', { class: 'upd-sub', text: 'Что изменилось' }),
      renderNotes(info.notes),
      h('div', { class: 'upd-actions' }, later, update),
    );
    this.backdrop.hidden = false;
    update.focus();
  }

  private async install(info: UpdateInfo): Promise<void> {
    this.busy = true;
    const fill = h('div', { class: 'upd-fill' });
    const status = h('div', { class: 'small', text: 'Подключаемся…' });
    this.box.replaceChildren(
      h('div', { class: 'upd-title', text: `Обновление до ${info.version}` }),
      status,
      h('div', { class: 'upd-bar' }, fill),
      h('div', { class: 'muted small', text: 'Не закрывайте программу — после установки она перезапустится сама.' }),
    );
    const unProgress = await listen<{ downloaded: number; total: number | null }>('update:progress', (e) => {
      const { downloaded, total } = e.payload;
      const mb = (n: number) => (n / 1048576).toFixed(1).replace('.', ',');
      if (total) {
        fill.style.width = `${Math.min(100, (downloaded / total) * 100).toFixed(1)}%`;
        status.textContent = `Скачивание: ${mb(downloaded)} из ${mb(total)} МБ`;
      } else {
        fill.classList.add('is-indeterminate');
        status.textContent = `Скачивание: ${mb(downloaded)} МБ`;
      }
    });
    const unInstalling = await listen('update:installing', () => {
      fill.style.width = '100%';
      status.textContent = 'Устанавливаем… программа перезапустится';
    });
    try {
      await invoke('update_install');
    } catch (e) {
      this.busy = false;
      const retry = h('button', { type: 'button', class: 'btn btn-primary', text: 'Повторить' });
      const later = h('button', { type: 'button', class: 'btn', text: 'Позже' });
      retry.addEventListener('click', () => void this.install(info));
      later.addEventListener('click', () => this.close());
      this.box.replaceChildren(
        h('div', { class: 'upd-title', text: 'Не удалось обновиться' }),
        h('div', { class: 'small err', text: errorMessage(e) }),
        h('div', { class: 'upd-actions' }, later, retry),
      );
    } finally {
      unProgress();
      unInstalling();
    }
  }

  private close(): void {
    this.backdrop.hidden = true;
  }
}

export const updates = new UpdateDialog();

/** Release notes from GitHub: "- item" lines become a list, the rest paragraphs. */
function renderNotes(notes: string | null): HTMLElement {
  const box = h('div', { class: 'upd-notes' });
  if (!notes) {
    box.append(h('p', { class: 'muted', text: 'Исправления и улучшения.' }));
    return box;
  }
  let list: HTMLUListElement | null = null;
  for (const raw of notes.split(/\r?\n/)) {
    const line = raw.trim().replace(/^#+\s*/, '').replace(/\*\*/g, '');
    if (!line) {
      list = null;
      continue;
    }
    const item = line.match(/^[-*•]\s+(.*)$/);
    if (item) {
      if (!list) box.append((list = h('ul')));
      list.append(h('li', { text: item[1]! }));
    } else {
      list = null;
      box.append(h('p', { text: line }));
    }
  }
  return box;
}
