import { I } from './icons';

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
    title: 'Только в SC Desk',
    items: [
      pick('radar', 'Андеграунд-радар', 'малоизвестные артисты в твоём вкусе', I.catRadar),
      pick('forgotten', 'Забытые лайки', 'лайкнул давно и не слушал', I.catHeartCrack),
      pick('year-ago', 'Год назад', 'что ты лайкал в это время год назад', I.catCalendarClock),
      pick('repeat', 'На повторе', 'самое заслушанное за месяц', I.catRepeat2),
    ],
  },
  {
    title: 'Чарты',
    items: [
      { key: 'chart:ru:day', title: 'Сегодня в России', chart: 'ru:day', note: 'чарт Яндекс Музыки за сутки', icon: I.catTrendingUp },
      { key: 'chart:ru:week', title: 'Хиты недели', chart: 'ru:week', note: 'сумма слушателей за 7 дней', icon: I.catCalendarClock },
      { key: 'chart:ru:month', title: 'Хиты месяца', chart: 'ru:month', note: 'сумма слушателей за 30 дней', icon: I.catCalendarClock },
      { key: 'chart:ru:year', title: 'Хиты года', chart: 'ru:year', note: 'сумма слушателей за год', icon: I.catCalendarClock },
      { key: 'chart:world:day', title: 'Сегодня в мире', chart: 'world:day', note: 'глобальный чарт за сутки', icon: I.catAudioLines },
      { key: 'chart:years', title: 'По годам', chart: 'ru:2025', note: 'итоги каждого года с 2001', mark: '01-25' },
      { key: 'chart:all', title: 'Тренды SoundCloud', mix: trend('all-genres'), icon: I.catTrendingUp },
      genre('hip-hop', 'Топ хип-хопа', I.catMicVocal),
      genre('pop', 'Топ попа', I.catMusic2),
      genre('rock', 'Топ рока', I.catGuitar),
      genre('electronic', 'Топ электроники', I.catZap),
    ],
  },
  {
    title: 'Жанры',
    items: [
      genre('trap', 'Трэп', I.catSpeaker),
      genre('alternative-hip-hop', 'Альтернативный хип-хоп', I.catMic),
      genre('alternative-rock', 'Альтернативный рок', I.catGuitar),
      genre('indie', 'Инди', I.catHeadphones),
      genre('punk', 'Панк', I.catSkull),
      genre('house', 'Хаус', I.catDisc3),
      genre('techno', 'Техно', I.catCpu),
      genre('edm', 'EDM', I.catAudioLines),
      genre('dubstep', 'Дабстеп', I.catRadio),
      genre('drum--n--bass', 'Драм-н-бейс', I.catDrum),
      genre('trance', 'Транс', I.catWaves),
      genre('hardcore', 'Хардкор', I.catFlame),
      genre('breakbeat', 'Брейкбит', I.catDrum),
      genre('industrial', 'Индастриал', I.catFactory),
      genre('ambient', 'Эмбиент', I.catMoon),
    ],
  },
  {
    title: 'Эпохи',
    items: [
      theme('era:80', '80-е', { mark: '80' }, 'хиты 80-х', '80s hits'),
      theme('era:90', '90-е', { mark: '90' }, 'хиты 90-х', '90s hits'),
      theme('era:2000', '2000-е', { mark: '00' }, 'хиты 2000-х', '2000s hits'),
      theme('era:2010', '2010-е', { mark: '10' }, 'хиты 2010-х', '2010s hits'),
      theme('era:forever', 'Вечные хиты', { mark: '∞' }, 'вечные хиты', 'ностальгия хиты', 'greatest hits of all time'),
    ],
  },
  {
    title: 'Русское',
    items: [
      theme('ru:rap', 'Русский рэп', { icon: I.catMicVocal }, 'русский рэп', 'русский рэп хиты'),
      theme('ru:rock', 'Русский рок', { icon: I.catGuitar }, 'русский рок', 'русский рок 90-х'),
      theme('ru:pop', 'Русская поп-музыка', { icon: I.catMusic2 }, 'русская поп музыка', 'русские хиты'),
      theme('ru:90', 'Русские хиты 90-х', { mark: '90' }, 'русские хиты 90-х', 'русская дискотека 90-х'),
    ],
  },
  {
    title: 'Нейромузыка',
    items: [
      theme('ai:all', 'AI-треки', { icon: I.catBot }, 'suno ai', 'ai music', 'udio ai', 'нейросеть песни'),
      theme('ai:covers', 'AI-каверы', { icon: I.catSparkles }, 'ai cover', 'ai кавер'),
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
