/**
 * The rules Notes' list and editor follow, kept apart from drawing them so they can be tested.
 */
import type { Note } from "../../lib/ipc";
import { dateText, timeText } from "../../lib/when.ts";
import { t } from "../../lib/i18n.ts";

const midnight = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate());
const daysBetween = (from: Date, to: Date) =>
  Math.round((midnight(to).getTime() - midnight(from).getTime()) / 864e5);

/** One run of the note list under a heading. */
export type Section = { key: string; heading: string; notes: Note[] };

/** Settings ▸ Sort Checked Items Automatically: off unless turned on, kept on this computer. */
export const MOVE_CHECKED_KEY = "notes.moveChecked";

/** How the list is ordered, as Notes' View menu offers it. */
export type NoteSort = "edited" | "created" | "title";
export type NoteOrder = { by: NoteSort; groupByDate: boolean };
export const DEFAULT_ORDER: NoteOrder = { by: "edited", groupByDate: true };

/** A note's title for sorting: its first line, as the list shows it. */
const titleOf = (n: Note) => (n.title || "").trim();
const byTitle = new Intl.Collator(undefined, { sensitivity: "base", numeric: true });

/**
 * The list the way Notes groups it: Pinned first, then — sorted by a date and grouped — Today,
 * Yesterday, Previous 7 Days, Previous 30 Days, each earlier month of this year by name, then
 * each earlier year, by that date. Within a group, newest first; by title, A to Z with no date
 * groups (as Notes does). A group with nothing in it is not shown. Notes with no date at all
 * come last, under Earlier, whatever the order.
 */
export function groupNotes(notes: Note[], now = new Date(), order: NoteOrder = DEFAULT_ORDER): Section[] {
  const dateOf = (n: Note) => (order.by === "created" ? n.createdAt : n.modifiedAt) ?? null;
  const sorted = [...notes].sort((a, b) =>
    order.by === "title"
      ? byTitle.compare(titleOf(a), titleOf(b)) || (b.modifiedAt ?? 0) - (a.modifiedAt ?? 0)
      : (dateOf(b) ?? -Infinity) - (dateOf(a) ?? -Infinity) || byTitle.compare(titleOf(a), titleOf(b)),
  );
  const out = new Map<string, Section>();
  const add = (key: string, heading: string, note: Note) => {
    const s = out.get(key) ?? { key, heading, notes: [] };
    s.notes.push(note);
    out.set(key, s);
  };
  const dated = order.by !== "title" && order.groupByDate;
  for (const note of sorted) {
    if (note.pinned) {
      add("pinned", t("notes.pinned"), note);
      continue;
    }
    if (!dated) {
      add("notes", t("sidebar.notes"), note);
      continue;
    }
    const at = dateOf(note);
    if (at === null) {
      add("undated", t("notes.earlier"), note);
      continue;
    }
    const when = new Date(at * 1000);
    const ago = daysBetween(when, now);
    if (ago <= 0) add("today", t("notes.today"), note);
    else if (ago === 1) add("yesterday", t("notes.yesterday"), note);
    else if (ago <= 7) add("week", t("notes.previous7Days"), note);
    else if (ago <= 30) add("month", t("notes.previous30Days"), note);
    else if (when.getFullYear() === now.getFullYear())
      add(`m${when.getMonth()}`, dateText(when, { month: "long" }), note);
    else add(`y${when.getFullYear()}`, String(when.getFullYear()), note);
  }
  // Map keeps insertion order, and notes went in sorted, so groups already run in order —
  // Pinned aside, which always leads, and Earlier (no date), which always ends.
  const sections = [...out.values()];
  const pinned = sections.filter((s) => s.key === "pinned");
  const undated = sections.filter((s) => s.key === "undated");
  const rest = sections.filter((s) => s.key !== "pinned" && s.key !== "undated");
  // With nothing pinned and no date groups, one untitled run: no "Notes" heading over it.
  if (!dated && pinned.length === 0) rest.forEach((s) => (s.heading = ""));
  return [...pinned, ...rest, ...undated];
}

