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
import { mb } from './profile';
import { currentLang, setLang, T, type Lang } from '../i18n';
import { HOTKEY_ACTIONS, hotkeys, recorder, resetCombos, setInGames, setMiniClickSecs } from '../hotkeys';

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
  private vpnBox = h('div');
  private storageBox = h('div');
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
        h('div', { class: 'settings' }, this.account, this.themeBox, this.langBox, this.hotkeysBox, this.discordBox, this.lyricsBox, this.netBox, this.systemBox, this.storageBox),
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
      void this.renderStorage();
      this.renderNet();
      this.autoPause = await api.autopauseGet();
      const sys = await api.systemGet();
      this.renderSystem(sys.autostart, sys.start_minimized, sys.fast_protected, sys.notify_new, sys.notify_rooms, sys.share_listening);
    } catch (e) {
      toast(errorMessage(e), 'error');
    }
  }

  // ---------------------------------------------------------- account

  private renderHotkeys(): void {
    const games = toggle(hotkeys().inGames, (v) => setInGames(v));
    // how long the mini player takes clicks over a game after its key
    const secs = h('select', { class: 'input', 'aria-label': T('Сколько пилюля нажимаема в игре') });
    for (const [v, label] of [[5, T('5 секунд')], [10, T('10 секунд')], [20, T('20 секунд')], [30, T('30 секунд')], [60, T('1 минуту')], [0, T('до повторного нажатия клавиши')]] as [number, string][]) {
      const o = h('option', { value: String(v), text: label });
      if (v === (hotkeys().miniClickSecs ?? 20)) o.selected = true;
      secs.append(o);
    }
    secs.addEventListener('change', () => setMiniClickSecs(Number(secs.value)));
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
        row(T('Сколько пилюля нажимаема в игре'), secs, T('После клавиши «Мини-плеер: нажимать в игре». Нажатие во время этого времени возвращает всё сразу.')),
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
    const cid = h('input', { class: 'input', type: 'password', placeholder: T('client_id (необязательно)'), autocomplete: 'off' });
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
      // not signed in: the manual way is open at once (some accounts cannot sign in through the window)
      { class: 'details', ...(a.has_credentials ? {} : { open: 'open' }) },
      h('summary', { text: T('Ввести токен вручную') }),
      h('p', { class: 'muted small', text: T('Если вход через окно не получается. Самый простой способ: откройте soundcloud.com в браузере и войдите, затем F12 → Application (Приложение) → Cookies → https://soundcloud.com → строка oauth_token → скопируйте её значение и вставьте сюда. client_id можно не вводить: программа найдёт его сама.') }),
      cid,
      tok,
      save,
    );
    // the playback account: another account's token that only gets the audio (the subscription lives there)
    const streamBox = h('div', { class: 'set-stream' });
    const renderStream = (name: string | null, error = '') => {
      const field = h('input', { class: 'input', type: 'password', placeholder: 'oauth_token', autocomplete: 'off' });
      const saveStream = btn(T('Сохранить и проверить'), null);
      saveStream.addEventListener('click', async () => {
        try {
          const n = await invoke<string>('stream_token_save', { token: field.value });
          field.value = '';
          renderStream(n);
          toast(T('Подключён: {0}', n));
        } catch (e) {
          toast(errorMessage(e), 'error');
        }
      });
      const off = btn(T('Убрать'), null);
      off.addEventListener('click', async () => {
        await invoke('stream_token_clear').catch((e) => toast(errorMessage(e), 'error'));
        renderStream(null);
      });
      streamBox.replaceChildren(
        ...(name || error ? [h('div', { class: 'muted small', text: name ? T('Подключён: {0}', name) : error })] : []),
        field,
        h('div', { class: 'row-actions' }, saveStream, ...(name ? [off] : [])),
      );
    };
    renderStream(null);
    if (a.has_credentials) {
      void invoke<string | null>('stream_token_state').then(
        (n) => renderStream(n),
        (e) => renderStream(null, errorMessage(e)),
      );
    }
    const streamDetails = h(
      'details',
      { class: 'details' },
      h('summary', { text: T('Второй аккаунт (подписка)') }),
      h('p', { class: 'muted small', text: T('oauth_token второго аккаунта. Основной остаётся как есть.') }),
      streamBox,
    );
    this.account.replaceChildren(
      group(
        T('Аккаунт SoundCloud'),
        a.has_credentials
          ? h('div', { class: 'set-user' }, avatar, h('div', {}, h('div', { class: 'strong', text: a.username ?? T('Аккаунт') }), h('div', { class: 'muted small', text: T('Вход выполнен') })))
          : h('p', { class: 'muted small', text: T('Откроется окно soundcloud.com — войдите как обычно. Ключи сохранятся в системном хранилище.') }),
        h('div', { class: 'row-actions' }, a.has_credentials ? logout : loginBtn),
        manual,
        ...(a.has_credentials ? [streamDetails] : []),
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
        row(T('Подстраивать текст по звуку'), toggle(p.lyricsAuto !== false, (v) => updatePrefs((x) => (x.lyricsAuto = v))), T('Если слова появляются раньше или позже голоса, программа сама сдвигает текст по записи. Свой сдвиг (кнопки +/− под текстом) запоминается для каждого трека.')),
        row(T('Узнавать песню по словам'), toggle(p.lyricsHear !== false, (v) => updatePrefs((x) => (x.lyricsHear = v))), T('Если по названию текста нигде нет, программа послушает 15 секунд песни и найдёт её по услышанным словам. Только для таких треков и один раз на трек, около секунды работы. В первый раз скачает модель распознавания (57 МБ).')),
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
      input.addEventListener('change', async () => {
        mode = m;
        proxy.disabled = mode !== 'proxy';
        // the choice is kept at once; "Применить" is for the proxy address. A proxy without an address is not switched on.
        if (mode === 'direct' || proxy.value.trim()) {
          try {
            this.cfg = await api.configSetNetwork(mode, proxy.value || null, cfg.chrome_version);
          } catch (e) {
            toast(errorMessage(e), 'error');
          }
        } else {
          proxy.focus();
        }
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
    // everything about the network back to how it is by default: direct, bypass on «Авто», no own server
    const resetNet = btn(T('Сбросить сеть'), null);
    resetNet.addEventListener('click', async () => {
      try {
        updatePrefs((p) => (p.scBypass = 'auto'));
        await api.vpnSelect(null).catch(() => undefined);
        this.cfg = await api.configSetNetwork('direct', null, cfg.chrome_version);
        toast(T('Сеть сброшена: прямое подключение'));
        this.renderNet();
      } catch (e) {
        toast(errorMessage(e), 'error');
      }
    });
    void this.renderVpn();
    this.netBox.replaceChildren(
      group(
        T('Сеть'),
        radios,
        h('div', { class: 'row-actions proxy-row' }, proxy, check, apply, resetNet),
        result,
        (() => {
          // the provider blocks SoundCloud by its name: cut the greeting (net/bypass.rs), no server in between
          const sel = h('select', { class: 'input' }) as HTMLSelectElement;
          for (const [v, label] of [
            ['auto', T('Авто')],
            ['on', T('Всегда')],
            ['off', T('Выключен')],
          ] as const) {
            const o = h('option', { value: v, text: label }) as HTMLOptionElement;
            o.selected = (prefs().scBypass ?? 'auto') === v;
            sel.append(o);
          }
          sel.addEventListener('change', () => updatePrefs((p) => (p.scBypass = sel.value as 'auto' | 'on' | 'off')));
          return row(
            T('Обход блокировки SoundCloud'),
            sel,
            T('Если провайдер блокирует SoundCloud, программа сама режет начало соединения так, что фильтр не видит имя сайта. Без чужих серверов и VPN. «Авто» включает это, только когда SoundCloud не открывается напрямую, поэтому с «запретом» и VPN не конфликтует. С прокси не используется. Не поможет, если заблокирован сам адрес сервера.'),
          );
        })(),
        this.vpnBox,
        faq(T('Как пользоваться прокси?'), [
          [T('Зачем'), T('весь трафик приложения пойдёт через указанный адрес — например, через ваш VPN-клиент, если он не включён системно.')],
          [T('Где взять адрес'), T('в настройках VPN-клиента найдите «локальный SOCKS-порт» (v2rayN — 10808, NekoBox — 2080, Clash — 7890).')],
          [T('Что вписать'), T('socks5h://127.0.0.1:ПОРТ (h — DNS тоже идёт через прокси). С паролем: socks5://логин:пароль@адрес:порт')],
          [T('Проверка'), T('нажмите «Проверить». Ошибка 403 — SoundCloud не пускает этот адрес, нужен другой сервер.')],
        ]),
      ),
    );
  }

  /** Хранилище: what the app keeps on this computer, and the buttons that free it. */
  private async renderStorage(): Promise<void> {
    const s = await api.storageInfo().catch(() => null);
    if (!s) return;
    const line = (label: string, bytes: number, hint: string) => row(label, h('span', { class: 'muted', text: mb(bytes) }), hint);
    const clear = btn(T('Очистить кэш'), I.close);
    clear.addEventListener('click', async () => {
      await api.storageClearCache().catch((e) => toast(errorMessage(e), 'error'));
      toast(T('Обложки удалены. Кэш встроенного браузера очистится при следующем запуске программы.'));
      void this.renderStorage();
    });
    const model = btn(T('Удалить модель распознавания'), I.close);
    model.disabled = s.model === 0;
    model.addEventListener('click', async () => {
      await api.storageDeleteModel().catch((e) => toast(errorMessage(e), 'error'));
      void this.renderStorage();
    });
    this.storageBox.replaceChildren(
      group(
        T('Хранилище'),
        line(T('Библиотека'), s.database, T('треки, тексты песен, история: то, что программа уже знает, чтобы не спрашивать SoundCloud снова')),
        line(T('Обложки'), s.covers, T('не больше 12 МБ, старые удаляются сами')),
        line(T('Кэш встроенного браузера'), s.browser, T('окно входа и проверки SoundCloud; не больше 8 МБ с версии 1.2.1')),
        line(T('Модель распознавания песен'), s.model, T('скачивается, только когда песню надо узнать по словам; удалите, если не нужна')),
        line(T('Мои файлы'), s.local, T('ваши треки, сжатые и зашифрованные')),
        h('div', { class: 'row-actions' }, clear, model),
      ),
    );
  }

  /** «Свой сервер»: a VPN subscription pasted here, a server picked from the list, no VPN app needed. */
  private async renderVpn(): Promise<void> {
    const v = await api.vpnGet().catch(() => null);
    if (!v) return;
    const url = h('input', { class: 'input', type: 'password', placeholder: T('Ссылка на подписку или сервер (vless://, https://…)'), spellcheck: 'false' }) as HTMLInputElement;
    const status = h('span', { class: 'muted small' });
    const load = btn(v.has_url ? T('Обновить') : T('Загрузить'), null);
    // which VPN app to introduce itself as: some panels give the servers only to the apps they allow
    const as = h('select', { class: 'input', title: T('Представляться подписке как') }) as HTMLSelectElement;
    for (const [val, label] of [
      ['auto', T('Как: авто')],
      ['happ', 'Happ'],
      ['v2rayn', 'v2rayN'],
      ['hiddify', 'Hiddify'],
      ['nekobox', 'NekoBox'],
      ['streisand', 'Streisand'],
      ['clash', 'Koala / Clash'],
    ] as const) {
      as.append(h('option', { value: val, text: label }));
    }
    const sel = h('select', { class: 'input' }) as HTMLSelectElement;
    const fill = (s: typeof v) => {
      sel.replaceChildren(h('option', { value: '', text: T('Не использовать') }));
      s.servers.forEach((srv, i) => {
        const o = h('option', { value: String(i), text: srv.ok ? srv.name : `${srv.name} ${T('(не поддерживается)')}` }) as HTMLOptionElement;
        o.disabled = !srv.ok;
        o.selected = s.selected === i;
        sel.append(o);
      });
      sel.disabled = !s.servers.length;
      status.textContent = s.running ? T('Подключено: SoundCloud идёт через выбранный сервер') : s.servers.length ? T('Серверов: {0}', s.servers.length) : '';
    };
    fill(v);
    load.addEventListener('click', async () => {
      status.textContent = T('Загружаю…');
      try {
        fill(await api.vpnSetUrl(url.value, as.value));
        url.value = '';
      } catch (e) {
        status.textContent = errorMessage(e);
      }
    });
    sel.addEventListener('change', async () => {
      status.textContent = sel.value ? T('Подключаю… (в первый раз скачивается движок, около 30 МБ)') : '';
      try {
        fill(await api.vpnSelect(sel.value === '' ? null : Number(sel.value)));
      } catch (e) {
        status.textContent = errorMessage(e);
      }
    });
    // a service that gives the servers only to registered apps: take the copy Koala keeps on this computer
    const koala = btn(T('Взять из VPN-программы'), null);
    koala.addEventListener('click', async () => {
      status.textContent = T('Загружаю…');
      try {
        fill(await api.vpnSetUrl('koala:', 'koala'));
      } catch (e) {
        status.textContent = errorMessage(e);
      }
    });
    this.vpnBox.replaceChildren(
      row(T('Свой сервер (без VPN-программы)'), sel, T('Вставьте подписку из Happ, v2rayN и т.п. и выберите сервер: через него пойдёт только SoundCloud, а VPN-программу можно закрыть. Работает при «Прямом подключении». Ссылка хранится в защищённом хранилище Windows.')),
      h('div', { class: 'row-actions proxy-row' }, url, as, load, koala),
      status,
    );
  }

  // ----------------------------------------------------------- system

  private autoPause = { call: false, game: false };

  private saveAutoPause(patch: Partial<{ call: boolean; game: boolean }>): void {
    this.autoPause = { ...this.autoPause, ...patch };
    void api.autopauseSet(this.autoPause).catch((e) => toast(errorMessage(e), 'error'));
  }

  private renderSystem(autostart: boolean, minimized: boolean, fastProtected: boolean, notifyNew: boolean, notifyRooms: boolean, shareListening: boolean): void {
    let a = autostart;
    let m = minimized;
    let f = fastProtected;
    // on failure show the real state again (the switch must not lie)
    const save = () =>
      api.systemSet(a, m, f).catch(async (e) => {
        toast(errorMessage(e), 'error');
        const s = await api.systemGet();
        this.renderSystem(s.autostart, s.start_minimized, s.fast_protected, s.notify_new, s.notify_rooms, s.share_listening);
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
    const report = btn(T('Отчёт об ошибке'), I.download);
    report.addEventListener('click', async () => {
      try {
        await api.reportSave();
        toast(T('Отчёт сохранён на рабочем столе'));
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
        row(
          T('Показывать друзьям, что я слушаю'),
          toggle(shareListening, (v) => void api.friendsShareSet(v).catch((e) => toast(errorMessage(e), 'error'))),
          T('Видят только ваши друзья, больше никто. Выключено: друзья не увидят, что у вас играет.'),
        ),
        row(T('Пауза во время звонка'), toggle(this.autoPause.call, (v) => this.saveAutoPause({ call: v })), T('Музыка встаёт на паузу, когда другая программа включила микрофон (Discord, Zoom), и продолжается после звонка.')),
        row(T('Пауза в полноэкранной игре'), toggle(this.autoPause.game, (v) => this.saveAutoPause({ game: v })), T('Музыка встаёт на паузу, когда игра открылась на весь экран, и продолжается, когда вы вышли из неё. Выключено: музыка играет и в игре.')),
        row(T('Резервная копия'), h('div', { class: 'row-actions' }, backupBtn, restoreBtn), T('Лайки, история, дизлайки, настройки одним файлом. Вход в SoundCloud не входит в файл.')),
        row(T('Если что-то сломалось'), copyLog, T('Копирует конец журнала программы, его можно отправить вместе с описанием ошибки.')),
        row(T('Отчёт об ошибке'), report, T('Файл на рабочем столе: версия, система, настройки сети и конец журнала. Токены, ключи, ссылки подписок и имя пользователя Windows из него вырезаются, его можно отправлять.')),
        h('div', { class: 'row-actions' }, version, h('div', { class: 'spacer' }), check),
      ),
    );
  }
}
