// «Настройки» (mockup 8): account, theme, Discord, lyrics, network + FAQ, system.
import { invoke } from '@tauri-apps/api/core';
import { api, coverUrl, errorMessage, type AppConfig, type DiscordConfig, type NetMode } from '../api';
import { h, toast } from '../dom';
import { icon, I } from '../icons';
import { onPrefs, prefs, updatePrefs, type Theme } from '../prefs';
import { store } from '../store';
import { lyricsData } from '../lyrics_data';
import { updates } from '../update_dialog';
import { btn, viewHead, type View } from './common';

function toggle(on: boolean, onChange: (v: boolean) => void): HTMLButtonElement {
  const b = h('button', { type: 'button', class: 'toggle', role: 'switch' }, h('span', { class: 'toggle-knob' }));
  const render = (v: boolean) => {
    b.classList.toggle('is-on', v);
    b.setAttribute('aria-checked', String(v));
  };
  render(on);
  b.addEventListener('click', () => {
    const v = b.getAttribute('aria-checked') !== 'true';
    render(v);
    onChange(v);
  });
  return b;
}

function row(label: string, control: HTMLElement, hint?: string): HTMLElement {
  return h('div', { class: 'set-row' }, h('div', { class: 'set-label' }, h('div', { text: label }), hint ? h('div', { class: 'muted small', text: hint }) : null), control);
}

function group(title: string, ...children: (HTMLElement | null)[]): HTMLElement {
  return h('div', { class: 'group panel' }, h('h2', { text: title }), ...children);
}

function faq(title: string, items: [string, string][]): HTMLElement {
  return h(
    'div',
    { class: 'faq' },
    h('div', { class: 'faq-title' }, icon(I.help), h('span', { text: title })),
    ...items.map(([k, v]) => h('p', {}, h('b', { text: `${k}: ` }), v)),
  );
}

export class SettingsView implements View {
  el: HTMLElement;
  private cfg: AppConfig | null = null;
  private account = h('div');
  private discordBox = h('div');
  private lyricsBox = h('div');
  private netBox = h('div');
  private systemBox = h('div');
  private themeBox = h('div');

  constructor() {
    this.el = h(
      'section',
      { class: 'view' },
      viewHead('Настройки'),
      h(
        'div',
        { class: 'view-scroll pad settings' },
        h('div', { class: 'settings-grid' }, this.account, this.themeBox, this.discordBox, this.lyricsBox),
        this.netBox,
        this.systemBox,
      ),
    );
    store.on('auth', () => this.renderAccount());
    onPrefs(() => this.renderTheme());
  }

  show(): void {
    this.renderAccount();
    this.renderTheme();
    this.renderLyrics();
    void this.load();
  }

  private async load(): Promise<void> {
    try {
      this.cfg = await api.configGet();
      this.renderDiscord();
      this.renderNet();
      const sys = await api.systemGet();
      this.renderSystem(sys.autostart, sys.start_minimized);
    } catch (e) {
      toast(errorMessage(e), 'error');
    }
  }

  // ---------------------------------------------------------- account

  private renderAccount(): void {
    const a = store.auth;
    const avatar = h('img', { class: 'set-avatar', alt: '' });
    const src = coverUrl(a.avatar_url, 't67x67');
    if (src) avatar.src = src;
    const loginBtn = btn('Войти через SoundCloud', I.login, true);
    loginBtn.addEventListener('click', () => api.authLogin().catch((e) => toast(errorMessage(e), 'error')));
    const logout = btn('Выйти', null);
    logout.addEventListener('click', async () => {
      try {
        await api.authClear();
        store.setAuth(await api.authStatus());
        toast('Вы вышли из аккаунта');
      } catch (e) {
        toast(errorMessage(e), 'error');
      }
    });
    const cid = h('input', { class: 'input', type: 'password', placeholder: 'client_id', autocomplete: 'off' });
    const tok = h('input', { class: 'input', type: 'password', placeholder: 'oauth_token', autocomplete: 'off' });
    const save = btn('Сохранить и проверить', null);
    save.addEventListener('click', async () => {
      try {
        const st = await api.authSave(cid.value, tok.value);
        cid.value = tok.value = '';
        store.setAuth(st);
        await store.reloadSets();
        toast(`Вход выполнен: ${st.username ?? ''}`);
      } catch (e) {
        toast(errorMessage(e), 'error');
      }
    });
    const manual = h(
      'details',
      { class: 'details' },
      h('summary', { text: 'Ввести токены вручную' }),
      h('p', { class: 'muted small', text: 'soundcloud.com → F12 → Network → запрос к api-v2.soundcloud.com: client_id — в адресе, oauth_token — в заголовке Authorization после «OAuth ».' }),
      cid,
      tok,
      save,
    );
    this.account.replaceChildren(
      group(
        'Аккаунт SoundCloud',
        a.has_credentials
          ? h('div', { class: 'set-user' }, avatar, h('div', {}, h('div', { class: 'strong', text: a.username ?? 'Аккаунт' }), h('div', { class: 'muted small', text: 'Вход выполнен' })))
          : h('p', { class: 'muted small', text: 'Откроется окно soundcloud.com — войдите как обычно. Ключи сохранятся в системном хранилище.' }),
        h('div', { class: 'row-actions' }, a.has_credentials ? logout : loginBtn),
        manual,
      ),
    );
  }

