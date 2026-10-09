// Lyrics for the current track, shared by the side panel and the fullscreen view.
import { api, errorMessage, type Lyrics, type Track } from './api';

export type LyricsState =
  | { kind: 'idle' }
  | { kind: 'loading'; track: Track }
  | { kind: 'ready'; track: Track; lyrics: Lyrics }
  | { kind: 'error'; track: Track; message: string };

type Listener = (s: LyricsState) => void;

class LyricsData {
  state: LyricsState = { kind: 'idle' };
  private track: Track | null = null;
  private wanted = false;
  private seq = 0;
  private listeners = new Set<Listener>();
  /** ms the playing upload is shifted by against the original (Go+ stand-ins) */
  private shifts = new Map<number, number>();

  on(cb: Listener): void {
    this.listeners.add(cb);
  }

  /** Called when the playing track changes. */
  setTrack(track: Track | null): void {
    if (track?.id === this.track?.id) return;
    this.track = track;
    if (!track) return this.set({ kind: 'idle' });
    if (this.wanted) void this.load(false);
    else this.set({ kind: 'idle' });
  }

  /** Lyrics are fetched only while some view shows them. */
  setWanted(wanted: boolean): void {
    this.wanted = wanted;
    if (wanted && this.track && (this.state.kind === 'idle' || ('track' in this.state && this.state.track.id !== this.track.id))) {
      void this.load(false);
    }
  }

  /** The track plays through an upload with a longer intro: move the lyrics by `ms`. */
  setShift(trackId: number, ms: number): void {
    if ((this.shifts.get(trackId) ?? 0) === ms) return;
    this.shifts.set(trackId, ms);
    if (this.state.kind === 'ready' && this.state.track.id === trackId) this.set({ ...this.state });
  }

  reload(): void {
    void this.load(true);
  }

  private async load(force: boolean): Promise<void> {
    const track = this.track;
    if (!track) return;
    const seq = ++this.seq;
    this.set({ kind: 'loading', track });
    try {
      const lyrics = await api.lyrics(track, force);
      if (seq === this.seq) this.set({ kind: 'ready', track, lyrics });
    } catch (e) {
      if (seq === this.seq) this.set({ kind: 'error', track, message: errorMessage(e) });
    }
  }

  private set(s: LyricsState): void {
    if (s.kind === 'ready') s = { ...s, lyrics: shifted(s.lyrics, this.shifts.get(s.track.id) ?? 0) };
    this.state = s;
    this.listeners.forEach((cb) => cb(s));
  }
}

/** Lyrics with every timestamp moved by `ms` (a no-op for 0 or unsynced text). */
function shifted(l: Lyrics, ms: number): Lyrics {
  if (!ms || !l.synced || (l as Lyrics & { shiftedBy?: number }).shiftedBy === ms) return l;
  const prev = (l as Lyrics & { shiftedBy?: number }).shiftedBy ?? 0;
  const d = ms - prev;
  const mv = (t: number | null) => (t === null ? null : Math.max(0, t + d));
  return {
    ...l,
    shiftedBy: ms,
    lines: l.lines.map((x) => ({
      ...x,
      start_ms: mv(x.start_ms),
      end_ms: mv(x.end_ms),
      words: x.words.map((w) => ({ ...w, start_ms: w.start_ms + d, end_ms: w.end_ms + d })),
    })),
  } as Lyrics;
}

export const lyricsData = new LyricsData();

export const SOURCE_NAMES: Record<string, string> = {
  lrclib: 'LRCLIB',
  netease: 'NetEase',
  musixmatch: 'Musixmatch',
  genius: 'Genius',
};
