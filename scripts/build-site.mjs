// Builds the site (docs/index.html, docs/en/index.html) from one template. Run: node scripts/build-site.mjs
// The page follows the visitor's system theme (light / dark), shows the app's screenshots in the same
// theme, and takes the latest version and the download link from GitHub, so a new release needs no edit.
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..', 'docs');
const REPO = 'b1rkarokK/SC-Desktop';

const L = {
  ru: {
    lang: 'ru',
    title: 'SC Desk: SoundCloud без рекламы для Windows',
    description: 'SC Desk - бесплатный клиент SoundCloud для Windows без рекламы: умная волна, комнаты для совместного прослушивания, общие плейлисты, тексты песен по словам.',
    navFeatures: 'Возможности',
    navDownload: 'Скачать',
    h1a: 'SoundCloud на компьютере.',
    h1b: 'Без рекламы.',
    sub: 'Волна под ваш вкус, комнаты для совместного прослушивания, общие плейлисты и тексты песен по словам.',
    cta: 'Скачать для Windows',
    src: 'Исходный код',
    meta: 'Бесплатно · Windows 10 и 11',
    shotAlt: ['Моя волна', 'Комнаты', 'Общие плейлисты', 'Тексты песен'],
    steps: [
      ['Моя волна', 'Подборка, которая учится на ваших лайках, пропусках и на том, как звучат треки. Настроение на выбор: больше нового, только знакомое, спокойнее или энергичнее.'],
      ['Комнаты', 'Слушайте вместе с друзьями: один трек у всех одновременно, с точностью до долей секунды. Звук и переписка не передаются, только команды. Комната живёт, даже когда в ней никого нет.'],
      ['Общие плейлисты', 'Один список на всех. Видно, кто что добавил, есть история изменений и роли. Оформите его картинкой или цветом и нажмите «Играть вместе».'],
      ['Тексты', 'Тексты песен по строкам и по словам, полноэкранный режим и поиск ваших лайков по строчке из песни.'],
    ],
    of: 'из',
    features: [
      ['Без рекламы', 'Только музыка: никаких вставок между треками.'],
      ['Всё у вас', 'Лайки, история и настройки хранятся на вашем компьютере. Комнаты и общие плейлисты шифруются на ваших устройствах.'],
      ['Открытый код', 'Код программы можно прочитать и проверить на GitHub.'],
    ],
    dlTitle: 'Скачать SC Desk',
    dlText: 'Установщик для Windows 10 и 11. Дальше программа обновляется сама.',
    dlBtn: 'Скачать для Windows',
    version: 'Версия',
    size: 'МБ',
    hash: 'Контрольная сумма SHA-256',
    verifyQ: 'Проверить скачанный файл',
    warnQ: 'Windows пишет «Неизвестный издатель»?',
    warnA: 'У программы пока нет цифровой подписи, поэтому SmartScreen может предупредить. Нажмите «Подробнее», затем «Выполнить в любом случае». Исходный код открыт, а контрольную сумму файла можно сверить командой Get-FileHash.',
    madeBy: 'Made by b1rkarokK',
    unofficial: 'Неофициальный клиент, не связан с SoundCloud.',
    otherLang: ['English', 'en/'],
    otherLangBack: ['Русский', '../'],
  },
  en: {
    lang: 'en',
    title: 'SC Desk: SoundCloud without ads for Windows',
    description: 'SC Desk is a free SoundCloud client for Windows without ads: a smart wave, rooms to listen together, shared playlists, word-by-word lyrics.',
    navFeatures: 'Features',
    navDownload: 'Download',
    h1a: 'SoundCloud on your PC.',
    h1b: 'Without ads.',
    sub: 'A wave for your taste, rooms to listen together, shared playlists and word-by-word lyrics.',
    cta: 'Download for Windows',
    src: 'Source code',
    meta: 'Free · Windows 10 and 11',
    shotAlt: ['My Wave', 'Rooms', 'Shared playlists', 'Lyrics'],
    steps: [
      ['My Wave', 'A selection that learns from your likes, skips and how tracks sound. Pick the mood: more new, only familiar, calmer or more energetic.'],
      ['Rooms', 'Listen together with friends: one track at everyone\'s place at once, to a fraction of a second. Audio and chat are not sent, only commands. A room lives on even when nobody is in it.'],
      ['Shared playlists', 'One list for everybody. You see who added what, there is a history of changes and roles. Give it a picture or a colour and press "Play together".'],
      ['Lyrics', 'Lyrics by line and by word, a full-screen mode, and search of your likes by a line of a song.'],
    ],
    of: 'of',
    features: [
      ['No ads', 'Only music: nothing between the tracks.'],
      ['Yours alone', 'Likes, history and settings stay on your computer. Rooms and shared playlists are encrypted on your devices.'],
      ['Open source', 'The code can be read and checked on GitHub.'],
    ],
    dlTitle: 'Download SC Desk',
    dlText: 'Installer for Windows 10 and 11. After that the app updates itself.',
    dlBtn: 'Download for Windows',
    version: 'Version',
    size: 'MB',
    hash: 'SHA-256 checksum',
    verifyQ: 'Check the downloaded file',
    warnQ: 'Windows says "Unknown publisher"?',
    warnA: 'The app has no digital signature yet, so SmartScreen may warn. Click "More info", then "Run anyway". The source code is open, and you can check the file\'s checksum with Get-FileHash.',
    madeBy: 'Made by b1rkarokK',
    unofficial: 'An unofficial client, not affiliated with SoundCloud.',
    otherLang: ['Русский', '../'],
    otherLangBack: ['Русский', '../'],
  },
};

