// «Настройки» (mockup 8): account, theme, Discord, lyrics, network + FAQ, system.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { api, coverUrl, errorMessage, type AppConfig, type DiscordConfig, type NetMode } from '../api';
import { confirmDialog } from '../confirm';
import { h, toast } from '../dom';
import { icon, I } from '../icons';
import { customThemes, onPrefs, prefs, updatePrefs, type Theme } from '../prefs';
import { deleteTheme, editTheme, importTheme } from '../theme_editor';
import { store } from '../store';
import { lyricsData } from '../lyrics_data';
import { updates } from '../update_dialog';
import { btn, viewHead, type View } from './common';
import { currentLang, setLang, T, type Lang } from '../i18n';
import { HOTKEY_ACTIONS, hotkeys, recorder, resetCombos, setInGames } from '../hotkeys';

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
  // a switch is named by the row it sits in
  if (control.classList.contains('toggle') && !control.getAttribute('aria-label')) control.setAttribute('aria-label', label);
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
  private hotkeysBox = h('div');

  constructor() {
    this.el = h(
      'section',
      { class: 'view' },
      viewHead(T('Настройки')),
      h(
        'div',
        { class: 'view-scroll pad' },
        h('div', { class: 'settings' }, this.account, this.themeBox, this.langBox, this.hotkeysBox, this.discordBox, this.lyricsBox, this.netBox, this.systemBox),
      ),
    );
    store.on('auth', () => this.renderAccount());
    onPrefs(() => this.renderTheme());
  }

  show(): void {
    void listen('lyrics:index', () => void this.refreshIndex());
    this.renderAccount();
    this.renderTheme();
    this.renderLyrics();
    this.renderHotkeys();
    void this.load();
  }

  private async load(): Promise<void> {
    try {
      this.cfg = await api.configGet();
      this.renderDiscord();
      this.renderNet();
      this.autoPause = await api.autopauseGet();
      const sys = await api.systemGet();
      this.renderSystem(sys.autostart, sys.start_minimized, sys.fast_protected, sys.notify_new, sys.notify_rooms);
    } catch (e) {
      toast(errorMessage(e), 'error');
    }
  }

  // ---------------------------------------------------------- account

  private renderHotkeys(): void {
    const games = toggle(hotkeys().inGames, (v) => setInGames(v));
    const reset = btn(T('Сбросить'), null);
    reset.addEventListener('click', () => {
      resetCombos();
      this.renderHotkeys();
    });
    this.hotkeysBox.replaceChildren(
      group(
        T('Горячие клавиши'),
        row(T('Работают в играх'), games, T('Клавиши ловятся везде, даже поверх полноэкранной игры. Выключено - только когда окно SC Desk активно.')),
        ...HOTKEY_ACTIONS.map((a) => row(a.label, recorder(a.id))),
        row(T('Быстрый поиск'), h('span', { class: 'keycaps' }, h('kbd', { text: 'Ctrl' }), h('kbd', { text: 'K' })), T('Работает, когда окно открыто.')),
        row(T('Вернуть клавиши по умолчанию'), reset),
      ),
    );
  }

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
    // the user's own themes: a card each, drawn in their colours
    for (const c of customThemes()) {
      const active = p.theme === `custom:${c.id}`;
      const card = h(
        'button',
        { type: 'button', class: `theme-card${active ? ' is-active' : ''}` },
        h(
          'span',
          { class: 'tc-preview', style: `background:${c.bg}` },
          h('span', { class: 'tc-nav', style: `background:${c.panel}` }),
          h('span', { class: 'tc-l1', style: `background:${c.text}` }),
          h('span', { class: 'tc-l2', style: `background:color-mix(in srgb,${c.text} 34%,${c.bg})` }),
          h('span', { class: 'tc-play', style: `background:${c.accent}` }),
        ),
        h('span', { class: 'tc-label', dir: 'auto', text: c.name }),
      );
      card.addEventListener('click', () => updatePrefs((x) => (x.theme = `custom:${c.id}`)));
      cards.append(card);
    }
    const actions = h('div', { class: 'row-actions' });
    const add = btn(T('Своя тема'), I.plus);
    add.addEventListener('click', () => editTheme());
    const paste = btn(T('Вставить код темы'), I.link);
    paste.addEventListener('click', () => void importTheme());
    actions.append(add, paste);
    const current = customThemes().find((c) => p.theme === `custom:${c.id}`);
    if (current && !p.themeSystem) {
      const edit = btn(T('Изменить'), I.edit);
      edit.addEventListener('click', () => editTheme(current));
      const del = btn(T('Удалить'), I.trash);
      del.addEventListener('click', async () => {
        if (await confirmDialog(T('Удалить тему «{0}»?', current.name), T('Тема пропадёт из списка.'), T('Удалить'))) deleteTheme(current.id);
      });
      actions.append(edit, del);
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
      group(T('Тема'), cards, actions, row(T('Как в системе'), toggle(p.themeSystem, (v) => updatePrefs((x) => (x.themeSystem = v))), T('светлая днём, тёмная/OLED ночью'))),
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
        row(T('Кнопка «Присоединиться» для открытой комнаты'), toggle(d.room_join, (v) => ((d.room_join = v), void save())), T('В статусе появится кнопка для комнат без пароля. Друг, нажав её, попадёт на страницу, которая откроет комнату в SC Desk или предложит скачать программу. Код комнаты будет виден всем, кто смотрит ваш профиль.')),
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
        this.indexRow,
      ),
    );
    void this.refreshIndex();
  }

  /** Search by a line of a song (Ctrl+K → «По тексту»): looks the lyrics of all likes up in the background. */
  private indexRow = h('div');

  private async refreshIndex(): Promise<void> {
    try {
      const st = await api.lyricsIndexStatus();
      const btnRun = btn(st.running ? T('Остановить') : st.done >= st.total ? T('Обновить') : T('Проиндексировать'), st.running ? null : I.refresh);
      btnRun.addEventListener('click', async () => {
        await api.lyricsIndexRun(!st.running);
        void this.refreshIndex();
      });
      this.indexRow.replaceChildren(
        row(
          T('Поиск по тексту песен'),
          btnRun,
          st.total ? T('Тексты просмотрены у {0} из {1} лайков. Ищется в Ctrl+K на вкладке «По тексту».', st.done, st.total) : T('Сначала синхронизируйте лайки.'),
        ),
      );
    } catch (e) {
      toast(errorMessage(e), 'error');
    }
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

  private autoPause = { call: false, game: false };

  private saveAutoPause(patch: Partial<{ call: boolean; game: boolean }>): void {
    this.autoPause = { ...this.autoPause, ...patch };
    void api.autopauseSet(this.autoPause).catch((e) => toast(errorMessage(e), 'error'));
  }

  private renderSystem(autostart: boolean, minimized: boolean, fastProtected: boolean, notifyNew: boolean, notifyRooms: boolean): void {
    let a = autostart;
    let m = minimized;
    let f = fastProtected;
    // on failure show the real state again (the switch must not lie)
    const save = () =>
      api.systemSet(a, m, f).catch(async (e) => {
        toast(errorMessage(e), 'error');
        const s = await api.systemGet();
        this.renderSystem(s.autostart, s.start_minimized, s.fast_protected, s.notify_new, s.notify_rooms);
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
    const backupBtn = btn(T('Сохранить'), I.download);
    backupBtn.addEventListener('click', async () => {
      try {
        const path = await api.backupExport();
        if (path) toast(T('Копия сохранена: {0}', path.split(/[\/]/).pop() ?? path));
      } catch (e) {
        toast(errorMessage(e), 'error');
      }
    });
    const restoreBtn = btn(T('Восстановить'), I.upload);
    restoreBtn.addEventListener('click', async () => {
      try {
        const r = await api.backupImport();
        if (r) {
          toast(T('Восстановлено: {0} лайков, {1} прослушиваний. Настройки подхватятся после перезапуска.', r.likes, r.plays));
          await store.reloadSets();
        }
      } catch (e) {
        toast(errorMessage(e), 'error');
      }
    });
    const copyLog = btn(T('Скопировать лог'), I.link);
    copyLog.addEventListener('click', async () => {
      try {
        await navigator.clipboard.writeText(await api.logTail(300));
        toast(T('Журнал скопирован'));
      } catch (e) {
        toast(errorMessage(e), 'error');
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
        row(
          T('Уведомлять об открытых комнатах друзей'),
          toggle(notifyRooms, (v) => void api.roomsNotifySet(v).catch((e) => toast(errorMessage(e), 'error'))),
          T('Уведомление Windows, когда кто-то из подписок открыл комнату. Проверка раз в полторы минуты.'),
        ),
        row(T('Пауза во время звонка'), toggle(this.autoPause.call, (v) => this.saveAutoPause({ call: v })), T('Музыка встаёт на паузу, когда другая программа включила микрофон (Discord, Zoom), и продолжается после звонка.')),
        row(T('Пауза в полноэкранной игре'), toggle(this.autoPause.game, (v) => this.saveAutoPause({ game: v })), T('Музыка встаёт на паузу, когда игра открылась на весь экран, и продолжается, когда вы вышли из неё. Выключено: музыка играет и в игре.')),
        row(T('Резервная копия'), h('div', { class: 'row-actions' }, backupBtn, restoreBtn), T('Лайки, история, дизлайки, настройки одним файлом. Вход в SoundCloud не входит в файл.')),
        row(T('Если что-то сломалось'), copyLog, T('Копирует конец журнала программы, его можно отправить вместе с описанием ошибки.')),
        h('div', { class: 'row-actions' }, version, h('div', { class: 'spacer' }), check),
      ),
    );
  }
}
