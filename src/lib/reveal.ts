/**
 * Asking a screen to show one thing — a note, a reminder, an event, a contact, a file, a
 * photo — found somewhere else (the search across everything). The screen may not be built
 * yet (screens are built the first time they are opened), so a request waits until its
 * screen takes it: at once if it is listening, or when it first comes up.
 */
import { useEffect, useRef } from "react";

/** `within`: where it lives, when the screen needs it to get there — a reminder's list, a
 * file's folder. `startNew`: not a thing to show but a new one to start there, with no `id` —
 * Calendar's New Event from the taskbar's jump list. */
export type Reveal = { screen: string; id: string; within?: string | null; query?: string; at?: number; startNew?: boolean };

const EVENT = "quince:reveal";
const waiting = new Map<string, Reveal>();

/** Show `id` on `screen`; the screen is brought to the front by whoever asks. */
export function reveal(r: Reveal) {
  waiting.set(r.screen, r);
  window.dispatchEvent(new CustomEvent<Reveal>(EVENT, { detail: r }));
}

/** The request waiting for `screen`, taken: an app's own window hands it to that screen's
 *  window instead of showing it itself (Library's `choose`). */
export function takeWaiting(screen: string): Reveal | null {
  const r = waiting.get(screen) ?? null;
  waiting.delete(screen);
  return r;
}

/** A screen's part: `show` is called with each request meant for it, now or waiting. */
export function useReveal(screen: string, show: (r: Reveal) => void) {
  const latest = useRef(show);
  latest.current = show;
  useEffect(() => {
    const take = () => {
      const r = waiting.get(screen);
      if (!r) return;
      waiting.delete(screen);
      latest.current(r);
    };
    take();
    const on = (e: Event) => {
      if ((e as CustomEvent<Reveal>).detail.screen === screen) take();
    };
    window.addEventListener(EVENT, on);
    return () => window.removeEventListener(EVENT, on);
  }, [screen]);
}
