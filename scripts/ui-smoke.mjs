// A walk through every screen of the running dev app (CDP, port 9223 by default), looking for the kind of
// defects that slipped through by eye: console errors, text squeezed into a thin column, sideways overflow,
// "undefined" / "NaN" on screen, buttons nobody can name, empty screens, Russian left in the English UI.
// Screenshots go to scripts/ui-smoke-out/ (ignored by git).
//
//   node scripts/ui-smoke.mjs                # as the app is now
//   node scripts/ui-smoke.mjs --themes       # also dark, light and oled
//   node scripts/ui-smoke.mjs --en           # switch the app to English, then walk
//
// The app must be running with `scripts/dev-parallel.ps1` (it opens the debug port).
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const port = process.env.CDP_PORT || '9223';
const args = new Set(process.argv.slice(2));
const out = path.join(path.dirname(fileURLToPath(import.meta.url)), 'ui-smoke-out');
fs.mkdirSync(out, { recursive: true });

const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = targets.find((t) => t.type === 'page' && /1420/.test(t.url));
if (!page) throw new Error('no app page on the debug port: is dev-parallel.ps1 running?');
const ws = new WebSocket(page.webSocketDebuggerUrl);
let nextId = 0;
const pending = new Map();
const problems = [];
let current = 'start';
ws.onmessage = (m) => {
  const d = JSON.parse(m.data);
  if (d.id && pending.has(d.id)) return pending.get(d.id)(d);
  if (d.method === 'Runtime.exceptionThrown') {
    const e = d.params.exceptionDetails;
    problems.push({ where: current, kind: 'exception', text: (e.exception?.description || e.text || '').split('\n')[0].slice(0, 200) });
  } else if (d.method === 'Runtime.consoleAPICalled' && d.params.type === 'error') {
    const t = d.params.args.map((a) => a.value ?? a.description ?? '').join(' ').slice(0, 200);
    // the WebView's own noise about images that did not load is not ours
    if (!/Failed to load resource|ERR_|net::/.test(t)) problems.push({ where: current, kind: 'console.error', text: t });
  }
};
const send = (method, params = {}) =>
  new Promise((res, rej) => {
    const id = ++nextId;
    pending.set(id, res);
    ws.send(JSON.stringify({ id, method, params }));
    setTimeout(() => rej(new Error('timeout ' + method)), 30000);
  });
