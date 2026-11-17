/**
 * A grid's photos without holding the library: the index whole (every photo's id, grouping date
 * and marks — a few bytes each), and the photos themselves fetched a page at a time as the grid
 * or the viewer comes near them, at most KEEP held at once, the farthest let go. A library of
 * fifty thousand opens on its first screen and stays the same size in memory however far it is
 * scrolled.
 */
import { api, type Photo, type PhotoIndex } from "../../lib/ipc";

/** Photos asked for at once. */
export const PAGE = 240;
/** The most photos held at once; past it, the longest unused of those nothing shows go. */
export const KEEP = 2400;

const KINDS = ["image", "video", "live"] as const;

/** Whether two reads of one photo are the same in every field. */
export function samePhoto(a: Photo, b: Photo): boolean {
  for (const k in b) {
    const x = (a as Record<string, unknown>)[k];
    const y = (b as Record<string, unknown>)[k];
    if (x === y) continue;
    if (Array.isArray(x) && Array.isArray(y) && x.length === y.length && x.every((v, i) => v === y[i])) continue;
    return false;
  }
  return true;
}

/**
 * The photos fetched, by id, for every list drawn from them. A photo read again unchanged keeps
 * the object it had, so nothing draws again for a read that changed nothing.
 */
export class PhotoStore {
  private photos = new Map<string, Photo>();
  /** Read before the latest refresh: shown, and fetched again when next wanted. */
  private stale = new Set<string>();
  private asked = new Set<string>();
  /** How many times each photo held has changed: what a month watches to draw a tile again. */
  private revs = new Map<string, number>();
  private listeners = new Set<() => void>();
  /**
   * Photos something on or near the screen shows (a month near the view, the viewer's strip),
   * by how many hold each: never let go while held. Let go by distance from where the grid
   * last asked, a month of more photos than KEEP (a large Screenshots library) let its own
   * on-screen photos go as its pages came in, and nothing asked for them again: grey boxes
   * that could not be clicked until the month was scrolled away and back (#84).
   */
  private holds = new Map<string, number>();
  /** When each photo was last asked for: of those nothing holds, the longest unused go first. */
  private used = new Map<string, number>();
  private tick = 0;
  private told = 0;
  /** Bumped by every change; what a view subscribes to. */
  version = 0;

  get(id: string): Photo | undefined {
    return this.photos.get(id);
  }
  turns(id: string): number {
    return this.revs.get(id) ?? 0;
  }
  private turned(id: string) {
    this.revs.set(id, (this.revs.get(id) ?? 0) + 1);
  }
  get size(): number {
    return this.photos.size;
  }
  /**
   * `ids` held while what shows them is near the screen: never let go meanwhile. Answers the
   * release, which is safe to call more than once.
   */
  hold(ids: readonly string[]): () => void {
    for (const id of ids) this.holds.set(id, (this.holds.get(id) ?? 0) + 1);
    let released = false;
    return () => {
      if (released) return;
      released = true;
      for (const id of ids) {
        const n = (this.holds.get(id) ?? 1) - 1;
        if (n > 0) this.holds.set(id, n);
        else this.holds.delete(id);
      }
    };
  }
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => void this.listeners.delete(listener);
  };
  private changed() {
    this.version++;
    // Once a frame however many pages land in it.
    if (this.told) return;
    this.told = requestAnimationFrame(() => {
      this.told = 0;
      this.listeners.forEach((l) => l());
    });
  }

  /** Photos read, kept by id; one unchanged keeps the object it had. */
  put(read: Photo[]) {
    let any = false;
    for (const p of read) {
      this.asked.delete(p.id);
      this.stale.delete(p.id);
      const had = this.photos.get(p.id);
      if (had && samePhoto(had, p)) continue;
      this.photos.set(p.id, p);
      if (had) this.turned(p.id);
      if (!this.used.has(p.id)) this.used.set(p.id, ++this.tick);
      any = true;
    }
    this.trim();
    if (any) this.changed();
  }

  /** A change made here, shown at once (Favorite, a title, a date). */
  patch(ids: Iterable<string>, change: (p: Photo) => Photo) {
    let any = false;
    for (const id of ids) {
      const p = this.photos.get(id);
      if (!p) continue;
      this.photos.set(id, change(p));
      this.turned(id);
      any = true;
    }
    if (any) this.changed();
  }

  /**
   * Everything held is from before this moment (a sync landed): still shown, and fetched again
   * as it is wanted; anything far off is let go now rather than fetched again.
   */
  refresh() {
    for (const id of this.photos.keys()) this.stale.add(id);
    this.asked.clear();
  }

  /** The held photos read before the latest refresh, fetched again: what the screen shows. */
  renew(): Promise<void> {
    const wanted = [...this.holds.keys()].filter((id) => this.stale.has(id) && !this.asked.has(id));
    return this.fetch(wanted);
  }

  /** The photos `from`–`to` of `list` on their way, a page at a time, unless here (and fresh). */
  need(list: PhotoList, from: number, to: number): Promise<void> {
    from = Math.max(0, from);
    to = Math.min(list.length - 1, to);
    if (to < from) return Promise.resolve();
    const wanted: string[] = [];
    for (let i = from; i <= to; i++) {
      const id = list.ids[i];
      this.used.set(id, ++this.tick);
      if ((!this.photos.has(id) || this.stale.has(id)) && !this.asked.has(id)) wanted.push(id);
    }
    return this.fetch(wanted);
  }

  /** `wanted` fetched a page at a time. */
  private fetch(wanted: string[]): Promise<void> {
    if (wanted.length === 0) return Promise.resolve();
    wanted.forEach((id) => this.asked.add(id));
    const pages: Promise<void>[] = [];
    for (let i = 0; i < wanted.length; i += PAGE) {
      const ids = wanted.slice(i, i + PAGE);
      pages.push(
        api.photosGet(ids).then(
          (read) => {
            this.put(read);
            // Gone from the library meanwhile: asked for no longer.
            ids.forEach((id) => this.asked.delete(id));
          },
          () => ids.forEach((id) => this.asked.delete(id)),
        ),
      );
    }
    return Promise.all(pages).then(() => {});
  }

  /**
   * These photos here before what shows them is drawn: the ones a sync brought into the view
   * being looked at. Drawn before they were here, their month showed its placeholder where its
   * photos had been, and every tile in it was made again when they came.
   */
  take(ids: string[]): Promise<void> {
    const wanted = ids.filter((id) => !this.photos.has(id));
    if (wanted.length === 0) return Promise.resolve();
    const pages: Promise<void>[] = [];
    for (let i = 0; i < wanted.length; i += PAGE) {
      pages.push(api.photosGet(wanted.slice(i, i + PAGE)).then((read) => this.put(read), () => {}));
    }
    return Promise.all(pages).then(() => {});
  }

  /** Photos by id, fetched where they are not here: for an action on a selection. */
  async load(ids: string[]): Promise<Photo[]> {
    const missing = ids.filter((id) => !this.photos.has(id));
    if (missing.length) {
      const read = await api.photosGet(missing);
      // Held only for this answer: never pushes out what the grid is showing.
      const got = new Map(read.map((p) => [p.id, p]));
      return ids.map((id) => this.photos.get(id) ?? got.get(id)).filter((p): p is Photo => Boolean(p));
    }
    return ids.map((id) => this.photos.get(id)!);
  }

  /**
   * Held to KEEP where it can be: of the photos nothing holds, the longest unused go first.
   * What is held stays however many that is — it is what the screen shows.
   */
  private trim() {
    if (this.photos.size <= KEEP) return;
    const loose = [...this.photos.keys()]
      .filter((id) => !this.holds.has(id))
      .sort((a, b) => (this.used.get(a) ?? 0) - (this.used.get(b) ?? 0));
    const goal = Math.floor(KEEP * 0.8);
    for (const id of loose) {
      if (this.photos.size <= goal) break;
      this.photos.delete(id);
      this.stale.delete(id);
      this.revs.delete(id);
      this.used.delete(id);
    }
  }
}

