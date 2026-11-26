/**
 * A fade drawn by the page itself, one frame at a time.
 *
 * Every animation WebKitGTK handed to its compositor (CSS transitions and animations, the Web
 * Animations API) glitched on his screen under Hyprland: frames where the element was drawn as
 * if the animation had not started or had already ended. This one is never handed over — each
 * frame the page sets the element's opacity itself and the engine simply paints it — so there
 * is no hand-over to go wrong. Kept short, and only ever on opacity.
 */
const running = new WeakMap<HTMLElement, number>();

export function fadeIn(el: HTMLElement | null, { from = 0, duration = 150 }: { from?: number; duration?: number } = {}) {
  if (!el) return;
  if (typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
    el.style.opacity = "";
    return;
  }
  cancelAnimationFrame(running.get(el) ?? 0);
  const start = performance.now();
  el.style.opacity = String(from);
  const step = (now: number) => {
    // A frame's time can be from just before the fade began (it was started mid-frame): never
    // below where it starts.
    const t = Math.max(0, Math.min(1, (now - start) / duration));
    // Ease out: most of the change early, settling gently.
    const k = 1 - (1 - t) * (1 - t);
    if (t >= 1) {
      el.style.opacity = "";
      running.delete(el);
      return;
    }
    el.style.opacity = String(from + (1 - from) * k);
    running.set(el, requestAnimationFrame(step));
  };
  running.set(el, requestAnimationFrame(step));
}

/** The same fade the other way, then `done` — for something small leaving, as a toast. */
export function fadeOut(el: HTMLElement | null, done: () => void, duration = 120) {
  if (!el || (typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches)) {
    done();
    return;
  }
  cancelAnimationFrame(running.get(el) ?? 0);
  // Gone on time even where frames are not drawn (a window out of sight draws none).
  let finished = false;
  const finish = () => {
    if (finished) return;
    finished = true;
    running.delete(el);
    done();
  };
  window.setTimeout(finish, duration + 250);
  const start = performance.now();
  const step = (now: number) => {
    const t = Math.min(1, (now - start) / duration);
    // Ease in: gathering pace as it goes, so the end is not watched.
    el.style.opacity = String(1 - t * t);
    if (t >= 1) {
      finish();
      return;
    }
    running.set(el, requestAnimationFrame(step));
  };
  running.set(el, requestAnimationFrame(step));
}

/**
 * Every sheet, as it is added, fades in this way — set before the first frame is painted (a
 * mutation observer runs before painting), so none shows whole for a frame first. The
 * full-window dims behind them simply appear: a large surface is never animated.
 *
 * Every sheet is drawn at the top of the page (components/Portal), so only what is added to
 * the page's body is looked at. Watching the whole page looked through every row, tile and line
 * of text added anywhere — a photo grid scrolling, a note typed in — for a sheet. A sheet that
 * takes another's place on a dim already there fades itself in (`fadeIn`, ImportSheet).
 */
export function fadeSheetsIn() {
  const sheets = ".modal-sheet";
  // Popovers drawn at the top of the page grow out of what opened them (`grow`); menus and
  // the search's results are simply there, as a Mac's are.
  const popovers = ".cx-popover, .format-panel";
  watchPresses();
  new MutationObserver((changes) => {
    for (const change of changes) {
      for (const node of change.addedNodes) {
        if (!(node instanceof HTMLElement)) continue;
        if (node.matches(sheets)) fadeIn(node, { duration: 140 });
        node.querySelectorAll<HTMLElement>(sheets).forEach((el) => fadeIn(el, { duration: 140 }));
        if (node.matches(popovers)) grow(node);
      }
    }
  }).observe(document.body, { childList: true });
}

/* ---- a popover growing out of what opened it ----------------------------------------------
 *
 * As a macOS 26 popover does: from just smaller than itself, at the side nearest the control
 * or the spot that was clicked, to whole, fading in as it goes — 140ms, decelerating. Drawn by
 * the page frame by frame on opacity and `scale` only (the reasons are at the top of this
 * file), so nothing is laid out and nothing beside it moves; set before its first frame is
 * painted, so it never shows whole first. Only as it opens: moved to a new place while open,
 * it simply is there. Instant under Reduce Motion. */