  // ------------------------------------------------------------ theme

  private renderTheme(): void {
    const p = prefs();
    const cards = h('div', { class: 'theme-cards' });
    const themes: [Theme, string][] = [['oled', 'OLED'], ['dark', 'Тёмная'], ['light', 'Светлая']];
    for (const [t, label] of themes) {
      const card = h(
        'button',
        { type: 'button', class: `theme-card theme-${t}${p.theme === t ? ' is-active' : ''}` },
        h('span', { class: 'tc-preview' }, h('span', { class: 'tc-nav' }), h('span', { class: 'tc-l1' }), h('span', { class: 'tc-l2' }), h('span', { class: 'tc-play' })),
        h('span', { class: 'tc-label', text: label }),
      );
      card.addEventListener('click', () => updatePrefs((x) => (x.theme = t)));
      cards.append(card);
    }
    this.themeBox.replaceChildren(
      group('Тема', cards, row('Как в системе', toggle(p.themeSystem, (v) => updatePrefs((x) => (x.themeSystem = v))), 'светлая днём, тёмная/OLED ночью')),
    );
  }

  // ---------------------------------------------------------- discord

  private renderDiscord(): void {
    const d: DiscordConfig = { ...this.cfg!.discord };
    const save = () => api.discordSet(d).catch((e) => toast(errorMessage(e), 'error'));
    this.discordBox.replaceChildren(
      group(
        'Статус в Discord',
        row('Показывать, что я слушаю', toggle(d.enabled, (v) => ((d.enabled = v), void save()))),
        row('Полоса прогресса', toggle(d.progress, (v) => ((d.progress = v), void save())), 'тип «Слушает» с таймером трека'),
        row('Кнопка «Слушать на SoundCloud»', toggle(d.button, (v) => ((d.button = v), void save()))),
        row('На паузе — скрывать статус', toggle(d.hide_on_pause, (v) => ((d.hide_on_pause = v), void save())), 'иначе «на паузе» без таймера'),
        h('p', { class: 'muted small', text: 'Нужен только запущенный Discord на этом компьютере.' }),
      ),
    );
  }

  // ----------------------------------------------------------- lyrics

  private renderLyrics(): void {
    const p = prefs();
    this.lyricsBox.replaceChildren(
      group(
        'Тексты песен',
        row('Синхронный текст (караоке)', toggle(p.syncedLyrics, (v) => updatePrefs((x) => (x.syncedLyrics = v))), 'ищет по очереди: LRCLIB → NetEase → Musixmatch'),
        row('Подсветка по словам', toggle(p.wordHighlight, (v) => updatePrefs((x) => (x.wordHighlight = v))), 'где нет слов — плавно по строке'),
        h('p', { class: 'muted small', text: 'Если синхронного текста нигде нет — показывается обычный текст с Genius. Ничего вводить не нужно, тексты сохраняются в кэш.' }),
        h('div', { class: 'row-actions' }, (() => {
          const b = btn('Искать текст заново', I.refresh);
          b.addEventListener('click', () => lyricsData.reload());
          return b;
        })()),
      ),
    );
  }

  // ---------------------------------------------------------- network

