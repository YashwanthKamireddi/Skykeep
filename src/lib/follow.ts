import { toast } from "../components/Toast";
import { OPEN_NOTE_EVENT, refusedLink, linkedNote, outsideHref } from "./links";

/**
 * The one way a link is followed, from anywhere in the window. The engine never follows one
 * itself — every click on a link is taken here first (main.tsx) — so a link's address is
 * always checked before it goes anywhere:
 *   - a link to another note opens that note, as it does in Notes;
 *   - a web page, an email, a call or a place goes to the app this computer uses for it (as a
 *     navigation, which the window refuses and hands on; main.rs, `links`);
 *   - anything else is not opened, and says so.
 */
export function follow(href: string): boolean {
  const note = linkedNote(href);
  if (note) {
    window.dispatchEvent(new CustomEvent<string>(OPEN_NOTE_EVENT, { detail: note }));
    return true;
  }
  const out = outsideHref(href);
  if (!out) {
    toast(refusedLink());
    return false;
  }
  window.location.assign(out);
  return true;
}

/**
 * A click on a link: the engine's own navigation is always prevented, and a plain click (not
 * one extending a selection) is followed. Taken once — the window's own listener sees every
 * click first, so a screen's handler that calls this again finds it already done.
 */
export function followClick(event: MouseEvent): boolean {
  const a = (event.target as Element | null)?.closest?.("a[href]") as HTMLAnchorElement | null;
  if (!a) return false;
  if (event.defaultPrevented) return true;
  event.preventDefault();
  if (event.button !== 0 || event.shiftKey || event.altKey) return true;
  follow(a.getAttribute("href") ?? "");
  return true;
}
