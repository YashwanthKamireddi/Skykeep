import { useLayoutEffect, useRef, type RefObject } from "react";
import { fadeIn } from "./fade";

/**
 * Rows a sync brought into the list on screen — a reminder added on the phone, a note, a card —
 * fade in, each alone and briefly (lib/fade: the page paints it, nothing is handed to the
 * compositor), and nothing else on screen moves for them.
 *
 * Only something that came from elsewhere: never the person's own. A row they made shows as
 * it is made (its stand-in, then the real one in its place), so nothing fades for a moment
 * after their last key or click, nor while anything of theirs is still on its way to Apple.
 * Never on opening a list or going to another one (its rows are all new, none arrived), never
 * for a batch — more than a few at once simply appear — and never with reduced motion.
 */
const ARRIVALS = 12;
/** How long after the person's own key or click nothing fades: their own rows land meanwhile. */
const OWN_MS = 4000;
const FADE_MS = 120;

let lastInput = -Infinity;
let watching = false;
function watchInput() {
  if (watching || typeof window === "undefined") return;
  watching = true;
  const seen = () => {
    lastInput = performance.now();
  };
  // Captured, so a control that stops its key or click still counts.
  window.addEventListener("pointerdown", seen, true);
  window.addEventListener("keydown", seen, true);
  window.addEventListener("drop", seen, true);
}

/**
 * Watch `ids` (the rows drawn, in `box`, each with `attr` its id) within one `scope` (the list
 * and how it is shown): rows new to a list already drawn fade in. `busy` is true while the
 * person's own changes are on their way (the outbox), and for a moment after.
 */
export function useArrivals(box: RefObject<HTMLElement | null>, ids: readonly string[], scope: string, busy = false, attr = "data-guid") {
  watchInput();
  const last = useRef<{ scope: string; ids: Set<string> } | null>(null);
  const busyUntil = useRef(-Infinity);
  if (busy) busyUntil.current = Infinity;
  else if (busyUntil.current === Infinity) busyUntil.current = performance.now() + OWN_MS;
  useLayoutEffect(() => {
    const was = last.current;
    last.current = { scope, ids: new Set(ids) };
    const now = performance.now();
    // The list's first rows, another list, or the person at work: nothing has arrived.
    if (!was || was.scope !== scope || was.ids.size === 0 || now - lastInput < OWN_MS || now < busyUntil.current) return;
    const added = ids.filter((id) => !was.ids.has(id));
    if (added.length === 0 || added.length > ARRIVALS) return;
    const el = box.current;
    if (!el) return;
    for (const id of added) {
      const esc = typeof CSS !== "undefined" && CSS.escape ? CSS.escape(id) : id;
      fadeIn(el.querySelector<HTMLElement>(`[${attr}="${esc}"]`), { duration: FADE_MS });
    }
  }, [ids, scope]); // eslint-disable-line react-hooks/exhaustive-deps
}