  private renderNet(): void {
    const cfg = this.cfg!;
    let mode: NetMode = cfg.net_mode;
    const proxy = h('input', { class: 'input', placeholder: 'socks5h://127.0.0.1:10808', value: cfg.proxy ?? '', spellcheck: 'false' });
    const result = h('span', { class: 'muted small' });
    const radios = h('div', { class: 'radio-list', role: 'radiogroup' });
    const opts: [NetMode, string, string][] = [
      ['direct', 'Прямое подключение', 'Через системную сеть: если включён «запрет» или VPN — работает через них'],
      ['proxy', 'Прокси', 'SOCKS5 / HTTP — адрес ниже'],
    ];
    for (const [m, label, hint] of opts) {
      const input = h('input', { type: 'radio', name: 'net-mode', value: m });
      input.checked = m === mode;
      input.addEventListener('change', () => {
        mode = m;
        proxy.disabled = mode !== 'proxy';
      });
      radios.append(h('label', { class: 'radio' }, input, h('div', {}, h('div', { text: label }), h('div', { class: 'muted small', text: hint }))));
    }
    proxy.disabled = mode !== 'proxy';
    const check = btn('Проверить', null);
    check.addEventListener('click', async () => {
      result.textContent = 'Проверяем…';
      try {
        const r = await api.netCheck(mode, proxy.value || null);
        result.replaceChildren(r.ok ? icon(I.check) : '', `${r.message} · ${r.ms} мс`);
        result.className = r.ok ? 'small ok' : 'small err';
      } catch (e) {
        result.textContent = errorMessage(e);
        result.className = 'small err';
      }
    });
    const apply = btn('Применить', null, true);
    apply.addEventListener('click', async () => {
      try {
        this.cfg = await api.configSetNetwork(mode, proxy.value || null, cfg.chrome_version);
        toast('Сетевые настройки применены');
      } catch (e) {
        toast(errorMessage(e), 'error');
      }
    });
    this.netBox.replaceChildren(
      h(
        'div',
        { class: 'group panel net-group' },
        h(
          'div',
          { class: 'net-left' },
          h('h2', { text: 'Сеть' }),
          radios,
          h('div', { class: 'row-actions' }, proxy, check),
          result,
          h('div', { class: 'row-actions' }, apply),
        ),
        faq('Как пользоваться прокси', [
          ['Зачем', 'весь трафик приложения пойдёт через указанный адрес — например, через ваш VPN-клиент, если он не включён системно.'],
          ['Где взять адрес', 'в настройках VPN-клиента найдите «локальный SOCKS-порт» (v2rayN — 10808, NekoBox — 2080, Clash — 7890).'],
          ['Что вписать', 'socks5h://127.0.0.1:ПОРТ (h — DNS тоже идёт через прокси). С паролем: socks5://логин:пароль@адрес:порт'],
          ['Проверка', 'нажмите «Проверить». Ошибка 403 — SoundCloud не пускает этот адрес, нужен другой сервер.'],
        ]),
      ),
    );
  }

  // ----------------------------------------------------------- system

  private renderSystem(autostart: boolean, minimized: boolean): void {
    let a = autostart;
    let m = minimized;
    // on failure show the real state again (the switch must not lie)
    const save = () =>
      api.systemSet(a, m).catch(async (e) => {
        toast(errorMessage(e), 'error');
        const s = await api.systemGet();
        this.renderSystem(s.autostart, s.start_minimized);
      });
    const version = h('span', { class: 'muted small' });
    void invoke<string>('app_version').then((v) => (version.textContent = `Версия ${v}`));
    const check = btn('Проверить обновления', I.refresh);
    check.addEventListener('click', async () => {
      check.disabled = true;
      try {
        const r = await updates.check();
        if (r === 'none') toast('У вас последняя версия');
      } catch (e) {
        toast(errorMessage(e), 'error');
      } finally {
        check.disabled = false;
      }
    });
    this.systemBox.replaceChildren(
      group(
        'Система',
        h(
          'div',
          { class: 'two-col' },
          row('Запускать вместе с системой', toggle(a, (v) => ((a = v), void save()))),
          row('При запуске сворачивать в трей', toggle(m, (v) => ((m = v), void save()))),
        ),
        h('div', { class: 'row-actions' }, version, h('div', { class: 'spacer' }), check),
      ),
    );
  }
}
