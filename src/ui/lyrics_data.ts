// Lyrics for the current track, shared by the side panel and the fullscreen view.
import { api, errorMessage, type Lyrics, type Track } from './api';
import { prefs, updatePrefs } from './prefs';

export type LyricsState =
  | { kind: 'idle' }
  | { kind: 'loading'; track: Track }
  /** no lyrics under the name: the app is listening to the track to find the song by its words */
  | { kind: 'hearing'; track: Track }
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
  /** ms the sound says the lyrics of a track are off by (lyrics_align) */
  private autos = new Map<number, number>();

  /** Everything the lyrics of a track are moved by: stand-in, the sound, and the user's own nudge. */
  private total(id: number): number {
    return (this.shifts.get(id) ?? 0) + (this.autos.get(id) ?? 0) + (prefs().lyricsNudge?.[String(id)] ?? 0);
  }

  /** What moves the lyrics of a track now, for the panel: by the sound, and by hand. */
  info(id: number): { auto: number; nudge: number } {
    return { auto: this.autos.get(id) ?? 0, nudge: prefs().lyricsNudge?.[String(id)] ?? 0 };
  }

  /** The user moves the lyrics of the playing track by `delta` ms (kept for next time); 0 puts it back. */
  nudge(id: number, delta: number): void {
    updatePrefs((p) => {
      const all = { ...(p.lyricsNudge ?? {}) };
      const next = delta === 0 ? 0 : Math.max(-15_000, Math.min(15_000, (all[String(id)] ?? 0) + delta));
      if (next === 0) delete all[String(id)];
      else all[String(id)] = next;
      // the newest 300 tracks are remembered
      const keys = Object.keys(all);
      for (const k of keys.slice(0, Math.max(0, keys.length - 300))) delete all[k];
      p.lyricsNudge = all;
    });
    if (this.state.kind === 'ready' && this.state.track.id === id) this.set({ ...this.state });
  }

  /** Asks the sound how far off the timed lyrics are; the audio may not be there yet, so a few tries. */
  private async align(track: Track): Promise<void> {
    if (prefs().lyricsAuto === false) return;
    for (const wait of [3000, 7000, 20000]) {
      await new Promise((r) => window.setTimeout(r, wait));
      if (this.track?.id !== track.id) return;
      const r = await api.lyricsAlign(track.id).catch(() => null);
      if (r === null) continue;
      if (r.offset_ms && this.state.kind === 'ready' && this.state.track.id === track.id) {
        this.autos.set(track.id, r.offset_ms);
        this.set({ ...this.state });
      }
      return;
    }
  }

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
      if (seq !== this.seq) return;
      if (!lyrics.found && prefs().lyricsHear !== false) return void this.hear(track, lyrics, force, seq);
      this.ready(track, lyrics);
    } catch (e) {
      if (seq === this.seq) this.set({ kind: 'error', track, message: errorMessage(e) });
    }
  }

  private ready(track: Track, lyrics: Lyrics): void {
    this.set({ kind: 'ready', track, lyrics });
    if (lyrics.found && lyrics.synced && !lyrics.original && !this.autos.has(track.id)) void this.align(track);
  }

  /** Listens to the track (the audio may still be loading, so a few tries) and shows what that finds. */
  private async hear(track: Track, none: Lyrics, force: boolean, seq: number): Promise<void> {
    this.set({ kind: 'hearing', track });
    for (const wait of [0, 3000, 7000]) {
      if (wait) await new Promise((r) => window.setTimeout(r, wait));
      if (seq !== this.seq) return;
      const r = await api.lyricsHear(track, force).catch(() => null);
      if (seq !== this.seq) return;
      if (r?.wait) continue;
      return this.ready(track, r?.lyrics ?? none);
    }
    if (seq === this.seq) this.ready(track, none);
  }

  private set(s: LyricsState): void {
    if (s.kind === 'ready') s = { ...s, lyrics: shifted(s.lyrics, this.total(s.track.id)) };
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