/** The date a row shows: the time today, "Yesterday", the weekday this week, else the date. */
export function listDate(seconds: number | null, now = new Date()): string {
  if (seconds === null) return "";
  const when = new Date(seconds * 1000);
  const ago = daysBetween(when, now);
  if (ago <= 0) return timeText(when, { hour: "numeric", minute: "2-digit" });
  if (ago === 1) return t("notes.yesterday");
  if (ago < 7) return dateText(when, { weekday: "long" });
  return dateText(when, { day: "numeric", month: "numeric", year: "numeric" });
}

/** The date at the top of the editor: "30 September 2026 at 10:02 PM". */
export function editorDate(seconds: number | null): string {
  if (seconds === null) return "";
  const when = new Date(seconds * 1000);
  const day = dateText(when, { day: "numeric", month: "long", year: "numeric" });
  const time = timeText(when, { hour: "numeric", minute: "2-digit" });
  return t("notes.dayAtTime", { day, time });
}

/**
 * A note's text as the editor shows it: its first line, set as the title, and the rest.
 * Joining them back gives exactly the text that went in, so nothing is gained or lost by
 * editing the two halves.
 */
export function splitTitle(text: string): { title: string; rest: string } {
  const at = text.indexOf("\n");
  return at < 0 ? { title: text, rest: "" } : { title: text.slice(0, at), rest: text.slice(at + 1) };
}

export function joinTitle(title: string, rest: string, hadBreak: boolean): string {
  return rest.length > 0 || hadBreak ? `${title}\n${rest}` : title;
}

/**
 * Two edits of `base` combined when they change different stretches of it — the same rule
 * the save on the other side applies — or `null` when they overlap. Counted in code points
 * so a character outside the BMP is never cut in two.
 */
export function mergeEdits(base: string, mine: string, theirs: string): string | null {
  if (mine === theirs || base === theirs) return mine;
  if (base === mine) return theirs;
  const b = [...base];
  const m = script(b, [...mine]);
  const t = script(b, [...theirs]);
  let out = "";
  for (let gap = 0; gap <= b.length; gap++) {
    const ta = t.added[gap].join(""), ma = m.added[gap].join("");
    out += ta;
    if (ma !== ta) out += ma;
    if (gap < b.length && !m.removed[gap] && !t.removed[gap]) out += b[gap];
  }
  return out;
}

/**
 * One edit of `base`, character by character: what it removed, and what it put in each gap
 * (gap i is just before character i). The same rule the backend merges by, so the screen and
 * Apple agree on what a merge says.
 */
function script(b: string[], o: string[]): { removed: boolean[]; added: string[][] } {
  const removed = Array<boolean>(b.length).fill(false);
  const added: string[][] = Array.from({ length: b.length + 1 }, () => []);
  let pre = 0;
  while (pre < b.length && pre < o.length && b[pre] === o[pre]) pre++;
  let suf = 0;
  const max = Math.min(b.length, o.length) - pre;
  while (suf < max && b[b.length - 1 - suf] === o[o.length - 1 - suf]) suf++;
  const bm = b.slice(pre, b.length - suf), om = o.slice(pre, o.length - suf);
  if (bm.length * om.length <= 4_000_000) {
    const w = om.length + 1;
    const lcs = new Uint32Array((bm.length + 1) * w);
    for (let i = bm.length - 1; i >= 0; i--)
      for (let j = om.length - 1; j >= 0; j--)
        lcs[i * w + j] = bm[i] === om[j] ? lcs[(i + 1) * w + j + 1] + 1 : Math.max(lcs[(i + 1) * w + j], lcs[i * w + j + 1]);
    let i = 0, j = 0;
    while (i < bm.length || j < om.length) {
      if (i < bm.length && j < om.length && bm[i] === om[j]) { i++; j++; }
      else if (j < om.length && (i === bm.length || lcs[i * w + j + 1] >= lcs[(i + 1) * w + j])) { added[pre + i].push(om[j]); j++; }
      else { removed[pre + i] = true; i++; }
    }
  } else {
    for (let k = pre; k < pre + bm.length; k++) removed[k] = true;
    added[pre + bm.length] = om;
  }
  return { removed, added };
}