/** Where the last press was, and when: what a popover opened by it grows out of. */
let lastPress: { x: number; y: number; at: number } | null = null;
let watching = false;
function watchPresses() {
  if (watching) return;
  watching = true;
  document.addEventListener("pointerdown", (e) => { lastPress = { x: e.clientX, y: e.clientY, at: performance.now() }; }, true);
}

/** What opened a popover appearing now: the press just made, or else the control with the
    keyboard's focus (a popover opened from the keyboard). */
function opener(): { x: number; y: number } | null {
  if (lastPress && performance.now() - lastPress.at < 800) return lastPress;
  const el = document.activeElement;
  if (!(el instanceof HTMLElement) || el === document.body) return null;
  const r = el.getBoundingClientRect();
  return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
}

export function grow(el: HTMLElement | null, from: { x: number; y: number } | null = opener()) {
  if (!el) return;
  const duration = token("--motion-pop", 140);
  if (reduced() || duration <= 0) return;
  const start0 = token("--pop-from", 0.96);
  // Out of the point of the popover nearest what opened it: a popover below its button grows
  // down from its top edge under the button; one beside an event, out of the side facing it.
  const r = el.getBoundingClientRect();
  const ox = from ? Math.min(Math.max(from.x, r.left), r.right) - r.left : r.width / 2;
  const oy = from ? Math.min(Math.max(from.y, r.top), r.bottom) - r.top : 0;
  cancelAnimationFrame(running.get(el) ?? 0);
  el.style.transformOrigin = `${ox.toFixed(1)}px ${oy.toFixed(1)}px`;
  el.style.opacity = "0";
  el.style.scale = String(start0);
  const finish = () => {
    el.style.opacity = "";
    el.style.scale = "";
    el.style.transformOrigin = "";
    running.delete(el);
  };
  const start = performance.now();
  const step = (now: number) => {
    const t = Math.max(0, Math.min(1, (now - start) / duration));
    if (t >= 1) return finish();
    // Decelerating (cubic ease-out): most of the way at once, settling gently.
    const k = 1 - (1 - t) ** 3;
    el.style.opacity = String(Math.min(1, k * 1.4).toFixed(3));
    el.style.scale = (start0 + (1 - start0) * k).toFixed(4);
    running.set(el, requestAnimationFrame(step));
  };
  running.set(el, requestAnimationFrame(step));
  // Whole on time even where no frames are drawn (a window out of sight draws none).
  window.setTimeout(() => { if (running.has(el)) { cancelAnimationFrame(running.get(el)!); finish(); } }, duration + 250);
}

/* ---- arrival: content that came in from another device ------------------------------------
 *
 * It settles in without being pointed at — no highlight, no wash, as the best apps do it —
 * drawn by the page frame by frame (as above), on opacity and a 2px transform only, so nothing
 * is laid out and no row beside it moves; only ever on a small thing, a row or a tile, never a
 * pane or a list. Under Reduce Motion each is instant. Timings are tokens (tokens.css "Arrival").
 *
 *   arrive(row)          a new row: fades in over 120ms, rising 2px
 *   leave(row, remove)   a row deleted elsewhere: out by opacity over 140ms, lifting 2px, then
 *                        `remove` takes it away (the rows below close up at once)
 *   <ReceivingLine>      the calm line while many things arrive at once (components/ReceivingLine)
 */

export const reduced = () =>
  typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/** A timing token in ms (or px, for a distance), as tokens.css sets it — 0 under Reduce
    Motion — or `fallback`. */
export function token(name: string, fallback: number): number {
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  const n = parseFloat(v);
  if (!Number.isFinite(n)) return fallback;
  return v.endsWith("ms") ? n : v.endsWith("s") ? n * 1000 : n;
}

