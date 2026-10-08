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
    this.state = s;
    this.listeners.forEach((cb) => cb(s));
  }
}

export const lyricsData = new LyricsData();

export const SOURCE_NAMES: Record<string, string> = {
  lrclib: 'LRCLIB',
  netease: 'NetEase',
  musixmatch: 'Musixmatch',
  genius: 'Genius',
};
