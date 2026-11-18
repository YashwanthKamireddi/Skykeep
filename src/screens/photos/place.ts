/**
 * Where the photo grid is, kept through every change of its layout — as a collection view keeps
 * its place on a Mac and an iPhone: before the layout changes the view is anchored to something
 * (its top, its foot, a photo, a place it was sent to), and after, it is put back from that
 * anchor (UIKit's targetContentOffset(forProposedContentOffset:)).
 *
 * The grid's height changes all the time and for many reasons: the library's photos coming over
 * IPC after the window has drawn, months taking their real height as they are built, photos
 * landing from the phone, the window made wider or full screen (a wider tile is a taller one),
 * the zoom. Each was handled apart before — the foot held for half a second, the zoom's own
 * keeper, another for photos landing above, a jump held for thirty frames — and none for a
 * resize, so where the view ended up depended on which happened to be running: Photos opened at
 * 2019 instead of on the newest, and a full-screen window landed mid-library (18 Nov). Here
 * every change, whatever made it, is answered the same way.
 *
 * The anchor follows the person: each scroll of theirs sets it from where the view now is. A
 * change of the grid's height never moves the scroll by itself (the engine's own anchoring is
 * off, photos.css; WebKitGTK has none), and the scrolls made here are known, so a scroll event
 * that is not one of ours is theirs.
 */

export type Anchor =
  /** At the top: what grows below is what there is to see (newest first, opened). */
  | { kind: "top" }
  /** At the foot, on the newest (oldest first, opened): kept there whatever grows above. */
  | { kind: "foot" }
  /** A photo `top` pixels below the grid's top edge, kept there. */
  | { kind: "photo"; id: string; top: number }
  /** Sent somewhere (a month, a photo): `offset` is how far it still is from where it belongs. */
  | { kind: "target"; offset: () => number | null };

export class GridPlace {
  private anchor: Anchor = { kind: "top" };
  /** A glide of ours under way: its scroll is not the person's, and nothing is put back. */
  private gliding = false;
  private takeOver: (() => void) | null = null;
  private rest = 0;
  /** The scroll last set here: a scroll event at it is ours, any other the person's. */
  private ours = -1;
  private readonly grows: ResizeObserver;
  private readonly added: MutationObserver;

  /**
   * @param el The grid's scroller.
   * @param where How far below the scroller's top edge a photo's tile lies, built or not yet
   *   (its row in its month's block), or null when it is not in the grid.
   * @param top The photo at the top of the view and how far below the scroller's top edge it is,
   *   from the same layout as `where` (a month not built has no tiles to read it from).
   */
  constructor(
    private readonly el: HTMLElement,
    private readonly where: (id: string) => number | null,
    private readonly top: () => { id: string; top: number } | null,
  ) {
    this.grows = new ResizeObserver(() => this.keep());
    this.added = new MutationObserver(() => {
      this.watch();
      this.keep();
    });
    this.added.observe(el, { childList: true });
    el.addEventListener("scroll", this.scrolled, { passive: true });
    this.watch();
  }

  /** Anchored to `anchor`, and put there now. */
  set(anchor: Anchor) {
    this.anchor = anchor;
    this.keep();
  }

  /**
   * Sent to `offset`'s place with a glide (the month picker, with motion allowed). The glide's
   * own scroll is ours, not the person's: taken for theirs, each step became the anchor and the
   * months growing under it put the view back there, stopping it part way (photos-jump, 18 Nov).
   * Once it has come to rest it is put exactly there; a wheel, a touch, a press or a key while it
   * glides is the person taking over.
   */
  glide(offset: () => number | null) {
    const first = offset();
    if (first === null) return;
    this.anchor = { kind: "target", offset };
    this.gliding = true;
    const takeOver = () => this.landed();
    this.takeOver = takeOver;
    this.el.addEventListener("wheel", takeOver, { passive: true, once: true });
    this.el.addEventListener("pointerdown", takeOver, { passive: true, once: true });
    this.el.addEventListener("touchstart", takeOver, { passive: true, once: true });
    window.addEventListener("keydown", takeOver, { once: true });
    this.el.scrollTo({ top: this.el.scrollTop + first, behavior: "smooth" });
    this.settle();
  }

  /** What the view is anchored to now, to be set again later (All Photos, back from Months). */
  get(): Anchor {
    return this.anchor;
  }

  /** The view put back where its anchor says, if it has moved from there. */
  keep() {
    if (this.gliding) return;
    const el = this.el;
    // Out of sight (Years or Months in its place): nothing to measure, and its place stays.
    if (!el.clientHeight) return;
    const max = el.scrollHeight - el.clientHeight;
    const a = this.anchor;
    let to: number | null;
    if (a.kind === "top") to = 0;
    else if (a.kind === "foot") to = max;
    else {
      const off = a.kind === "photo" ? this.offsetOf(a.id, a.top) : a.offset();
      to = off === null ? null : el.scrollTop + off;
    }
    if (to === null) return;
    to = Math.max(0, Math.min(max, Math.round(to)));
    if (Math.abs(to - el.scrollTop) < 1) return;
    this.ours = to;
    el.scrollTop = to;
  }

  /** Let go: the grid's window is closing. */
  dispose() {
    this.landed();
    this.grows.disconnect();
    this.added.disconnect();
    this.el.removeEventListener("scroll", this.scrolled);
  }

  private offsetOf(id: string, top: number): number | null {
    const at = this.where(id);
    return at === null ? null : at - top;
  }

  /** The scroller and every month in it watched for a change of size. */
  private watch() {
    this.grows.observe(this.el);
    for (const child of this.el.children) this.grows.observe(child);
  }

  /** The glide has come to rest (or the person took over): put exactly where it was sent. */
  private landed() {
    if (!this.gliding) return;
    this.gliding = false;
    window.clearTimeout(this.rest);
    const t = this.takeOver;
    this.takeOver = null;
    if (t) {
      this.el.removeEventListener("wheel", t);
      this.el.removeEventListener("pointerdown", t);
      this.el.removeEventListener("touchstart", t);
      window.removeEventListener("keydown", t);
    }
    this.keep();
  }

  /** Rest is a moment with no scroll. */
  private settle() {
    window.clearTimeout(this.rest);
    this.rest = window.setTimeout(() => this.landed(), 150);
  }

  /** A scroll not made here is the person's: the anchor becomes where the view now is. */
  private scrolled = () => {
    if (this.gliding) return this.settle();
    const el = this.el;
    if (Math.abs(el.scrollTop - this.ours) < 1) return;
    this.ours = -1;
    const max = el.scrollHeight - el.clientHeight;
    if (el.scrollTop <= 1) this.anchor = { kind: "top" };
    else if (max - el.scrollTop <= 2) this.anchor = { kind: "foot" };
    else {
      const seen = this.top();
      if (seen) this.anchor = { kind: "photo", ...seen };
    }
  };
}
