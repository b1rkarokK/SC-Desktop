// Tiny in-app navigation: top-level sections reset the stack, detail pages
// (track / artist / playlist) push onto it so "Назад" works.

export type LibTab = 'tracks' | 'playlists' | 'albums' | 'artists';

export type Route =
  | { name: 'likes'; tab?: LibTab }
  | { name: 'wave' }
  | { name: 'history' }
  | { name: 'search' }
  | { name: 'settings' }
  | { name: 'track'; id: number }
  | { name: 'artist'; id: number }
  | { name: 'playlist'; id: number };

export type Section = 'likes' | 'wave' | 'history' | 'search' | 'settings';

const TOP: ReadonlySet<string> = new Set(['likes', 'wave', 'history', 'search', 'settings']);

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

export function openPlaylist(id: number): void {
  router.go({ name: 'playlist', id });
}
