// The wheel over the volume: 1 % a notch, so that the level can be set exactly. The level is kept here and updated
// at once (reading it from the player's snapshot made fast turns stand on one value), a touchpad's small steps add up,
// and the setting is saved a moment after the last turn, not at every notch.
import { store } from './store';

const NOTCH = 100; // deltaY of one notch of a mouse wheel
const IDLE_MS = 600;
const SAVE_MS = 400;

let level: number | null = null;
let stamp = 0;
let rest = 0;
let saver: number | undefined;

/** `apply(v, persist)` sets the volume (the UI and the player); returns true when the event was used. */
export function wheelVolume(e: WheelEvent, apply: (v: number, persist: boolean) => void): void {
  e.preventDefault();
  const now = performance.now();
  // a pause: start again from what the volume really is (a slider, the keys or the phone may have changed it)
  if (level === null || now - stamp > IDLE_MS) {
    level = store.snapshot?.volume ?? 0;
    rest = 0;
  }
  stamp = now;
  rest += -e.deltaY / NOTCH;
  const steps = Math.trunc(rest);
  if (steps === 0) return;
  rest -= steps;
  level = Math.min(1, Math.max(0, Math.round((level + steps / 100) * 100) / 100));
  if (store.snapshot) store.snapshot.volume = level;
  const v = level;
  apply(v, false);
  window.clearTimeout(saver);
  saver = window.setTimeout(() => apply(v, true), SAVE_MS);
}
