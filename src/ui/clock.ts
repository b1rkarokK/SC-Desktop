// Playback clock: the backend position is polled at ≤4 Hz; between polls the
// position is extrapolated with performance.now() so karaoke stays smooth.

type Listener = () => void;

class Clock {
  private base = 0;
  private at = 0;
  playing = false;
  private listeners = new Set<Listener>();

  set(ms: number, playing: boolean): void {
    // ignore tiny backwards jitter from the 250 ms audio-thread tick
    const predicted = this.now();
    const jump = Math.abs(predicted - ms) > 600 || playing !== this.playing;
    if (jump || ms > predicted) {
      this.base = ms;
      this.at = performance.now();
    }
    const changed = playing !== this.playing;
    this.playing = playing;
    if (changed || jump) this.listeners.forEach((cb) => cb());
  }

  /** Hard reset (seek / new track). */
  reset(ms: number): void {
    this.base = ms;
    this.at = performance.now();
    this.listeners.forEach((cb) => cb());
  }

  now(): number {
    return this.playing ? this.base + (performance.now() - this.at) : this.base;
  }

  on(cb: Listener): void {
    this.listeners.add(cb);
  }
}

export const clock = new Clock();
