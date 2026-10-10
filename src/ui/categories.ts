import { I } from './icons';
import { T } from './i18n';

// «Категории» like Yandex Music: charts, genres, eras, Russian music, AI music.
// Genre tiles open SoundCloud's live "trending by genre" charts (50 tracks,
// updated daily); the others collect the most liked playlists for a theme.

export interface Category {
  key: string;
  title: string;
  /** SoundCloud system playlist (a live chart) */
  mix?: string;
  /** otherwise: searches whose most liked playlists make up the page */
  queries?: string[];
  /** or a pick made by the app itself (see picks.rs) */
  pick?: string;
  /** or a real chart matched onto SoundCloud (see charts.rs) */
  chart?: string;
  /** one line under the title */
  note?: string;
  /** tile art: an icon, or big text ("90", "∞") */
  icon?: (typeof I)[keyof typeof I];
  mark?: string;
}

export interface CategoryGroup {
  title: string;
  items: Category[];
}

const trend = (slug: string) => `soundcloud:system-playlists:trending-by-genre:${slug}`;

const genre = (slug: string, title: string, icon: Category['icon']): Category => ({ key: `genre:${slug}`, title, mix: trend(slug), icon });

const theme = (key: string, title: string, art: { icon?: Category['icon']; mark?: string }, ...queries: string[]): Category => ({ key, title, queries, ...art });

const pick = (kind: string, title: string, note: string, icon: Category['icon']): Category => ({ key: `pick:${kind}`, title, pick: kind, note, icon });

export const CATEGORY_GROUPS: CategoryGroup[] = [
  {
    title: T('Только в SC Desk'),
    items: [
      pick('radar', T('Андеграунд-радар'), T('малоизвестные артисты в твоём вкусе'), I.catRadar),
      pick('forgotten', T('Забытые лайки'), T('лайкнул давно и не слушал'), I.catHeartCrack),
      pick('year-ago', T('Год назад'), T('что ты лайкал в это время год назад'), I.catCalendarClock),
      pick('repeat', T('На повторе'), T('самое заслушанное за месяц'), I.catRepeat2),
    ],
  },
  {
    title: T('Чарты'),
    items: [
      { key: 'chart:ru:day', title: T('Сегодня в России'), chart: 'ru:day', note: T('чарт Яндекс Музыки за сутки'), icon: I.catTrendingUp },
      { key: 'chart:ru:week', title: T('Хиты недели'), chart: 'ru:week', note: T('сумма слушателей за 7 дней'), icon: I.catCalendarClock },
      { key: 'chart:ru:month', title: T('Хиты месяца'), chart: 'ru:month', note: T('сумма слушателей за 30 дней'), icon: I.catCalendarClock },
      { key: 'chart:ru:year', title: T('Хиты года'), chart: 'ru:year', note: T('сумма слушателей за год'), icon: I.catCalendarClock },
      { key: 'chart:world:day', title: T('Сегодня в мире'), chart: 'world:day', note: T('глобальный чарт за сутки'), icon: I.catAudioLines },
      { key: 'chart:years', title: T('По годам'), chart: 'ru:2025', note: T('итоги каждого года с 2001'), mark: '01-25' },
      { key: 'chart:all', title: T('Тренды SoundCloud'), mix: trend('all-genres'), icon: I.catTrendingUp },
      genre('hip-hop', T('Топ хип-хопа'), I.catMicVocal),
      genre('pop', T('Топ попа'), I.catMusic2),
      genre('rock', T('Топ рока'), I.catGuitar),
      genre('electronic', T('Топ электроники'), I.catZap),
    ],
  },
  {
    title: T('Жанры'),
    items: [
      genre('trap', T('Трэп'), I.catSpeaker),
      genre('alternative-hip-hop', T('Альтернативный хип-хоп'), I.catMic),
      genre('alternative-rock', T('Альтернативный рок'), I.catGuitar),
      genre('indie', T('Инди'), I.catHeadphones),
      genre('punk', T('Панк'), I.catSkull),
      genre('house', T('Хаус'), I.catDisc3),
      genre('techno', T('Техно'), I.catCpu),
      genre('edm', 'EDM', I.catAudioLines),
      genre('dubstep', T('Дабстеп'), I.catRadio),
      genre('drum--n--bass', T('Драм-н-бейс'), I.catDrum),
      genre('trance', T('Транс'), I.catWaves),
      genre('hardcore', T('Хардкор'), I.catFlame),
      genre('breakbeat', T('Брейкбит'), I.catDrum),
      genre('industrial', T('Индастриал'), I.catFactory),
      genre('ambient', T('Эмбиент'), I.catMoon),
    ],
  },
  {
    title: T('Эпохи'),
    items: [
      theme('era:80', T('80-е'), { mark: '80' }, 'хиты 80-х', '80s hits'),
      theme('era:90', T('90-е'), { mark: '90' }, 'хиты 90-х', '90s hits'),
      theme('era:2000', T('2000-е'), { mark: '00' }, 'хиты 2000-х', '2000s hits'),
      theme('era:2010', T('2010-е'), { mark: '10' }, 'хиты 2010-х', '2010s hits'),
      theme('era:forever', T('Вечные хиты'), { mark: '∞' }, 'вечные хиты', 'ностальгия хиты', 'greatest hits of all time'),
    ],
  },
  {
    title: T('Русское'),
    items: [
      theme('ru:rap', T('Русский рэп'), { icon: I.catMicVocal }, 'русский рэп', 'русский рэп хиты'),
      theme('ru:rock', T('Русский рок'), { icon: I.catGuitar }, 'русский рок', 'русский рок 90-х'),
      theme('ru:pop', T('Русская поп-музыка'), { icon: I.catMusic2 }, 'русская поп музыка', 'русские хиты'),
      theme('ru:90', T('Русские хиты 90-х'), { mark: '90' }, 'русские хиты 90-х', 'русская дискотека 90-х'),
    ],
  },
  {
    title: T('Нейромузыка'),
    items: [
      theme('ai:all', T('AI-треки'), { icon: I.catBot }, 'suno ai', 'ai music', 'udio ai', 'нейросеть песни'),
      theme('ai:covers', T('AI-каверы'), { icon: I.catSparkles }, 'ai cover', 'ai кавер'),
    ],
  },
];

export function findCategory(key: string): Category | undefined {
  for (const g of CATEGORY_GROUPS) {
    const c = g.items.find((i) => i.key === key);
    if (c) return c;
  }
  return undefined;
}