/** One read of a source: its index, and its photos as they are here. */
export class PhotoList {
  readonly ids: string[];
  readonly keys: number[];
  readonly marks: number[];
  readonly first: number | null;
  readonly last: number | null;
  private places: Map<string, number> | null = null;
  /** Only the first page of the view, painted before its index came (see `of`). */
  partial = false;

  constructor(index: PhotoIndex, readonly store: PhotoStore) {
    this.ids = index.ids;
    this.keys = index.keys;
    this.marks = index.marks;
    this.first = index.first;
    this.last = index.last;
  }

  /** A list from photos alone: the first screen, before the index comes. */
  static of(photos: Photo[], store: PhotoStore, keyOf: (p: Photo) => number): PhotoList {
    store.put(photos);
    const taken = photos.map((p) => p.takenAt);
    const list = new PhotoList(
      {
        ids: photos.map((p) => p.id),
        keys: photos.map(keyOf),
        marks: photos.map((p) => (p.kind === "video" ? 1 : p.kind === "live" ? 2 : 0) | (p.favorite ? 4 : 0) | ((p.rating ?? 0) << 3)),
        first: taken.length ? Math.min(...taken) : null,
        last: taken.length ? Math.max(...taken) : null,
      },
      store,
    );
    list.partial = true;
    return list;
  }

  get length(): number {
    return this.ids.length;
  }
  /** The photo at `i`, when it is here. */
  at(i: number): Photo | undefined {
    const id = this.ids[i];
    return id === undefined ? undefined : this.store.get(id);
  }
  indexOf(id: string): number {
    if (!this.places) this.places = new Map(this.ids.map((x, i) => [x, i]));
    return this.places.get(id) ?? -1;
  }
  kind(i: number): Photo["kind"] {
    return this.at(i)?.kind ?? KINDS[this.marks[i] & 3] ?? "image";
  }
  favorite(i: number): boolean {
    return this.at(i)?.favorite ?? (this.marks[i] & 4) !== 0;
  }
  stars(i: number): number {
    return this.at(i)?.rating ?? (this.marks[i] >> 3);
  }
  /** The photos `from`–`to` on their way. */
  need(from: number, to: number): Promise<void> {
    return this.store.need(this, from, to);
  }
  /** The same list without `gone` (taken out here, at once). */
  without(gone: Set<string>): PhotoList {
    const keep = this.ids.map((id) => !gone.has(id));
    const pick = <T,>(xs: T[]) => xs.filter((_, i) => keep[i]);
    return new PhotoList({ ids: pick(this.ids), keys: pick(this.keys), marks: pick(this.marks), first: this.first, last: this.last }, this.store);
  }
  /** The same as `other` throughout: a read that changed nothing. */
  same(other: PhotoList | null): boolean {
    if (!other || other.length !== this.length || other.first !== this.first || other.last !== this.last) return false;
    for (let i = 0; i < this.length; i++) {
      if (other.ids[i] !== this.ids[i] || other.keys[i] !== this.keys[i] || other.marks[i] !== this.marks[i]) return false;
    }
    return true;
  }
}
