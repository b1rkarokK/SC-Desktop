// Holding Alt over a track row: a short listen from a third in, over the playing track (which goes quiet),
// and back as soon as Alt is let go or the mouse leaves the row. Nothing plays by a mere hover.
import { api, type Track } from './api';

type RowEl = HTMLElement & { __track?: Track };

let hovered: RowEl | null = null;
let playing: RowEl | null = null;

function stop(): void {
  if (!playing) return;
  playing.classList.remove('is-previewing');
  playing = null;
  void api.previewStop().catch(() => {});
}

export function initPreview(): void {
  document.addEventListener('mouseover', (e) => {
    const row = (e.target as HTMLElement).closest('.row') as RowEl | null;
    hovered = row?.__track ? row : null;
    if (playing && playing !== hovered) stop();
  });
  document.addEventListener('keydown', (e) => {
    if (e.key !== 'Alt' || e.repeat) return;
    e.preventDefault();
    const track = hovered?.__track;
    if (!hovered || !track || playing === hovered) return;
    stop();
    playing = hovered;
    playing.classList.add('is-previewing');
    void api.previewStart(track).catch(() => playing?.classList.remove('is-previewing'));
  });
  document.addEventListener('keyup', (e) => {
    if (e.key === 'Alt') {
      e.preventDefault();
      stop();
    }
  });
  window.addEventListener('blur', stop);
}
