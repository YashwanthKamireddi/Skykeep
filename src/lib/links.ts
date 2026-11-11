import { t } from "./i18n.ts";

/**
 * Which links Skykeep opens. Much of what the window shows was written by someone else — a
 * shared note, a shared list, an invitation, a public calendar — so a link's address is theirs
 * to choose, and the computer's opener runs whatever an address names: a `file:` link on a
 * share, a `search-ms:` query, a `.desktop` file. Only the kinds a person means by "a link" go
 * out: a web page, an email, a call, a place on a map. Anything else stays text.
 *
 * The same list is kept in Rust, where the window's navigation is refused (main.rs, `links`),
 * so a link that slips past this still goes nowhere.
 */

/** The kinds of address handed to the computer: everything else is refused. */
export const OUTSIDE_SCHEMES = ["http:", "https:", "mailto:", "tel:", "geo:"] as const;

/** What a refused link says, once, when clicked. */
/** What a link Skykeep will not open says, in the interface's language. */
export const refusedLink = () => t("common.refusedLink");

/** A link to another note asked to be followed: the Notes screen (NotesView) opens it. */
export const OPEN_NOTE_EVENT = "quince:open-note";

/** The note an Apple Notes link points at, as the library names it: `applenotes:note/<id>?…`
 * carries the note's record name, lower-cased (measured on his "Link feature of notes"). */
export function linkedNote(href: string): string | null {
  const m = /^applenotes:(?:\/\/)?note\/([0-9a-f-]{36})/i.exec(href);
  return m ? m[1].toUpperCase() : null;
}

/**
 * The address to hand to the computer for a link, or null when it is not one Skykeep opens.
 * Read as the engine reads it (WHATWG URL), so tabs, newlines, case and leading spaces cannot
 * dress one scheme up as another.
 */
export function outsideHref(href: string | null | undefined): string | null {
  if (!href) return null;
  let url: URL;
  try {
    url = new URL(href.trim());
  } catch {
    return null;
  }
  if (!(OUTSIDE_SCHEMES as readonly string[]).includes(url.protocol)) return null;
  // A web address names a host; `http:foo` or `https:///x` is not one.
  if ((url.protocol === "http:" || url.protocol === "https:") && !url.hostname) return null;
  return url.href;
}
