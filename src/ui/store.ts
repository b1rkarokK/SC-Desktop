// Minimal shared state with change notifications.
import { api, errorMessage, type AuthStatus, type PlayerSnapshot, type Track, type User } from './api';
import { toast } from './dom';

type Topic = 'player' | 'likes' | 'dislikes' | 'auth' | 'follows';
type Listener = () => void;

class Store {
  snapshot: PlayerSnapshot | null = null;
  auth: AuthStatus = { has_credentials: false, username: null, user_id: null, avatar_url: null };
  liked = new Set<number>();
  disliked = new Set<number>();
  following = new Set<number>();
  private listeners = new Map<Topic, Set<Listener>>();

  on(topic: Topic, cb: Listener): void {
    let set = this.listeners.get(topic);
    if (!set) this.listeners.set(topic, (set = new Set()));
    set.add(cb);
  }

  emit(topic: Topic): void {
    this.listeners.get(topic)?.forEach((cb) => cb());
  }

  currentId(): number | null {
    return this.snapshot?.track?.id ?? null;
  }

  setSnapshot(s: PlayerSnapshot): void {
    this.snapshot = s;
    this.emit('player');
  }

  setAuth(a: AuthStatus): void {
    this.auth = a;
    this.emit('auth');
  }

  async reloadSets(): Promise<void> {
    try {
      const [liked, disliked] = await Promise.all([api.likesIds(), api.dislikedIds()]);
      this.liked = new Set(liked);
      this.disliked = new Set(disliked);
      this.emit('likes');
      this.emit('dislikes');
    } catch (e) {
      toast(errorMessage(e), 'error');
    }
  }

  setFollowing(users: User[]): void {
    this.following = new Set(users.map((u) => u.id));
    this.emit('follows');
  }

  /** Optimistic like/unlike; rolled back if SoundCloud refuses. */
  async setLiked(track: Track, liked: boolean): Promise<void> {
    const had = this.liked.has(track.id);
    if (liked) {
      this.liked.add(track.id);
      if (this.disliked.delete(track.id)) {
        this.emit('dislikes');
        void api.dislikeSet(track.id, false);
      }
    } else this.liked.delete(track.id);
    this.emit('likes');
    try {
      await api.likeSet(track, liked);
    } catch (e) {
      if (had) this.liked.add(track.id);
      else this.liked.delete(track.id);
      this.emit('likes');
      toast(errorMessage(e), 'error');
    }
  }

  /** "Не рекомендовать" — persisted, excluded from the wave forever. */
  async setDisliked(track: Track, disliked: boolean): Promise<void> {
    if (disliked) this.disliked.add(track.id);
    else this.disliked.delete(track.id);
    this.emit('dislikes');
    try {
      await api.dislikeSet(track.id, disliked);
      if (disliked) toast('Больше не будем рекомендовать этот трек');
    } catch (e) {
      if (disliked) this.disliked.delete(track.id);
      else this.disliked.add(track.id);
      this.emit('dislikes');
      toast(errorMessage(e), 'error');
    }
  }

  async setFollow(user: User, follow: boolean): Promise<void> {
    if (follow) this.following.add(user.id);
    else this.following.delete(user.id);
    this.emit('follows');
    try {
      await api.followSet(user, follow);
    } catch (e) {
      if (follow) this.following.delete(user.id);
      else this.following.add(user.id);
      this.emit('follows');
      toast(errorMessage(e), 'error');
    }
  }
}

export const store = new Store();
