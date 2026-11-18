/**
 * Web addresses in a note are links the moment they are there, as Notes' own data detectors
 * make them: typed, pasted, or written on another device without a link of its own. Drawn as
 * links (decorations), never written into the note — the note keeps exactly what Apple has.
 */
import { followClick } from "../../lib/follow";

// Found and drawn with the rest of what the editor draws over a note (decorations.ts).
export { addressesIn, autolink, type Found } from "./decorations";

/** A click on a link (its own, or one found) opens it, as a click does in Notes on the Mac —
 * through the window's one checked way out (lib/follow.ts). */
export { OPEN_NOTE_EVENT, linkedNote } from "../../lib/links";

export function openLinkAt(event: MouseEvent): boolean {
  return followClick(event);
}