// where the cursor goes on each screenshot (percent of the picture), measured on the real frames
const HOT = [
  [23, 27],
  [70.4, 16.7],
  [20, 40],
  [84.5, 15.2],
];
const NAMES = ['wave', 'rooms', 'shared', 'lyrics'];

function page(c, prefix, isEn) {
  const a = prefix; // path to docs/ from this page
  const pic = (name, i, cls = '') =>
    `<picture><source media="(prefers-color-scheme: light)" srcset="${a}assets/${name}-light.webp"><img class="${cls}" src="${a}assets/${name}-dark.webp" width="2000" height="1267" alt="${c.shotAlt[i]}"${i === 0 ? '' : ' loading="lazy"'} decoding="async"></picture>`;
  const caps = c.steps
    .map(([t, d], i) => `<li class="cap${i === 0 ? ' is-on' : ''}" data-step="${i}"><span class="step-no">${i + 1} ${c.of} ${c.steps.length}</span><h3>${t}</h3><p>${d}</p></li>`)
    .join('\n          ');
  const shots = NAMES.map((n, i) => `<div class="shot${i === 0 ? ' is-on' : ''}" data-step="${i}" data-x="${HOT[i][0]}" data-y="${HOT[i][1]}">${pic(n, i)}</div>`).join('\n            ');
  const ticks = NAMES.map((_, i) => `<span class="tick${i === 0 ? ' is-on' : ''}"></span>`).join('');
  const feats = c.features.map(([t, d]) => `<li><h3>${t}</h3><p>${d}</p></li>`).join('\n        ');
  const switchLink = isEn ? `<a class="lang" href="${c.otherLangBack[1]}" hreflang="ru" lang="ru">${c.otherLangBack[0]}</a>` : `<a class="lang" href="${c.otherLang[1]}" hreflang="en" lang="en">${c.otherLang[0]}</a>`;
  return `<!doctype html>
<html lang="${c.lang}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light dark">
<title>${c.title}</title>
<meta name="description" content="${c.description}">
<meta property="og:title" content="${c.title}">
<meta property="og:description" content="${c.description}">
<meta property="og:image" content="${a}assets/rooms-dark.webp">
<link rel="icon" href="${a}assets/favicon.png">
<link rel="apple-touch-icon" href="${a}assets/icon.png">
<link rel="stylesheet" href="${a}assets/site.css">
</head>
<body>
<header class="top">
  <a class="brand" href="#top"><img src="${a}assets/icon.png" alt="" width="24" height="24"><span>SC <b>Desk</b></span></a>
  <nav><a href="#story">${c.navFeatures}</a><a href="https://github.com/${REPO}">GitHub</a>${switchLink}</nav>
  <a class="btn primary dl-link" href="https://github.com/${REPO}/releases/latest" data-download>${c.navDownload}<small class="ver"></small></a>
</header>

<main id="top">
  <section class="hero">
    <h1>${c.h1a}<br><span>${c.h1b}</span></h1>
    <p class="lead">${c.sub}</p>
    <div class="cta">
      <a class="btn primary big dl-link" href="https://github.com/${REPO}/releases/latest" data-download>${c.cta}<small class="ver"></small></a>
      <a class="btn big" href="https://github.com/${REPO}">${c.src}</a>
    </div>
    <p class="meta"><span>${c.meta}</span><span class="meta-size"></span></p>
    <div class="window hero-window">
      <div class="winbar"><i></i><i></i><i></i></div>
      ${pic('wave', 0)}
    </div>
  </section>

  <section class="story" id="story" aria-label="${c.navFeatures}">
    <div class="story-sticky">
      <div class="story-copy">
        <ol class="caps">
          ${caps}
        </ol>
        <div class="ticks" aria-hidden="true">${ticks}</div>
      </div>
      <div class="stage">
        <div class="window">
          <div class="winbar"><i></i><i></i><i></i></div>
          <div class="shots">
            ${shots}
            <span class="cursor" aria-hidden="true"><svg viewBox="0 0 24 24" width="26" height="26"><path d="M5 3l14 8-6 1.6L9.6 19z"/></svg></span>
          </div>
        </div>
      </div>
    </div>
  </section>

  <section class="features">
    <ul>
        ${feats}
    </ul>
  </section>

  <section class="download" id="download">
    <h2>${c.dlTitle}</h2>
    <p>${c.dlText}</p>
    <a class="btn primary big dl-link" href="https://github.com/${REPO}/releases/latest" data-download>${c.dlBtn}<small class="ver"></small></a>
    <details>
      <summary>${c.warnQ}</summary>
      <p>${c.warnA}</p>
    </details>
    <details class="verify" hidden>
      <summary>${c.verifyQ}</summary>
      <dl class="facts">
        <div><dt>${c.version}</dt><dd class="f-ver"></dd></div>
        <div><dt>${c.size}</dt><dd class="f-size"></dd></div>
        <div class="wide"><dt>${c.hash}</dt><dd class="f-hash"></dd></div>
      </dl>
    </details>
  </section>
</main>

<footer>
  <span>${c.madeBy}</span>
  <span>${c.unofficial}</span>
  <a href="https://github.com/${REPO}">GitHub</a>
</footer>
<script src="${a}assets/site.js" defer></script>
</body>
</html>
`;
}

mkdirSync(join(root, 'en'), { recursive: true });
writeFileSync(join(root, 'index.html'), page(L.ru, '', false));
writeFileSync(join(root, 'en', 'index.html'), page(L.en, '../', true));
console.log('site built');
