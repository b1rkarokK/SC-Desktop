// Tiny in-app navigation: top-level sections reset the stack, detail pages
// (track / artist / playlist) push onto it so "Назад" works.

export type LibTab = 'tracks' | 'playlists' | 'albums' | 'artists';
export type ProfileTab = 'downloads' | 'queue';

export type Route =
  | { name: 'likes'; tab?: LibTab }
  | { name: 'wave' }
  | { name: 'history' }
  | { name: 'search' }
  | { name: 'settings' }
  | { name: 'profile'; tab?: ProfileTab }
  | { name: 'track'; id: number }
  | { name: 'artist'; id: number }
  | { name: 'playlist'; id: number };

export type Section = 'likes' | 'wave' | 'history' | 'search' | 'settings' | 'profile';

const TOP: ReadonlySet<string> = new Set(['likes', 'wave', 'history', 'search', 'settings', 'profile']);

class Router {
  private stack: Route[] = [{ name: 'likes' }];
  private listeners = new Set<(r: Route) => void>();

  get current(): Route {
    return this.stack[this.stack.length - 1]!;
  }

  get canGoBack(): boolean {
    return this.stack.length > 1;
  }

  /** Section highlighted in the nav (detail pages keep their origin section). */
  get section(): Section {
    for (let i = this.stack.length - 1; i >= 0; i--) {
      const r = this.stack[i]!;
      if (TOP.has(r.name)) return r.name as Section;
    }
    return 'likes';
  }

  go(route: Route): void {
    if (TOP.has(route.name)) this.stack = [route];
    else {
      const cur = this.current;
      if ('id' in cur && 'id' in route && cur.name === route.name && cur.id === route.id) return;
      this.stack.push(route);
      if (this.stack.length > 50) this.stack.splice(1, 1);
    }
    this.notify();
  }

  back(): void {
    if (this.stack.length > 1) {
      this.stack.pop();
      this.notify();
    }
  }

  on(cb: (r: Route) => void): void {
    this.listeners.add(cb);
  }

  private notify(): void {
    this.listeners.forEach((cb) => cb(this.current));
  }
}

export const router = new Router();

export function openTrack(id: number): void {
  router.go({ name: 'track', id });
}

export function openArtist(id: number): void {
  if (id) router.go({ name: 'artist', id });
}

/**
 * The shown artist can be a credit from the label metadata (SHTRIHCOD) while
 * `user_id` is the uploader (свет). Look the credited name up first and open
 * that profile on an exact name match, otherwise the uploader.
 */
export async function openTrackArtist(t: { artist: string; user_id: number }): Promise<void> {
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    const users = await invoke<{ id: number; username: string }[]>('search_users', { query: t.artist });
    const want = t.artist.trim().toLowerCase();
    const exact = users.find((u) => u.username.trim().toLowerCase() === want);
    if (exact) return openArtist(exact.id);
  } catch {
    /* fall back to the uploader */
  }
  openArtist(t.user_id);
}

export function openPlaylist(id: number): void {
  router.go({ name: 'playlist', id });
}