await new Promise((r) => (ws.onopen = r));
await send('Runtime.enable');
const evaluate = async (expression) => {
  const r = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
  if (r.result.exceptionDetails) throw new Error(r.result.exceptionDetails.exception?.description || 'eval failed');
  return r.result.result.value;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const shot = async (name) => {
  const [w, h] = JSON.parse(await evaluate('JSON.stringify([innerWidth, innerHeight])'));
  const s = await send('Page.captureScreenshot', { format: 'png', clip: { x: 0, y: 0, width: w, height: h, scale: 1 } });
  fs.writeFileSync(path.join(out, name.replace(/[^\w.-]+/g, '_') + '.png'), Buffer.from(s.result.data, 'base64'));
};

// What is wrong on the screen right now (run inside the page).
const AUDIT = `(() => {
  const found = [];
  const visible = (el) => { const r = el.getBoundingClientRect(); const cs = getComputedStyle(el); return r.width > 0 && r.height > 0 && cs.visibility !== 'hidden' && cs.display !== 'none' && !el.closest('[hidden]'); };
  const center = document.getElementById('center');
  const root = document.documentElement;
  if (root.scrollWidth > innerWidth + 1) found.push(['overflow', 'the page is wider than the window: ' + root.scrollWidth + ' > ' + innerWidth]);
  if (center && center.scrollWidth > center.clientWidth + 1) found.push(['overflow', 'the main area scrolls sideways: ' + center.scrollWidth + ' > ' + center.clientWidth]);
  if (center && center.innerText.trim().length < 3 && !center.querySelector('canvas,img')) found.push(['empty', 'the main area has no text at all']);
  const all = [...document.querySelectorAll('body *')].filter(visible);
  for (const el of all) {
    const own = [...el.childNodes].filter((n) => n.nodeType === 3).map((n) => n.textContent).join('').trim();
    const w = el.getBoundingClientRect().width;
    if ([...own].length >= 18 && w < 70 && !el.closest('svg')) found.push(['squeezed', '"' + own.slice(0, 40) + '" in ' + Math.round(w) + ' px (' + el.className + ')']);
    if (/\\b(undefined|NaN|\\[object)\\b/.test(own)) found.push(['bad-text', '"' + own.slice(0, 60) + '"']);
  }
  for (const b of document.querySelectorAll('button, [role=button]')) {
    if (!visible(b)) continue;
    const name = (b.textContent || '').trim() || b.getAttribute('aria-label') || b.getAttribute('title');
    if (!name) found.push(['no-name', 'a button without a name: ' + b.className]);
  }
  for (const img of document.images) {
    if (visible(img) && img.complete && img.naturalWidth === 0 && img.getAttribute('src')) found.push(['image', 'did not load: ' + img.getAttribute('src').slice(0, 60)]);
  }
  if (document.documentElement.lang === 'en' || localStorage.getItem('scdesk.lang') === 'en') {
    const chrome = [...document.querySelectorAll('button, .btn, .seg-tab, .nav-label, label, summary, h2, h3, .upd-title, .muted')].filter(visible).filter((e) => !e.closest('.row, .rm-qrow, .rm-q, .card, .pal-row, .profile-row, .fd-row, .rm-item, .rm-person, .sp-tile, .hist-row, .lyrics, .ly-line'));
    for (const el of chrome) {
      const own = [...el.childNodes].filter((n) => n.nodeType === 3).map((n) => n.textContent).join('').trim();
      if (own === 'Русский' || own === 'Язык / Language' || (el.closest('.upd') && own.includes(' - ')) || own.startsWith('· ') || el.closest('.eq-device, .hero, .pb-now, .fs-meta, .mini-meta, .wave-meta')) continue;
      if (/[А-Яа-яЁё]/.test(own) && own.length < 120) found.push(['untranslated', '"' + own.slice(0, 60) + '"']);
    }
  }
  return JSON.stringify(found);
})()`;

const audit = async (label) => {
  current = label;
  await sleep(1300);
  const found = JSON.parse(await evaluate(AUDIT));
  const seen = new Set();
  for (const [kind, text] of found) {
    const key = kind + text;
    if (seen.has(key)) continue;
    seen.add(key);
    problems.push({ where: label, kind, text });
  }
  await shot(label);
};

const click = (sel, text) =>
  evaluate(`(() => { const els = [...document.querySelectorAll(${JSON.stringify(sel)})]; const el = ${text ? `els.find((e) => e.textContent.trim().startsWith(${JSON.stringify(text)}))` : 'els[0]'}; if (!el) return false; el.click(); return true; })()`);

async function walk(tag) {
  const names = ['home', 'search', 'wave', 'likes', 'playlists', 'rooms', 'history', 'stats', 'settings'];
  for (let i = 0; i < names.length; i++) {
    await evaluate(`document.querySelectorAll('.nav-item')[${i}]?.click(); 1`);
    await audit(`${tag}-${names[i]}`);
  }
  // library tabs
  await evaluate(`document.querySelectorAll('.nav-item')[3].click(); 1`);
  await sleep(600);
  for (const t of ['Плейлисты', 'Playlists', 'Альбомы', 'Albums', 'Артисты', 'Artists']) {
    if (await click('.seg-tab', t)) await audit(`${tag}-likes-${t}`);
  }
  // profile and its tabs
  await evaluate(`document.querySelector('.nav-user')?.click(); 1`);
  await sleep(500);
  const tabs = JSON.parse(await evaluate(`JSON.stringify([...document.querySelectorAll('.seg-tab')].map((b) => b.textContent.trim()))`));
  for (const t of tabs.slice(0, 4)) {
    await click('.seg-tab', t.replace(/\s*\d+$/, ''));
    await audit(`${tag}-profile-${t}`);
  }
  // popovers: equalizer (with its SoundCloud-player note forced on), queue, sleep, quick search
  const bar = '.pb-right .btn-icon';
  for (const [i, label] of [[1, 'queue'], [2, 'sleep'], [3, 'eq']]) {
    await evaluate(`document.querySelectorAll(${JSON.stringify(bar)})[${i}]?.click(); 1`);
    if (label === 'eq') await evaluate(`(() => { const n = document.querySelector('.eq-note'); if (n) n.hidden = false; return 1; })()`);
    await audit(`${tag}-${label}`);
    await evaluate(`document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })); 1`);
    await evaluate(`document.querySelector('.eq-note') && (document.querySelector('.eq-note').hidden = true); 1`);
  }
  await evaluate(`document.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', ctrlKey: true, bubbles: true })); window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', code: 'KeyK', ctrlKey: true, bubbles: true })); 1`);
  await audit(`${tag}-palette`);
  await evaluate(`document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })); 1`);
  await deep(tag);
  await dialogs(tag);
  await evaluate(`document.querySelectorAll('.nav-item')[0].click(); 1`);
}

const esc = () => evaluate(`document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })); window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })); 1`);
const clickText = (sel, ...texts) =>
  evaluate(`(() => { const els = [...document.querySelectorAll(${JSON.stringify(sel)})]; const want = ${JSON.stringify(texts)}; const el = els.find((e) => want.some((t) => e.textContent.trim().startsWith(t))); if (!el) return false; el.click(); return true; })()`);

// the dialogs and menus one opens on purpose
async function dialogs(tag) {
  const open = async (nav, sel, texts, label, tab) => {
    await evaluate(`document.querySelectorAll('.nav-item')[${nav}].click(); 1`);
    await sleep(700);
    if (tab) {
      await evaluate(`document.querySelector('.nav-user')?.click(); 1`);
      await sleep(400);
      await clickText('.seg-tab', ...tab);
      await sleep(500);
    }
    if (await clickText(sel, ...texts)) {
      await audit(`${tag}-dialog-${label}`);
      await esc();
      await sleep(300);
      // a dialog that Escape does not close is closed by its own "done" / "cancel"
      await evaluate(`document.querySelectorAll('.upd-backdrop .btn').forEach((b) => b.click()); 1`);
    }
  };
  await open(8, 'button', ['Своя тема', 'Custom theme'], 'custom-theme');
  await open(8, 'button', ['Вставить код темы', 'Paste a theme code'], 'paste-theme');
  await open(0, '.nav-user', [''], 'profile-friends-open', ['Друзья', 'Friends']);
  await clickText('button', 'Добавить друга', 'Add a friend');
  await audit(`${tag}-dialog-add-friend`);
  await esc();
  await evaluate(`document.querySelectorAll('.upd-backdrop .btn').forEach((b) => b.click()); 1`);
  await open(5, '.rm-joinbtn, button', ['Присоединиться', 'Join'], 'join-room');
  await open(4, 'button', ['Создать', 'Create'], 'create-shared');
  await open(4, 'button', ['Присоединиться', 'Join'], 'join-shared');
  // a track's menu and the share dialog
  await evaluate(`document.querySelectorAll('.nav-item')[3].click(); 1`);
  await sleep(800);
  await clickText('.seg-tab', 'Треки', 'Tracks');
  await sleep(600);
  await evaluate(`(() => { const r = document.querySelector('.row'); if (!r) return 0; r.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: 300, clientY: 300 })); return 1; })()`);
  await sleep(300);
  await audit(`${tag}-track-menu`);
  if (await clickText('.ctx button, .menu button, [role=menuitem]', 'Поделиться', 'Share')) {
    await audit(`${tag}-dialog-share`);
    await esc();
    await evaluate(`document.querySelectorAll('.upd-backdrop .btn').forEach((b) => b.click()); 1`);
  }
  await esc();
}

// pages one clicks into: search results, an artist with all its tabs, a playlist, a track
async function deep(tag) {
  await evaluate(`document.querySelectorAll('.nav-item')[1].click(); 1`);
  await sleep(500);
  await evaluate(`(() => { const i = document.querySelector('.view input[type=search], .view input.input'); if (!i) return 0; i.value = 'lofi'; i.dispatchEvent(new Event('input', { bubbles: true })); i.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })); return 1; })()`);
  await sleep(2500);
  await audit(`${tag}-search-results`);
  // an artist from the library
  await evaluate(`document.querySelectorAll('.nav-item')[3].click(); 1`);
  await sleep(500);
  for (const t of ['Артисты', 'Artists']) await click('.seg-tab', t);
  await sleep(500);
  if (await evaluate(`(() => { const c = document.querySelector('.grid-artists .card, .grid-artists > *'); if (!c) return false; c.click(); return true; })()`)) {
    await sleep(1500);
    const tabs = JSON.parse(await evaluate(`JSON.stringify([...document.querySelectorAll('.seg-tab')].map((b) => b.textContent.trim()))`));
    for (const t of tabs.slice(0, 7)) {
      await click('.seg-tab', t);
      await audit(`${tag}-artist-${t}`);
    }
  }
  // a playlist of the library
  await evaluate(`document.querySelectorAll('.nav-item')[3].click(); 1`);
  await sleep(500);
  for (const t of ['Плейлисты', 'Playlists']) await click('.seg-tab', t);
  await sleep(500);
  if (await evaluate(`(() => { const c = document.querySelector('.grid .card, .grid > *'); if (!c) return false; c.click(); return true; })()`)) await audit(`${tag}-playlist-page`);
  // a track page from a liked track
  await evaluate(`document.querySelectorAll('.nav-item')[3].click(); 1`);
  await sleep(500);
  for (const t of ['Треки', 'Tracks']) await click('.seg-tab', t);
  await sleep(800);
  if (await evaluate(`(() => { const b = document.querySelector('.row .row-title'); if (!b) return false; b.click(); return true; })()`)) await audit(`${tag}-track-page`);
}

if (args.has('--en')) {
  await evaluate(`localStorage.setItem('scdesk.lang', 'en'); location.reload(); 1`).catch(() => {});
  await sleep(9000);
}
if (args.has('--narrow')) await send('Emulation.setDeviceMetricsOverride', { width: 900, height: 620, deviceScaleFactor: 1, mobile: false });
const themes = args.has('--themes') ? ['dark', 'oled', 'light'] : [null];
for (const th of themes) {
  if (th) await evaluate(`document.documentElement.dataset.theme = ${JSON.stringify(th)}; 1`);
  await walk(th ?? 'now');
}
if (args.has('--narrow')) await send('Emulation.clearDeviceMetricsOverride');
if (args.has('--en')) await evaluate(`localStorage.removeItem('scdesk.lang'); location.reload(); 1`).catch(() => {});

const byKind = new Map();
for (const p of problems) byKind.set(p.kind, (byKind.get(p.kind) ?? 0) + 1);
console.log(`checked; screenshots in ${out}`);
if (!problems.length) console.log('no defects found');
for (const p of problems) console.log(`[${p.kind}] ${p.where}: ${p.text}`);
console.log('summary:', Object.fromEntries(byKind));
process.exit(problems.length ? 1 : 0);
