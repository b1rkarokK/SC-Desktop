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
import { currentLang, setLang, T, type Lang } from '../i18n';

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

/** A settings section: title + rows, no box — sections are separated by space. */
function group(title: string, ...children: (HTMLElement | null)[]): HTMLElement {
  return h('section', { class: 'set-section' }, h('h2', { text: title }), ...children);
}

/** Collapsible help: one line «? title», details on click. */
function faq(title: string, items: [string, string][]): HTMLElement {
  return h(
    'details',
    { class: 'faq' },
    h('summary', { class: 'faq-title' }, icon(I.help), h('span', { text: title })),
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
  private langBox = h('div');

  constructor() {
    this.el = h(
      'section',
      { class: 'view' },
      viewHead(T('Настройки')),
      h(
        'div',
        { class: 'view-scroll pad' },
        h('div', { class: 'settings' }, this.account, this.themeBox, this.langBox, this.discordBox, this.lyricsBox, this.netBox, this.systemBox),
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
      this.renderSystem(sys.autostart, sys.start_minimized, sys.fast_protected, sys.notify_new);
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
    const loginBtn = btn(T('Войти через SoundCloud'), I.login, true);
    loginBtn.addEventListener('click', () => api.authLogin().catch((e) => toast(errorMessage(e), 'error')));
    const logout = btn(T('Выйти'), null);
    logout.addEventListener('click', async () => {
      try {
        await api.authClear();
        store.setAuth(await api.authStatus());
        toast(T('Вы вышли из аккаунта'));
      } catch (e) {
        toast(errorMessage(e), 'error');
      }
    });
    const cid = h('input', { class: 'input', type: 'password', placeholder: 'client_id', autocomplete: 'off' });
    const tok = h('input', { class: 'input', type: 'password', placeholder: 'oauth_token', autocomplete: 'off' });
    const save = btn(T('Сохранить и проверить'), null);
    save.addEventListener('click', async () => {
      try {
        const st = await api.authSave(cid.value, tok.value);
        cid.value = tok.value = '';
        store.setAuth(st);
        await store.reloadSets();
        toast(T('Вход выполнен: {0}', st.username ?? ''));
      } catch (e) {
        toast(errorMessage(e), 'error');
      }
    });
    const manual = h(
      'details',
      { class: 'details' },
      h('summary', { text: T('Ввести токены вручную') }),
      h('p', { class: 'muted small', text: T('soundcloud.com → F12 → Network → запрос к api-v2.soundcloud.com: client_id — в адресе, oauth_token — в заголовке Authorization после «OAuth ».') }),
      cid,
      tok,
      save,
    );
    this.account.replaceChildren(
      group(
        T('Аккаунт SoundCloud'),
        a.has_credentials
          ? h('div', { class: 'set-user' }, avatar, h('div', {}, h('div', { class: 'strong', text: a.username ?? T('Аккаунт') }), h('div', { class: 'muted small', text: T('Вход выполнен') })))
          : h('p', { class: 'muted small', text: T('Откроется окно soundcloud.com — войдите как обычно. Ключи сохранятся в системном хранилище.') }),
        h('div', { class: 'row-actions' }, a.has_credentials ? logout : loginBtn),
        manual,
      ),
    );
  }

  // ------------------------------------------------------------ theme

  private renderTheme(): void {
    const p = prefs();
    const cards = h('div', { class: 'theme-cards' });
    const themes: [Theme, string][] = [['oled', 'OLED'], ['dark', T('Тёмная')], ['light', T('Светлая')]];
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
    // language: both names in their own language, so either reader finds it
    const langs = h('div', { class: 'seg', role: 'radiogroup', 'aria-label': 'Язык / Language' });
    for (const [l, label] of [['ru', 'Русский'], ['en', 'English']] as [Lang, string][]) {
      const b = h('button', { type: 'button', class: `seg-tab${currentLang() === l ? ' is-active' : ''}`, role: 'radio', 'aria-checked': String(currentLang() === l), text: label });
      b.addEventListener('click', async () => {
        if (currentLang() === l) return;
        setLang(l);
        updatePrefs((x) => (x.lang = l));
        // saved at once (not debounced): the window reloads in the new language
        await api.configSetUi(prefs() as unknown as Record<string, unknown>).catch(() => {});
        location.reload();
      });
      langs.append(b);
    }
    this.themeBox.replaceChildren(
      group(T('Тема'), cards, row(T('Как в системе'), toggle(p.themeSystem, (v) => updatePrefs((x) => (x.themeSystem = v))), T('светлая днём, тёмная/OLED ночью'))),
    );
    this.langBox.replaceChildren(group('Язык / Language', row(T('Язык интерфейса'), langs, T('Тексты песен не переводятся'))));
  }

  // ---------------------------------------------------------- discord

  private renderDiscord(): void {
    const d: DiscordConfig = { ...this.cfg!.discord };
    const save = () => api.discordSet(d).catch((e) => toast(errorMessage(e), 'error'));
    this.discordBox.replaceChildren(
      group(
        T('Статус в Discord'),
        row(T('Показывать, что я слушаю'), toggle(d.enabled, (v) => ((d.enabled = v), void save()))),
        row(T('Полоса прогресса'), toggle(d.progress, (v) => ((d.progress = v), void save())), T('тип «Слушает» с таймером трека')),
        row(T('Кнопка «Слушать на SoundCloud»'), toggle(d.button, (v) => ((d.button = v), void save()))),
        row(T('На паузе — скрывать статус'), toggle(d.hide_on_pause, (v) => ((d.hide_on_pause = v), void save())), T('иначе «на паузе» без таймера')),
        h('p', { class: 'muted small', text: T('Нужен только запущенный Discord на этом компьютере.') }),
      ),
    );
  }

  // ----------------------------------------------------------- lyrics

  private renderLyrics(): void {
    const p = prefs();
    this.lyricsBox.replaceChildren(
      group(
        T('Тексты песен'),
        row(T('Синхронный текст (караоке)'), toggle(p.syncedLyrics, (v) => updatePrefs((x) => (x.syncedLyrics = v))), T('ищет по очереди: LRCLIB → NetEase → Musixmatch')),
        row(T('Подсветка по словам'), toggle(p.wordHighlight, (v) => updatePrefs((x) => (x.wordHighlight = v))), T('где нет слов — плавно по строке')),
        h('p', { class: 'muted small', text: T('Если синхронного текста нигде нет — показывается обычный текст с Genius. Ничего вводить не нужно, тексты сохраняются в кэш.') }),
        h('div', { class: 'row-actions' }, (() => {
          const b = btn(T('Искать текст заново'), I.refresh);
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
      ['direct', T('Прямое подключение'), T('Через системную сеть: если включён «запрет» или VPN — работает через них')],
      ['proxy', T('Прокси'), T('SOCKS5 / HTTP — адрес ниже')],
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
    const check = btn(T('Проверить'), null);
    check.addEventListener('click', async () => {
      result.textContent = T('Проверяем…');
      try {
        const r = await api.netCheck(mode, proxy.value || null);
        result.replaceChildren(r.ok ? icon(I.check) : '', T('{0} · {1} мс', r.message, r.ms));
        result.className = r.ok ? 'small ok' : 'small err';
      } catch (e) {
        result.textContent = errorMessage(e);
        result.className = 'small err';
      }
    });
    const apply = btn(T('Применить'), null, true);
    apply.addEventListener('click', async () => {
      try {
        this.cfg = await api.configSetNetwork(mode, proxy.value || null, cfg.chrome_version);
        toast(T('Сетевые настройки применены'));
      } catch (e) {
        toast(errorMessage(e), 'error');
      }
    });
    this.netBox.replaceChildren(
      group(
        T('Сеть'),
        radios,
        h('div', { class: 'row-actions proxy-row' }, proxy, check, apply),
        result,
        faq(T('Как пользоваться прокси?'), [
          [T('Зачем'), T('весь трафик приложения пойдёт через указанный адрес — например, через ваш VPN-клиент, если он не включён системно.')],
          [T('Где взять адрес'), T('в настройках VPN-клиента найдите «локальный SOCKS-порт» (v2rayN — 10808, NekoBox — 2080, Clash — 7890).')],
          [T('Что вписать'), T('socks5h://127.0.0.1:ПОРТ (h — DNS тоже идёт через прокси). С паролем: socks5://логин:пароль@адрес:порт')],
          [T('Проверка'), T('нажмите «Проверить». Ошибка 403 — SoundCloud не пускает этот адрес, нужен другой сервер.')],
        ]),
      ),
    );
  }

  // ----------------------------------------------------------- system

  private renderSystem(autostart: boolean, minimized: boolean, fastProtected: boolean, notifyNew: boolean): void {
    let a = autostart;
    let m = minimized;
    let f = fastProtected;
    // on failure show the real state again (the switch must not lie)
    const save = () =>
      api.systemSet(a, m, f).catch(async (e) => {
        toast(errorMessage(e), 'error');
        const s = await api.systemGet();
        this.renderSystem(s.autostart, s.start_minimized, s.fast_protected, s.notify_new);
      });
    const version = h('span', { class: 'muted small' });
    void invoke<string>('app_version').then((v) => (version.textContent = T('Версия {0}', v)));
    const check = btn(T('Проверить обновления'), I.refresh);
    check.addEventListener('click', async () => {
      check.disabled = true;
      try {
        const r = await updates.check();
        if (r === 'none') toast(T('У вас последняя версия'));
      } catch (e) {
        toast(errorMessage(e), 'error');
      } finally {
        check.disabled = false;
      }
    });
    this.systemBox.replaceChildren(
      group(
        T('Система'),
        row(T('Запускать вместе с системой'), toggle(a, (v) => ((a = v), void save()))),
        row(T('При запуске сворачивать в трей'), toggle(m, (v) => ((m = v), void save()))),
        row(
          T('Мгновенный старт защищённых треков'),
          toggle(f, (v) => ((f = v), void save())),
          T('Держит плеер SoundCloud готовым, примерно +60 МБ памяти. Без этого первый такой трек стартует 5-10 секунд.'),
        ),
        row(
          T('Уведомлять о новых треках подписок'),
          toggle(notifyNew, (v) => void api.newsSet(v).catch((e) => toast(errorMessage(e), 'error'))),
          T('Уведомление Windows, когда артист из подписок выложил трек. Проверка раз в час.'),
        ),
        h('div', { class: 'row-actions' }, version, h('div', { class: 'spacer' }), check),
      ),
    );
  }
}
