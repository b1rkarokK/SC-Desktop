// Lucide icons, bundled locally (tree-shaken), monochrome via currentColor.
import {
  AudioWaveform,
  Check,
  ChartColumn,
  ChevronDown,
  ChevronLeft,
  CircleHelp,
  createElement,
  Clock,
  Download,
  ExternalLink,
  FolderOpen,
  Heart,
  GripVertical,
  History,
  House,
  ListOrdered,
  ImagePlus,
  Pencil,
  PictureInPicture2,
  Trash2,
  Upload,
  Link,
  ListEnd,
  ListMusic,
  ListPlus,
  ListStart,
  Music,
  User,
  LogIn,
  Maximize2,
  Minimize2,
  Palette,
  PanelLeftClose,
  PanelLeftOpen,
  Pause,
  Play,
  Plus,
  RefreshCw,
  Repeat,
  Repeat1,
  Search,
  Settings,
  Shuffle,
  SkipBack,
  SkipForward,
  SlidersHorizontal,
  ThumbsDown,
  Type,
  UserCheck,
  UserX,
  Volume1,
  Volume2,
  VolumeX,
  X,
  Mic,
  MicVocal,
  Guitar,
  Music2,
  Zap,
  Radio,
  Headphones,
  Disc3,
  Drum,
  Waves,
  Factory,
  Flame,
  Sparkles,
  Bot,
  Radar,
  CalendarClock,
  Repeat2,
  HeartCrack,
  Skull,
  TrendingUp,
  AudioLines,
  Speaker,
  Cpu,
  Moon,
  Ghost,
  type IconNode,
} from 'lucide';

export const I = {
  catMic: Mic,
  catMicVocal: MicVocal,
  catGuitar: Guitar,
  catMusic2: Music2,
  catZap: Zap,
  catRadio: Radio,
  catHeadphones: Headphones,
  catDisc3: Disc3,
  catDrum: Drum,
  catWaves: Waves,
  catFactory: Factory,
  catFlame: Flame,
  catSparkles: Sparkles,
  catBot: Bot,
  catRadar: Radar,
  catCalendarClock: CalendarClock,
  catRepeat2: Repeat2,
  catHeartCrack: HeartCrack,
  catSkull: Skull,
  catTrendingUp: TrendingUp,
  catAudioLines: AudioLines,
  catSpeaker: Speaker,
  catCpu: Cpu,
  catMoon: Moon,
  catGhost: Ghost,

  wave: AudioWaveform,
  check: Check,
  chevronDown: ChevronDown,
  back: ChevronLeft,
  help: CircleHelp,
  download: Download,
  external: ExternalLink,
  folder: FolderOpen,
  heart: Heart,
  history: History,
  image: ImagePlus,
  trash: Trash2,
  upload: Upload,
  home: House,
  timecode: Clock,
  link: Link,
  queueEnd: ListEnd,
  queueNext: ListStart,
  playlist: ListMusic,
  playlistAdd: ListPlus,
  smartShuffle: Sparkles,
  track: Music,
  user: User,
  login: LogIn,
  fullscreen: Maximize2,
  exitFullscreen: Minimize2,
  palette: Palette,
  navCollapse: PanelLeftClose,
  navExpand: PanelLeftOpen,
  pause: Pause,
  play: Play,
  plus: Plus,
  refresh: RefreshCw,
  repeat: Repeat,
  repeatOne: Repeat1,
  search: Search,
  settings: Settings,
  shuffle: Shuffle,
  prev: SkipBack,
  next: SkipForward,
  eq: SlidersHorizontal,
  dislike: ThumbsDown,
  lyrics: Type,
  following: UserCheck,
  banArtist: UserX,
  volLow: Volume1,
  vol: Volume2,
  mute: VolumeX,
  close: X,
  sleep: Moon,
  grip: GripVertical,
  mini: PictureInPicture2,
  stats: ChartColumn,
  edit: Pencil,
  queue: ListOrdered,
} satisfies Record<string, IconNode>;

export function icon(node: IconNode, size = 16): SVGElement {
  const el = createElement(node);
  el.setAttribute('width', String(size));
  el.setAttribute('height', String(size));
  el.setAttribute('stroke-width', size <= 16 ? '1.75' : '2');
  el.setAttribute('aria-hidden', 'true');
  el.classList.add('icon');
  return el;
}

/** Icon-only button with a tooltip / accessible label. */
export function iconButton(node: IconNode, label: string, className = 'btn-icon'): HTMLButtonElement {
  const b = document.createElement('button');
  b.type = 'button';
  b.className = className;
  b.title = label;
  b.setAttribute('aria-label', label);
  b.append(icon(node));
  return b;
}

export function setIcon(button: HTMLElement, node: IconNode, size = 16): void {
  button.replaceChildren(icon(node, size));
}

/** SoundCloud cloud mark — flat #ff5500, no gradient (design rule). */
export function scLogo(size = 28): SVGElement {
  const ns = 'http://www.w3.org/2000/svg';
  const svg = document.createElementNS(ns, 'svg');
  svg.setAttribute('viewBox', '0 0 32 16');
  svg.setAttribute('width', String(size));
  svg.setAttribute('height', String(size / 2));
  svg.setAttribute('aria-hidden', 'true');
  svg.classList.add('sc-logo');
  const bars = [
    [0, 9, 4], [2, 7, 7], [4, 5, 10], [6, 4, 11], [8, 3, 12], [10, 3, 12], [12, 2, 13],
  ];
  for (const [x, y, h] of bars) {
    const r = document.createElementNS(ns, 'rect');
    r.setAttribute('x', String(x));
    r.setAttribute('y', String(y));
    r.setAttribute('width', '1.2');
    r.setAttribute('height', String(h));
    r.setAttribute('rx', '0.6');
    svg.append(r);
  }
  const cloud = document.createElementNS(ns, 'path');
  cloud.setAttribute('d', 'M14 2.2c1-.7 2.2-1.2 3.6-1.2 3.2 0 5.9 2.4 6.2 5.5.4-.2.9-.3 1.4-.3 2.6 0 4.8 2.1 4.8 4.8S27.8 15.8 25.2 15.8H14z');
  svg.append(cloud);
  return svg;
}
