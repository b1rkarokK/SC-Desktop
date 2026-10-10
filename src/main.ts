import { listen } from '@tauri-apps/api/event';
import { api, errorMessage, isAppError, type AppErrorPayload, type AuthStatus, type PlayerSnapshot, type Track } from './ui/api';
import { clock } from './ui/clock';
import { setWindowShown, toast } from './ui/dom';
import { EqPopover } from './ui/eq_popover';
import { QueuePopover } from './ui/queue_popover';
import { Fullscreen } from './ui/fullscreen';
import { lyricsData } from './ui/lyrics_data';
import { LyricsPanel } from './ui/lyrics_panel';
import { MainWindow } from './ui/main_window';
import { PlayerBar } from './ui/player_bar';
import { loadPrefs, prefs, updatePrefs } from './ui/prefs';
import { router } from './ui/router';
import { store } from './ui/store';
import { TitleBar } from './ui/titlebar';
import { updates } from './ui/update_dialog';
import { currentLang, T } from './ui/i18n';

const $ = (id: string) => document.getElementById(id)!;

async function boot(): Promise<void> {
  const cfg = await api.configGet();
  loadPrefs(cfg.ui);
  // the core shows a few texts itself (tray, Discord): tell it the language
  if (prefs().lang !== currentLang()) updatePrefs((p) => (p.lang = currentLang()));

  new TitleBar($('titlebar'));
  const fullscreen = new Fullscreen();
  document.body.append(fullscreen.el);
  const eq = new EqPopover();
  $('player').append(eq.el);
  const queue = new QueuePopover();
  $('player').append(queue.el);
  const main = new MainWindow($('nav'), $('center'));
  new LyricsPanel($('lyrics'), () => fullscreen.toggle());
  const bar = new PlayerBar($('player'), eq, queue, () => fullscreen.toggle());
  eq.init(cfg.eq, cfg.fx, (open, enabled) => bar.setEqActive(open, enabled));

  await Promise.all([
    listen<PlayerSnapshot>('player:state', (e) => store.setSnapshot(e.payload)),
    listen<Track>('player:track', (e) => {
      clock.reset(0);
      lyricsData.setTrack(e.payload);
    }),
    listen<number>('player:seek', (e) => clock.reset(e.payload)),
    listen<{ id: number; ms: number }>('player:lyrics-shift', (e) => lyricsData.setShift(e.payload.id, e.payload.ms)),
    listen<AppErrorPayload>('player:error', (e) => toast(e.payload.message, 'error')),
    listen<number>('likes:progress', (e) => main.library.setProgress(e.payload)),
    listen<number>('likes:changed', () => {
      void store.reloadSets();
      main.library.reloadFromCache();
    }),
    listen<boolean>('window:visibility', (e) => setWindowShown(e.payload)),
    listen<AuthStatus>('auth:changed', (e) => {
      store.setAuth(e.payload);
      void store.reloadSets();
      toast(e.payload.username ? T('Вход выполнен: {0}', e.payload.username) : T('Вход выполнен'));
      router.go({ name: 'likes' });
      void main.library.sync();
    }),
  ]);

  void updates.listen();
  const [auth, snap] = await Promise.all([api.authStatus(), api.snapshot()]);
  store.setAuth(auth);
  store.setSnapshot(snap);
  lyricsData.setTrack(snap.track);

  if (!auth.has_credentials) {
    main.start({ name: 'settings' });
    return;
  }
  main.start({ name: 'home' });
  void store.reloadSets();
  api.libraryArtists(false).then((u) => store.setFollowing(u), () => undefined);
  api.authVerify().then(
    (a) => store.setAuth(a),
    (e) => {
      toast(errorMessage(e), 'error');
      if (isAppError(e) && (e.kind === 'auth_expired' || e.kind === 'not_authorized')) router.go({ name: 'settings' });
    },
  );
}

document.addEventListener('keydown', (e) => {
  const t = e.target;
  if (t instanceof Element && t.closest('input, textarea, select, [contenteditable], [role="slider"]')) return;
  if (e.code === 'Space') {
    e.preventDefault();
    void api.toggle();
  } else if (e.altKey && e.key === 'ArrowLeft') {
    router.back();
  }
});

// mouse "back" button
window.addEventListener('mouseup', (e) => {
  if (e.button === 3) router.back();
});

// no native context menu: this is an app, not a web page
document.addEventListener('contextmenu', (e) => {
  if (!(e.target instanceof Element && e.target.closest('input, textarea'))) e.preventDefault();
});

// the mini player is the same page in its own small window
if (new URLSearchParams(location.search).has('mini')) {
  void import('./ui/mini').then((m) => m.bootMini()).catch((e) => toast(errorMessage(e), 'error'));
} else {
  boot().catch((e) => toast(T('Ошибка запуска: {0}', errorMessage(e)), 'error', 15000));
}
