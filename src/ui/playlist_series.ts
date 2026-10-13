// SoundCloud holds up to 500 songs in a playlist. A longer one is kept as a series of playlists named
// «Name (1/3)», «Name (2/3)»… (the import makes them so); here the series is shown and used as one playlist.
import { api, type Playlist, type PlaylistPage, type Track } from './api';

export type Series = Playlist & { parts?: Playlist[] };

const PART = /^(.*\S)\s+\((\d+)\/(\d+)\)$/;

/** Own playlists that are parts of one series become a single entry (the others stay as they are). */
export function groupSeries(items: Playlist[]): Series[] {
  const groups = new Map<string, { n: number; p: Playlist }[]>();
  const out: (Series | string)[] = [];
  for (const p of items) {
    const m = p.own && !p.is_album ? PART.exec(p.title) : null;
    if (!m) {
      out.push(p);
      continue;
    }
    const key = `${m[1]}\u0000${m[3]}\u0000${p.user_id}`;
    const list = groups.get(key) ?? [];
    if (!list.length) out.push(key);
    list.push({ n: Number(m[2]), p });
    groups.set(key, list);
  }
  return out.flatMap((e): Series[] => {
    if (typeof e !== 'string') return [e];
    const parts = (groups.get(e) ?? []).sort((a, b) => a.n - b.n).map((x) => x.p);
    const first = parts[0];
    if (!first || parts.length < 2) return parts; // a lone «(1/3)»: the other parts are not here, show it as it is
    return [{ ...first, title: PART.exec(first.title)?.[1] ?? first.title, track_count: parts.reduce((n, p) => n + p.track_count, 0), parts }];
  });
}

/** The page of a playlist; of a series, its parts one after another. */
export async function loadSeries(p: Series): Promise<PlaylistPage> {
  if (!p.parts) return api.playlistPage(p.id);
  const pages = await Promise.all(p.parts.map((x) => api.playlistPage(x.id)));
  const tracks: Track[] = pages.flatMap((x) => x.tracks);
  return { playlist: { ...p, track_count: tracks.length }, tracks };
}
