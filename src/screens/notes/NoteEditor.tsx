import "prosemirror-view/style/prosemirror.css";
import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from "react";
import { EditorState, Plugin, TextSelection, type Command, type Transaction } from "prosemirror-state";
import { Decoration, DecorationSet, EditorView } from "prosemirror-view";
import { history, undo, redo } from "prosemirror-history";
import { keymap } from "prosemirror-keymap";
import { baseKeymap, splitBlockAs, toggleMark, chainCommands } from "prosemirror-commands";
import { Fragment, Slice, type Node as PMNode } from "prosemirror-model";
import type { Run } from "./Formatted";
import { checklistMove, fromDoc, listNumbers, LIST_STYLES, schema, STYLE, tableCells, toDoc } from "./rich";
import { MOVE_CHECKED_KEY } from "./model";
import { foundRanges } from "./found";

function moveCheckedOn(): boolean {
  try {
    return localStorage.getItem(MOVE_CHECKED_KEY) === "1";
  } catch {
    return false;
  }
}
import { api, messageOf, noteMediaSrc, type NoteAttachment } from "../../lib/ipc";
import { RowMenu, type MenuEntry } from "../../components/RowMenu";
import { autolink, openLinkAt } from "./links";
import { linkAt, setLink } from "./linking";
import { LinkBox, type LinkAsk } from "./LinkBox";
import { composing } from "../../lib/keys";
import { t } from "../../lib/i18n";

/** `content` with every attachment taken out, a table's cells kept as text in its place. */
function withoutAttachments(content: Fragment): Fragment {
  const kept: PMNode[] = [];
  content.forEach((node) => {
    if (node.type === schema.nodes.attachment) {
      const text = node.textContent || (node.type.spec.leafText?.(node) ?? "");
      if (text) kept.push(schema.text(text.replace(/\s*\n\s*/g, " ")));
      return;
    }
    kept.push(node.isLeaf ? node : node.copy(withoutAttachments(node.content)));
  });
  return Fragment.fromArray(kept);
}

/** Whether a piece of the note holds an attachment — a picture, a file, a table. */
function sliceHasAttachment(slice: Slice): boolean {
  let found = false;
  slice.content.descendants((node) => {
    if (node.type === schema.nodes.attachment) found = true;
    return !found;
  });
  return found;
}

/** Every table on screen, by its element: how to read it again. */
const liveTables = new Map<HTMLElement, () => void>();

/** Every table on screen read again — after a sync, for a change made on another device. */
export function refreshTables() {
  liveTables.forEach((read) => read());
}

/** Whether tables can be edited here. Mirrors `TABLE_CELL_WRITES` / `TABLE_LINE_WRITES` in
 * src-tauri/src/main.rs. Cells are on: a cell written here showed once on his iPhone with no
 * conflict copy (2026-10-22). Rows and columns stay off until one is watched on a phone. */
const TABLE_CELL_WRITES = true;
const TABLE_LINE_WRITES = false;

/** A table's right-click menu: drawn by the editor that holds the table, opened by a cell. */
type CellMenu = { at: { x: number; y: number; alignRight: boolean }; entries: MenuEntry[] };
let openCellMenu: ((menu: CellMenu) => void) | null = null;

/**
 * An attachment in the note: a table drawn as the grid it is, with the cells Apple holds;
 * anything else — a picture, a sketch — as a labelled placeholder that keeps its place.
 *
 * A table's cells are edited where they stand. Each cell is a plain-text field of its own —
 * keys typed there are the cell's, never the note's — saved to Apple when it is left or
 * Return is pressed, drawn at once, and put back with the reason if Apple refuses. An empty
 * cell is editable too: a table made on a phone or on icloud.com already holds an empty text
 * object for every cell (measured on every table in his account).
 */
/** What each attachment is, asked once a session: a note re-drawn does not ask again. */
const described = new Map<string, Promise<NoteAttachment | null>>();
export function describe(id: string): Promise<NoteAttachment | null> {
  let known = described.get(id);
  if (!known) {
    // A lookup that failed (offline, the session being renewed) is not kept: the next time
    // the note is drawn asks again, rather than leaving the attachment a blank chip all session.
    known = api.noteAttachmentInfo(id).catch(() => {
      described.delete(id);
      return null;
    });
    described.set(id, known);
  }
  return known;
}

function size(bytes: number | null): string {
  if (!bytes) return "";
  if (bytes >= 1024 ** 2) return `${(bytes / 1024 ** 2).toLocaleString(undefined, { minimumFractionDigits: 1, maximumFractionDigits: 1 })} MB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

/**
 * A picture, document, recording or file where it sits in its note, as Notes shows it: a
 * picture at its own proportions, a PDF, scan or film as Apple's rendering of it with its
 * name, anything else as a labelled chip. Clicking one opens it in the app this computer uses
 * for it. A picture that cannot be had falls back to the chip, never a broken frame.
 */
/** A note's picture asked to be shown large: the viewer (NotePicture) listens for it. */
export const PICTURE_EVENT = "quince:note-picture";
export type PictureShown = { id: string; name: string; src: string; srcDark?: string; drawing?: boolean; whole?: string };
/** A note's picture drawn, so now kept on this computer: the list may show it beside the note
 *  (NotesView asks which notes have one). */
export const PICTURE_KEPT_EVENT = "quince:note-picture-kept";

export function drawMedia(dom: HTMLElement, info: NoteAttachment) {
  const name = info.filename ?? info.title ?? (info.kind === "audio" ? t("notes.recording") : t("notes.attachment"));
  // Opened in the app this computer uses for it; one that cannot be says why, on the chip
  // itself, rather than a click that seems to do nothing.
  let said: HTMLElement | null = null;
  // Answered on the click itself: the attachment dims and its line says it is opening, until
  // the app this computer uses for it has it — a file not yet on this computer is fetched
  // first, and a click that showed nothing meanwhile read as one that did nothing.
  let opening = false;
  const open = () => {
    if (opening) return;
    opening = true;
    dom.classList.add("is-opening");
    dom.setAttribute("aria-busy", "true");
    const before = said?.textContent ?? null;
    if (said) {
      said.textContent = t("notes.opening");
      said.classList.remove("is-trouble");
    }
    void api
      .noteAttachmentOpen(info.id)
      .then(
        () => {
          if (said) said.textContent = before;
        },
        (e) => {
          if (!said) return;
          said.textContent = t("notes.couldntOpen", { reason: messageOf(e) });
          said.classList.add("is-trouble");
        },
      )
      .finally(() => {
        opening = false;
        dom.classList.remove("is-opening");
        dom.removeAttribute("aria-busy");
      });
  };
  const chip = () => {
    const c = document.createElement("button");
    c.type = "button";
    c.className = `nf-file is-${info.kind}`;
    const glyph = document.createElement("span");
    glyph.className = "nf-file-glyph";
    // The kind of file, as Finder labels a document: its extension, short.
    const ext = name.includes(".") ? name.split(".").pop()!.slice(0, 4).toUpperCase() : "";
    glyph.textContent = info.kind === "audio" ? "♪" : info.kind === "video" ? "▶" : ext || t("notes.fileBadge");
    const label = document.createElement("span");
    label.className = "nf-file-name";
    label.textContent = name;
    const meta = document.createElement("span");
    meta.className = "nf-file-meta";
    meta.textContent = size(info.size);
    said = meta;
    c.append(glyph, label, meta);
    c.title = t("notes.openNamed", { name });
    c.addEventListener("click", open);
    return c;
  };
  const picture = (part: "file" | "inline" | "preview" | "drawing" | "drawing-dark", cls: string) => {
    const img = document.createElement("img");
    img.className = cls;
    img.alt = name;
    img.decoding = "async";
    img.draggable = false;
    // Its own size known before a byte of it arrives, so it holds its place at its proportions
    // (drawn no wider than the note) instead of a gap that jumps when it lands.
    if (info.width && info.height) {
      img.width = Math.round(info.width);
      img.height = Math.round(info.height);
      img.style.aspectRatio = `${info.width} / ${info.height}`;
    }
    img.loading = "lazy";
    img.src = noteMediaSrc(info.id, part);
    img.addEventListener("load", () => window.dispatchEvent(new Event(PICTURE_KEPT_EVENT)), { once: true });
    // The backend says whether the picture can be shown: a 404 (a HEIC with no decoder on
    // this computer, a fetch that failed) and it is the file's chip, drawn as one.
    img.addEventListener("error", () => {
      dom.className = "nf-attachment is-file";
      dom.replaceChildren(chip());
    });
    return img;
  };
  // A #tag or @mention sits in its line as the words it shows, as Notes sets it — not a file.
  if (info.kind === "inline") {
    const words = document.createElement("span");
    words.className = "nf-inline";
    words.textContent = info.title ?? "";
    dom.className = "nf-attachment is-inline";
    dom.title = "";
    dom.replaceChildren(words);
    return;
  }
  dom.title = t("notes.openNamed", { name });
  if (info.kind === "image") {
    // Drawn in the note no larger than the note needs (a phone's photo is 12 to 48 megapixels,
    // and a note of them decoded each whole while scrolling); the viewer shows it whole.
    const img = picture("inline", "nf-image");
    // Shown here at once, from the picture already drawn — never a wait on another app — and
    // whole once that is ready (NotePicture).
    img.addEventListener("click", () =>
      window.dispatchEvent(
        new CustomEvent<PictureShown>(PICTURE_EVENT, { detail: { id: info.id, name, src: img.src, whole: noteMediaSrc(info.id, "file") } }),
      ),
    );
    dom.className = "nf-attachment is-image";
    dom.replaceChildren(img);
  } else if (info.kind === "drawing") {
    // A drawing made in the note, as its ink alone (made on this computer from Apple's
    // rendering): one for a light page, one for a dark — the look in force shows its own.
    const light = picture("drawing", "nf-drawing nf-ink-light");
    const dark = picture("drawing-dark", "nf-drawing nf-ink-dark");
    const show = () =>
      window.dispatchEvent(
        new CustomEvent<PictureShown>(PICTURE_EVENT, {
          detail: { id: info.id, name: info.title ?? t("notes.drawing"), src: light.src, srcDark: dark.src, drawing: true },
        }),
      );
    light.addEventListener("click", show);
    dark.addEventListener("click", show);
    dom.className = "nf-attachment is-image is-drawing";
    dom.replaceChildren(light, dark);
  } else if (info.kind === "table") {
    // A table this app cannot read yet: said plainly, where it stands, never offered as a file
    // to open. It is read again after every sync, and drawn as its grid once it can be.
    const box = document.createElement("span");
    box.className = "nf-table-unread";
    box.textContent = t("notes.tableUnread");
    dom.className = "nf-attachment is-table-unread";
    dom.title = "";
    dom.replaceChildren(box);
  } else if ((info.kind === "document" || info.kind === "video") && info.hasPreview) {
    const figure = document.createElement("button");
    figure.type = "button";
    figure.className = `nf-doc is-${info.kind}`;
    const caption = document.createElement("span");
    caption.className = "nf-doc-name";
    caption.textContent = name;
    figure.append(picture("preview", "nf-doc-preview"), caption);
    figure.addEventListener("click", open);
    dom.className = "nf-attachment is-doc";
    dom.replaceChildren(figure);
  } else {
    dom.className = "nf-attachment is-file";
    dom.replaceChildren(chip());
  }
}

function attachmentView(node: PMNode) {
  const dom = document.createElement("span");
  dom.className = "nf-attachment";
  dom.contentEditable = "false";
  // Nothing until it is known what it is — then the picture, the table, the file. A word
  // standing in for it ("Attachment") read as clutter, and stayed where a lookup failed.
  dom.textContent = "";
  dom.classList.add("is-pending");
  dom.title = t("notes.changeOnApple");
  const id = node.attrs.id as string | null;
  // With no id (the note's formatting not read yet) there is nothing to look up: no frame.
  if (!id) dom.classList.replace("is-pending", "is-anon");

  const draw = (cells: string[][]) => {
    if (id) tableCells.set(id, cells.map((row) => [...row]));
    const table = document.createElement("table");
    table.className = "nf-table";
    cells.forEach((row, r) => {
      const tr = table.insertRow();
      row.forEach((cell, c) => {
        const td = tr.insertCell();
        const was = cell.trimEnd();
        if (!id || !TABLE_CELL_WRITES) {
          td.textContent = was;
          td.className = was ? "nf-cell" : "nf-cell-empty";
          return;
        }
        // Right-click, as in Notes: a row added above or below this one.
        td.addEventListener("contextmenu", (e) => {
          if (!TABLE_LINE_WRITES) return;
          e.preventDefault();
          const add = (at: number, column = false) => () => {
            dom.classList.add("is-saving");
            (column ? api.noteTableAddColumn(id, at) : api.noteTableAddRow(id, at)).then(
              (next) => { dom.classList.remove("is-saving"); draw(next); },
              (err) => { dom.classList.remove("is-saving"); dom.title = String(err); },
            );
          };
          const remove = (at: number, column: boolean) => () => {
            dom.classList.add("is-saving");
            api.noteTableRemove(id, at, column).then(
              (next) => { dom.classList.remove("is-saving"); draw(next); },
              (err) => { dom.classList.remove("is-saving"); dom.title = String(err); },
            );
          };
          openCellMenu?.({
            at: { x: e.clientX, y: e.clientY, alignRight: false },
            entries: [
              { label: t("notes.addRowAbove"), run: add(r) },
              { label: t("notes.addRowBelow"), run: add(r + 1) },
              null,
              { label: t("notes.addColumnBefore"), run: add(c, true) },
              { label: t("notes.addColumnAfter"), run: add(c + 1, true) },
              null,
              // A table keeps at least one row and one column, as Notes does.
              ...(cells.length > 1 ? [{ label: t("notes.deleteRow"), danger: true, run: remove(r, false) }] : []),
              ...((cells[0]?.length ?? 0) > 1 ? [{ label: t("notes.deleteColumn"), danger: true, run: remove(c, true) }] : []),
            ],
          });
        });
        const field = document.createElement("div");
        field.className = "nf-cell-edit";
        field.contentEditable = "plaintext-only";
        field.spellcheck = true;
        field.textContent = was;
        let saved = was;
        const save = () => {
          const now = (field.textContent ?? "").replace(/\n+/g, " ");
          if (now === saved) return;
          const before = saved;
          saved = now;
          field.classList.add("is-saving");
          const copy = tableCells.get(id);
          if (copy?.[r]) copy[r][c] = now;
          api.noteTableSetCell(id, r, c, now).then(
            () => field.classList.remove("is-saving"),
            (e) => {
              // Refused: back as Apple has it, and why.
              saved = before;
              field.textContent = before;
              if (copy?.[r]) copy[r][c] = before;
              field.classList.remove("is-saving");
              field.title = String(e);
              field.classList.add("is-refused");
              window.setTimeout(() => { field.classList.remove("is-refused"); field.title = ""; }, 4000);
            },
          );
        };
        field.addEventListener("keydown", (e) => {
          if (composing(e)) return;
          if (e.key === "Enter") {
            e.preventDefault();
            field.blur();
          } else if (e.key === "Escape") {
            field.textContent = saved;
            field.blur();
          }
        });
        field.addEventListener("blur", save);
        td.appendChild(field);
      });
    });
    dom.className = "nf-attachment is-table";
    dom.title = "";
    dom.replaceChildren(table);
  };

  /**
   * What the attachment is, drawn once known. A lookup that fails (offline, the session being
   * renewed, Apple slow) is asked again a little later, and again after every sync — never left
   * a blank gap. Until then a quiet frame holds its place; after the last try it is a card
   * that still opens it.
   */
  let tries = 0;
  let retry: number | null = null;
  let gone = false;
  const showMedia = (mediaId: string) => {
    if (retry !== null) window.clearTimeout(retry);
    retry = null;
    void describe(mediaId).then((info) => {
      if (gone) return;
      if (info) {
        dom.classList.remove("is-pending", "is-unknown");
        drawMedia(dom, info);
        return;
      }
      tries += 1;
      if (tries < 4) retry = window.setTimeout(() => showMedia(mediaId), 1500 * 2 ** tries);
      else if (dom.classList.contains("is-pending")) {
        drawMedia(dom, { id: mediaId, uti: "", kind: "file", filename: null, title: null, width: null, height: null, size: null, hasPreview: false });
        dom.classList.add("is-unknown");
      }
    });
  };

  if (id) {
    // What it is, first — kept on disk once known — so a picture or a file is drawn at once.
    // Asking for table cells first went to Apple for every picture on every open (a picture
    // has no cells to keep, so the answer was never kept), and each one waited for it.
    void describe(id).then((info) => {
      if (info && info.kind !== "table") return showMedia(id);
      api.noteTable(id).then(
        (cells) => {
          if (cells && cells.length) draw(cells);
          else showMedia(id);
        },
        () => showMedia(id),
      );
    });
    // Read again after every sync, so a table typed in on a phone shows its new cells here
    // without leaving the note; drawn again only when they changed.
    liveTables.set(dom, () => {
      // An attachment not yet known is asked about again now that Apple has been reached.
      if (dom.classList.contains("is-pending") || dom.classList.contains("is-unknown")) {
        tries = Math.min(tries, 3);
        return showMedia(id);
      }
      if (!dom.classList.contains("is-table") && !dom.classList.contains("is-table-unread")) return;
      // Never under a cell being typed in: drawn again, the table threw the typing away and the
      // cursor with it. The next sync after the cell is left brings the other cells' changes.
      if (dom.contains(document.activeElement)) return;
      api.noteTable(id).then(
        (cells) => {
          if (cells && cells.length && !dom.contains(document.activeElement) && JSON.stringify(cells) !== JSON.stringify(tableCells.get(id))) draw(cells);
        },
        () => {},
      );
    });
  }
  return {
    dom,
    destroy: () => {
      gone = true;
      liveTables.delete(dom);
      if (retry !== null) window.clearTimeout(retry);
    },
    ignoreMutation: () => true,
    // A cell's keys, clicks and typing are the cell's, not the note editor's.
    stopEvent: (e: Event) => Boolean((e.target as HTMLElement | null)?.closest?.(".nf-cell-edit, .nf-image, .nf-doc, .nf-file")),
  };
}

/** How often, at most, a note being typed in is read back for saving (see `report`). The save
 *  itself waits for a pause in typing far longer than this. */
const REPORT_MS = 150;

/** What the toolbar shows as on: the style of the line the cursor is in, and its marks. */
export type EditorStatus = {
  style: number | null;
  bold: boolean;
  italic: boolean;
  underline: boolean;
  strike: boolean;
  checklist: boolean;
  quote: boolean;
  /** The text colour where the cursor is (1 purple … 5 blue), as Notes' colour menu ticks it. */
  colour: number | null;
  /** The cursor or selection is in a link. */
  link: boolean;
};

export type EditorHandle = {
  /** Text colour on the selection (or on what is typed next), or none. */
  setColour: (colour: number | null) => void;
  /** Indent or outdent the lines the selection touches. */
  indent: (by: 1 | -1) => void;
  /** Move the lines the selection touches up or down past the next line. */
  moveLines: (by: 1 | -1) => void;
  /** Block Quote on the lines the selection touches, or off. */
  toggleQuote: () => void;
  /** Set every line the selection touches to one of Notes' styles. */
  setStyle: (style: number | null) => void;
  /** Lines to checklist items, or — when they all are — back to plain lines. */
  toggleChecklist: () => void;
  toggleMark: (mark: "bold" | "italic" | "underline" | "strike") => void;
  /** Take in text Apple now holds, keeping the cursor where it was as near as it can. */
  replace: (text: string, runs: Run[] | null) => void;
  focus: () => void;
  /** Hand over any change not yet reported, now. */
  flushChange: () => void;
  /** Add Link (Command-K): the box for the selection, or for the link the cursor is in. */
  openLink: () => void;
  /** What the editor holds this moment: its text and how it is set. */
  current: () => { text: string; runs: Run[] } | null;
  /** Where the cursor is, in UTF-16 units of `current().text`; null with no editor. */
  caret: () => number | null;
};

type Props = {
  text: string;
  runs: Run[] | null;
  editable: boolean;
  label: string;
  placeholder?: string;
  autoFocus?: boolean;
  onChange: (text: string, runs: Run[]) => void;
  onStatus?: (status: EditorStatus) => void;
  onBlur?: () => void;
  /** The words of the search the note was opened from, marked where they fall (found.ts). */
  found?: string[];
};

/** Every paragraph the selection touches, with where it starts. */
function linesIn(state: EditorState): { node: PMNode; pos: number }[] {
  const out: { node: PMNode; pos: number }[] = [];
  const { from, to } = state.selection;
  state.doc.nodesBetween(from, to, (node, pos) => {
    if (node.type === schema.nodes.paragraph) out.push({ node, pos });
    return node.type === schema.nodes.doc;
  });
  return out;
}

function restyle(style: number | null, tick?: (was: PMNode) => boolean | null): Command {
  return (state, dispatch) => {
    const tr = state.tr;
    for (const { node, pos } of linesIn(state)) {
      const checked = style === STYLE.checklist ? (tick ? tick(node) : (node.attrs.style === STYLE.checklist ? node.attrs.checked : false)) : null;
      tr.setNodeMarkup(pos, undefined, {
        ...node.attrs,
        style,
        checked,
        // Out of a list, a line is not indented as one.
        indent: style !== null && LIST_STYLES.includes(style) ? node.attrs.indent : 0,
      });
    }
    dispatch?.(tr.scrollIntoView());
    return true;
  };
}

/** Block Quote on every line the selection touches, or — when they all are — off, as
 * Format › Block Quote (Command-') does in Notes. */
const toggleQuote: Command = (state, dispatch) => {
  const lines = linesIn(state);
  const on = !lines.every(({ node }) => node.attrs.quote);
  const tr = state.tr;
  for (const { node, pos } of lines) tr.setNodeMarkup(pos, undefined, { ...node.attrs, quote: on });
  dispatch?.(tr.scrollIntoView());
  return true;
};

/** Indent or outdent the lines in a list, as Tab and Command-] do. */
function shift(by: 1 | -1, onlyLists: boolean): Command {
  return (state, dispatch) => {
    const lines = linesIn(state);
    if (onlyLists && !lines.every(({ node }) => LIST_STYLES.includes(node.attrs.style))) return false;
    const tr = state.tr;
    for (const { node, pos } of lines) {
      tr.setNodeMarkup(pos, undefined, { ...node.attrs, indent: Math.max(0, Math.min(8, node.attrs.indent + by)) });
    }
    dispatch?.(tr);
    return true;
  };
}

/** Notes' text colour on the selection, or on what is typed next when nothing is selected. */
function colour(c: number | null): Command {
  return (state, dispatch) => {
    const type = schema.marks.emphasis;
    const { from, to, empty } = state.selection;
    const tr = state.tr;
    if (empty) {
      tr.removeStoredMark(type);
      if (c) tr.addStoredMark(type.create({ color: c }));
    } else {
      tr.removeMark(from, to, type);
      if (c) tr.addMark(from, to, type.create({ color: c }));
    }
    dispatch?.(tr);
    return true;
  };
}

/**
 * The lines the selection touches moved up or down past one line, as Notes' Move Line Up
 * and Down do — the cursor and selection go with them, and a table moves with its line.
 */
function moveLines(by: 1 | -1): Command {
  return (state, dispatch) => {
    const lines = linesIn(state);
    if (!lines.length) return false;
    const start = lines[0].pos;
    const end = lines[lines.length - 1].pos + lines[lines.length - 1].node.nodeSize;
    const first = state.doc.resolve(start).index(0);
    const last = first + lines.length - 1;
    if (by < 0 ? first === 0 : last >= state.doc.childCount - 1) return false;
    const past = state.doc.child(by < 0 ? first - 1 : last + 1).nodeSize;
    const block = state.doc.slice(start, end).content;
    const tr = state.tr.delete(start, end);
    const shiftBy = by < 0 ? -past : past;
    tr.insert(start + (by < 0 ? -past : past), block);
    const { from, to } = state.selection;
    tr.setSelection(TextSelection.create(tr.doc, from + shiftBy, to + shiftBy));
    dispatch?.(tr.scrollIntoView());
    return true;
  };
}

/** Each numbered line's number (`data-n`, which the stylesheet draws), counted as Notes counts
 *  them (rich.ts listNumbers) — worked out again only when the note changes. */
function numbering(doc: PMNode): DecorationSet {
  const lines: PMNode[] = [];
  const at: number[] = [];
  doc.forEach((node, offset) => {
    lines.push(node);
    at.push(offset);
  });
  const numbers = listNumbers(lines.map((n) => n.attrs as { style: number | null; indent: number; start: number | null }));
  const marks: Decoration[] = [];
  numbers.forEach((n, i) => {
    if (n !== null) marks.push(Decoration.node(at[i], at[i] + lines[i].nodeSize, { "data-n": String(n) }));
  });
  return DecorationSet.create(doc, marks);
}
const numbered: Plugin<DecorationSet> = new Plugin<DecorationSet>({
  state: {
    init: (_, state) => numbering(state.doc),
    apply: (tr, was) => (tr.docChanged ? numbering(tr.doc) : was),
  },
  props: { decorations: (state): DecorationSet | undefined => numbered.getState(state) },
});

/**
 * The words of a search marked in the note, as Notes marks them in a note opened from its
 * results. Only drawn — never part of the note — and worked out again only when the words or
 * the text change; with no search, nothing is walked at all.
 */
type FoundState = { words: string[]; marks: DecorationSet };
function foundMarks(doc: PMNode, words: string[]): DecorationSet {
  if (!words.length) return DecorationSet.empty;
  const marks: Decoration[] = [];
  doc.descendants((node, pos) => {
    if (!node.isText || !node.text) return true;
    for (const [a, b] of foundRanges(node.text, words)) marks.push(Decoration.inline(pos + a, pos + b, { class: "note-found" }));
    return false;
  });
  return DecorationSet.create(doc, marks);
}
const found: Plugin<FoundState> = new Plugin<FoundState>({
  state: {
    init: () => ({ words: [], marks: DecorationSet.empty }),
    apply: (tr, was) => {
      const words = tr.getMeta(found) as string[] | undefined;
      if (words) return { words, marks: foundMarks(tr.doc, words) };
      if (!tr.docChanged || !was.words.length) return was;
      return { words: was.words, marks: foundMarks(tr.doc, was.words) };
    },
  },
  props: { decorations: (state): DecorationSet | undefined => found.getState(state)?.marks },
});

const parentOf = (state: EditorState) => state.selection.$from.parent;
const inList = (state: EditorState) => LIST_STYLES.includes(parentOf(state).attrs.style);

/**
 * Return as Notes has it: in a list the next line is another item — a new checklist item
 * unticked — and Return on an empty item ends the list; after a title or heading the next
 * line is body text.
 */
const enter: Command = (state, dispatch) => {
  const p = parentOf(state);
  if (inList(state) && p.content.size === 0 && state.selection.empty) {
    return restyle(null)(state, dispatch);
  }
  // Return on an empty quoted line ends the quote, as it ends a list.
  if (p.attrs.quote && p.content.size === 0 && state.selection.empty) {
    return toggleQuote(state, dispatch);
  }
  return splitBlockAs((node, atEnd) => {
    const style = node.attrs.style as number | null;
    if (LIST_STYLES.includes(style as number)) {
      // The next item counts on from this one; only a list's first item holds where it starts.
      return { type: schema.nodes.paragraph, attrs: { ...node.attrs, checked: style === STYLE.checklist ? false : null, start: null } };
    }
    if (atEnd && (style === STYLE.title || style === STYLE.heading || style === STYLE.subheading)) {
      return { type: schema.nodes.paragraph, attrs: { style: null, indent: 0, checked: null } };
    }
    return { type: schema.nodes.paragraph, attrs: { ...node.attrs } };
  })(state, dispatch);
};

/** Backspace at the very start of a list item makes it a plain line, before joining lines. */
const backspace: Command = (state, dispatch) => {
  const { $from, empty } = state.selection;
  if (!empty || $from.parentOffset !== 0 || !inList(state)) return false;
  return restyle(null)(state, dispatch);
};

const lineBreak: Command = (state, dispatch) => {
  dispatch?.(state.tr.replaceSelectionWith(schema.nodes.line_break.create()).scrollIntoView());
  return true;
};

const tab: Command = (state, dispatch) => {
  if (shift(1, true)(state, dispatch)) return true;
  dispatch?.(state.tr.insertText("\t"));
  return true;
};

/**
 * Tab indents and types a tab here, so the keys alone could never leave a note: Escape, then
 * Tab (or Shift-Tab), moves on to the next control instead, as a code editor lets the keys out.
 * Any other key after Escape and Tab indents again.
 */
/**
 * A shortcut acts on the selection the person sees. The editor learns of a selection made
 * with the keys (Shift-Home, Shift-arrows) from the browser's `selectionchange`, which comes a
 * moment after the key; a shortcut pressed straight after — Command-K, Command-B — read the
 * selection from before, and a link meant for the words selected went in beside them. So
 * before a Ctrl or Command shortcut on a character, the browser's selection is taken in first.
 * Never before a key that moves the caret itself (Ctrl-Home, Ctrl-arrows): the editor takes the
 * selection change after one it made as its own, and the browser's move was then lost.
 */
const selectionFirst = new Plugin({
  props: {
    handleKeyDown(view, event) {
      if (!(event.ctrlKey || event.metaKey) || event.isComposing || event.key.length !== 1) return false;
      const sel = (view.root as Document).getSelection?.();
      if (!sel?.anchorNode || !sel.focusNode) return false;
      if (!view.dom.contains(sel.anchorNode) || !view.dom.contains(sel.focusNode)) return false;
      try {
        const anchor = view.posAtDOM(sel.anchorNode, sel.anchorOffset);
        const head = view.posAtDOM(sel.focusNode, sel.focusOffset);
        const now = view.state.selection;
        if (now instanceof TextSelection && (now.anchor !== anchor || now.head !== head)) {
          view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, anchor, head)));
        }
      } catch {
        // A selection the editor cannot place (inside an attachment's own controls): left be.
      }
      return false;
    },
  },
});

const released = new WeakSet<EditorView>();
const letTabOut = new Plugin({
  props: {
    handleKeyDown(view, e) {
      if (e.key === "Escape") released.add(view);
      else if (e.key !== "Tab" && e.key !== "Shift") released.delete(view);
      return false;
    },
  },
});
const unlessReleased = (cmd: Command): Command => (state, dispatch, view) => {
  if (view && released.has(view)) {
    released.delete(view);
    return false;
  }
  return cmd(state, dispatch, view);
};

function statusOf(state: EditorState): EditorStatus {
  const p = parentOf(state);
  const marks = state.storedMarks ?? state.selection.$from.marks();
  const { from, to, empty } = state.selection;
  const has = (name: string) => {
    const type = schema.marks[name];
    return empty ? marks.some((m) => m.type === type) : state.doc.rangeHasMark(from, to, type);
  };
  return {
    style: p.attrs.style,
    bold: has("bold"),
    italic: has("italic"),
    underline: has("underline"),
    strike: has("strike"),
    checklist: p.attrs.style === STYLE.checklist,
    quote: Boolean(p.attrs.quote),
    colour: ((empty ? marks : state.doc.resolve(from).marksAcross(state.doc.resolve(to)) ?? marks).find((m) => m.type === schema.marks.emphasis)
      ?.attrs.color as number | undefined) ?? null,
    link: linkAt(state) !== null,
  };
}

/**
 * A note, edited formatted — as Notes edits it on icloud.com, a Mac or a phone. Typing, the
 * Aa styles, the checklist button and the keys all change the same document, which is read
 * back as Apple's text and runs on every change.
 */
export const NoteEditor = forwardRef<EditorHandle, Props>(function NoteEditor(
  { text, runs, editable, label, placeholder, autoFocus, onChange, onStatus, onBlur, found: words },
  ref,
) {
  const host = useRef<HTMLDivElement | null>(null);
  const view = useRef<EditorView | null>(null);
  const callbacks = useRef({ onChange, onStatus, onBlur });
  callbacks.current = { onChange, onStatus, onBlur };
  /** The toolbar's state as last told: told again only when something in it changed. Every
   *  move of the cursor told it anew, and the whole screen was drawn again for each. */
  const told = useRef<EditorStatus | null>(null);
  const tell = (state: EditorState) => {
    const next = statusOf(state);
    const was = told.current;
    if (was && (Object.keys(next) as (keyof EditorStatus)[]).every((k) => was[k] === next[k])) return;
    told.current = next;
    callbacks.current.onStatus?.(next);
  };

  /**
   * The note read back as Apple's text and runs — a walk of the whole note — done just after
   * the key is drawn rather than before, so a long note types as fast as a short one: at most
   * once in REPORT_MS while keys keep coming, rather than once a key. Done at once whenever
   * the note is left, so nothing typed can be missed.
   */
  const later = useRef<number | null>(null);
  /** The Add Link box, while it is open; `linking` is read by the editor's blur. */
  const [linkAsk, setLinkAsk] = useState<LinkAsk | null>(null);
  const linking = useRef(false);
  const openLinkRef = useRef(() => {});
  openLinkRef.current = () => {
    const v = view.current;
    if (!v || !v.editable) return;
    const { state } = v;
    const around = linkAt(state);
    const start = v.coordsAtPos(around && state.selection.empty ? around.from : state.selection.from);
    const foot = v.coordsAtPos(state.selection.to);
    // The note's own area, as it shows on screen now: the pane the editor scrolls in.
    const pane = (host.current?.closest(".note-editor") ?? host.current ?? v.dom).getBoundingClientRect();
    linking.current = true;
    setLinkAsk({
      at: { x: start.left, top: Math.min(start.top, foot.top), bottom: foot.bottom },
      within: { left: pane.left, right: pane.right, top: pane.top, bottom: pane.bottom },
      href: around?.href ?? null,
      needsText: state.selection.empty && !around,
    });
  };
  /** The box closed: back into the note, or — clicked away elsewhere — the note left then. */
  const closeLink = (back: boolean) => {
    linking.current = false;
    setLinkAsk(null);
    const v = view.current;
    if (!v) return;
    if (back) {
      v.focus();
      return;
    }
    window.setTimeout(() => {
      if (!v.hasFocus()) {
        if (later.current !== null) report();
        callbacks.current.onBlur?.();
      }
    }, 0);
  };
  const report = () => {
    if (later.current !== null) {
      window.clearTimeout(later.current);
      later.current = null;
    }
    const v = view.current;
    if (!v) return;
    const out = fromDoc(v.state.doc);
    callbacks.current.onChange(out.text, out.runs);
  };
  const reportSoon = () => {
    if (later.current !== null) return;
    later.current = window.setTimeout(() => {
      later.current = null;
      report();
    }, REPORT_MS);
  };

  useEffect(() => {
    if (!host.current) return undefined;
    const state = EditorState.create({
      doc: toDoc(text, runs),
      plugins: [
        selectionFirst,
        letTabOut,
        autolink,
        numbered,
        found,
        history(),
        keymap({
          "Mod-z": undo,
          "Mod-Shift-z": redo,
          "Mod-y": redo,
          "Mod-k": () => {
            openLinkRef.current();
            return true;
          },
          "Mod-b": toggleMark(schema.marks.bold),
          "Mod-i": toggleMark(schema.marks.italic),
          "Mod-u": toggleMark(schema.marks.underline),
          "Mod-'": toggleQuote,
          // Move Line Up and Down, as Notes on a Mac has them (Control-Command-Up and Down).
          "Mod-Alt-ArrowUp": moveLines(-1),
          "Mod-Alt-ArrowDown": moveLines(1),
          "Mod-]": shift(1, false),
          "Mod-[": shift(-1, false),
          Tab: unlessReleased(tab),
          "Shift-Tab": unlessReleased(shift(-1, false)),
          Enter: enter,
          "Shift-Enter": lineBreak,
          Backspace: backspace,
          // The Mac's Format ▸ keys for Notes' styles.
          "Mod-Shift-t": restyle(STYLE.title),
          "Mod-Shift-h": restyle(STYLE.heading),
          "Mod-Shift-j": restyle(STYLE.subheading),
          "Mod-Shift-b": restyle(null),
          "Mod-Shift-m": restyle(STYLE.mono),
          "Mod-Shift-7": restyle(STYLE.bulleted),
          "Mod-Shift-8": restyle(STYLE.dashed),
          "Mod-Shift-9": restyle(STYLE.numbered),
          "Mod-Shift-l": (s, d) => toggleChecklistCmd(s, d),
          "Mod-Shift-u": tickSelected,
        }),
        keymap(baseKeymap),
      ],
    });
    const v = new EditorView(host.current, {
      state,
      nodeViews: { attachment: (node) => attachmentView(node) },
      editable: () => editable,
      attributes: {
        class: "note-rich",
        role: "textbox",
        "aria-multiline": "true",
        "aria-label": label,
        "aria-description": t("notes.editorHelp"),
        spellcheck: "true",
        ...(placeholder ? { "data-placeholder": placeholder } : {}),
      },
      dispatchTransaction(tr: Transaction) {
        const before = v.state;
        const next = before.apply(tr);
        v.updateState(next);
        // Only a change to what the note holds is an edit. A step that leaves the document as
        // it was — text dragged and dropped where it lay, a mark set where it already is —
        // is not, and must never become a save.
        if (tr.docChanged && !next.doc.eq(before.doc)) reportSoon();
        tell(next);
      },
      // A paste never brings an attachment's reference: Apple keeps each table or picture in
      // one place in one note, and a pasted copy referred to the same one twice, which made
      // Notes on a phone lag and crash. The copy's text (a table's cells) comes instead.
      // A pasted table, as Notes pastes one into text: a line per row, its cells by tabs.
      transformPastedHTML(html) {
        return html
          .replace(/<\/t[dh]>\s*<t[dh][^>]*>/gi, "\t")
          .replace(/<tr[^>]*>/gi, "<p>")
          .replace(/<\/tr>/gi, "</p>")
          .replace(/<\/?(table|tbody|thead|tfoot|td|th)[^>]*>/gi, "");
      },
      // Lines copied whole keep their style, tick, indent and numbering (rich.ts reads them
      // back). Into an empty line, the first copied line comes whole too, its setting with it,
      // rather than taking the empty line's; into a line with text in it, it joins that line.
      // Plain text has no setting of its own, so it takes the line's.
      transformPasted(slice, view, plain) {
        const { $from, empty } = view.state.selection;
        const blank = empty && $from.parent.type === schema.nodes.paragraph && $from.parent.content.size === 0;
        const openStart = blank && !plain ? 0 : slice.openStart;
        return new Slice(withoutAttachments(slice.content), openStart, slice.openEnd);
      },
      // Lists typed as on the phone: "- " a dashed list, "* " a bulleted one, "1. " a numbered
      // one; in a dashed line "[ ] " or "[x] " makes it a checklist item, so "- [ ] " works
      // (icloud.com's notes3 userDidInsert). The typed marker goes; one undo brings it back.
      handleTextInput(view, from, to, text) {
        if (text !== " " || from !== to) return false;
        const $from = view.state.doc.resolve(from);
        const line = $from.parent;
        if (line.type !== schema.nodes.paragraph) return false;
        const typed = line.textBetween(0, $from.parentOffset, undefined, "\ufffc") + " ";
        const style = line.attrs.style as number | null;
        const inList = style !== null && LIST_STYLES.includes(style);
        let next: { style: number; checked: boolean | null; start?: number | null } | null = null;
        const numbered = /^([0-9]{1,6})\. $/.exec(typed);
        if (style === STYLE.dashed && (typed === "[ ] " || typed === "[x] ")) {
          next = { style: STYLE.checklist, checked: typed === "[x] " };
        } else if (!inList) {
          if (typed === "- ") next = { style: STYLE.dashed, checked: null };
          else if (typed === "* ") next = { style: STYLE.bulleted, checked: null };
          // "5. " starts the list at 5, as Notes keeps it.
          else if (numbered) next = { style: STYLE.numbered, checked: null, start: Number(numbered[1]) === 1 ? null : Number(numbered[1]) };
        }
        if (!next) return false;
        const tr = view.state.tr.delete($from.start(), from);
        tr.setNodeMarkup($from.before(), undefined, { ...line.attrs, style: next.style, checked: next.checked, start: next.start ?? null });
        view.dispatch(tr.scrollIntoView());
        return true;
      },
      // Dragging within the note moves text, as Notes does. Never an attachment: a picture or
      // table dragged was dropped as its cells' text, the table itself gone from the note (and a
      // drag let go where it began still wrote the note back). A drop where the dragged text
      // already lies changes nothing and is not made.
      handleDrop(view, event, slice, moved) {
        if (sliceHasAttachment(slice)) return true;
        if (!moved) return false;
        const at = view.posAtCoords({ left: event.clientX, top: event.clientY })?.pos;
        const { from, to } = view.state.selection;
        return at === undefined || (at >= from && at <= to);
      },
      handleDOMEvents: {
        dragstart: (vw, event) => {
          if (!sliceHasAttachment(vw.state.selection.content())) return false;
          event.preventDefault();
          return true;
        },
        click: (_vw, event) => openLinkAt(event),
        blur: () => {
          if (later.current !== null) report();
          // Into the Add Link box: still editing, and the note is left only once it closes.
          if (!linking.current) callbacks.current.onBlur?.();
          return false;
        },
        // A checklist item's circle ticks it, as a tap does on the phone.
        mousedown: (vw, event) => {
          const line = (event.target as HTMLElement).closest?.('p[data-style="103"]') as HTMLElement | null;
          if (!line || !editable) return false;
          // A line holding a table or picture has no circle to press.
          if (line.querySelector(":scope > .nf-attachment:is(.is-table, .is-image, .is-doc, .is-drawing, .is-table-unread)")) return false;
          const box = line.getBoundingClientRect();
          const pad = parseFloat(getComputedStyle(line).paddingInlineStart) || 0;
          if (event.clientX - box.left > pad) return false;
          const pos = vw.posAtDOM(line, 0) - 1;
          const node = vw.state.doc.nodeAt(pos);
          if (!node) return false;
          event.preventDefault();
          vw.dispatch(tickLine(vw.state, pos, !node.attrs.checked));
          return true;
        },
      },
    });
    view.current = v;
    tell(v.state);
    if (autoFocus) {
      v.focus();
      v.dispatch(v.state.tr.setSelection(TextSelection.atEnd(v.state.doc)));
    }
    return () => {
      if (later.current !== null) report();
      v.destroy();
      view.current = null;
    };
    // One editor per note: the parent keys it by the note, and replace() takes later text.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    view.current?.setProps({ editable: () => editable });
  }, [editable]);

  // The search's words, marked; a new search marks its own, and leaving it clears them.
  const wordsKey = (words ?? []).join("\u0000");
  useEffect(() => {
    const v = view.current;
    if (!v) return;
    const now = words ?? [];
    const was = found.getState(v.state)?.words ?? [];
    if (was.join("\u0000") === now.join("\u0000")) return;
    v.dispatch(v.state.tr.setMeta(found, now).setMeta("addToHistory", false));
    // The first place it is found is brought into view, as Notes opens a result there — not
    // while someone is writing in the note, whose place is their own.
    if (now.length && !v.hasFocus()) {
      const first = v.dom.querySelector<HTMLElement>(".note-found");
      const scroller = v.dom.closest<HTMLElement>(".note-editor");
      if (first && scroller) {
        const box = scroller.getBoundingClientRect();
        const at = first.getBoundingClientRect();
        if (at.top < box.top || at.bottom > box.bottom) first.scrollIntoView({ block: "center" });
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wordsKey]);

  useImperativeHandle(ref, () => ({
    setStyle: (style) => {
      const v = view.current;
      if (!v) return;
      restyle(style)(v.state, v.dispatch);
      v.focus();
    },
    toggleChecklist: () => {
      const v = view.current;
      if (!v) return;
      toggleChecklistCmd(v.state, v.dispatch);
      v.focus();
    },
    setColour: (c) => {
      const v = view.current;
      if (!v) return;
      colour(c)(v.state, v.dispatch);
      v.focus();
    },
    indent: (by) => {
      const v = view.current;
      if (!v) return;
      shift(by, false)(v.state, v.dispatch);
      v.focus();
    },
    moveLines: (by) => {
      const v = view.current;
      if (!v) return;
      moveLines(by)(v.state, v.dispatch);
      v.focus();
    },
    toggleQuote: () => {
      const v = view.current;
      if (!v) return;
      toggleQuote(v.state, v.dispatch);
      v.focus();
    },
    toggleMark: (mark) => {
      const v = view.current;
      if (!v) return;
      toggleMark(schema.marks[mark])(v.state, v.dispatch);
      v.focus();
    },
    replace: (nextText, nextRuns) => {
      const v = view.current;
      if (!v) return;
      // A key typed this very moment is not yet reported: send it first, and keep what is on
      // screen — the parent sees it pending and does not lay Apple's copy over it. Replacing
      // now lost that key, and the report that followed sent the replaced note.
      if (later.current !== null) {
        report();
        return;
      }
      const doc = toDoc(nextText, nextRuns);
      const was = v.state.doc;
      if (doc.eq(was)) return;
      // What is on screen and stays still: the caret's line when it is in view, or else the
      // first line in view — measured now, put back where it was once the change is in.
      const keep = steady(v);
      // Only the part that differs is replaced, and the caret is carried through the change:
      // a line added above it on the phone leaves it on its own character. Replaced whole, the
      // caret was put back by its count from the start, and that count was now another line;
      // and every line was drawn again, a picture in it fetched and decoded again.
      let tr = v.state.tr;
      const start = was.content.findDiffStart(doc.content);
      const end = was.content.findDiffEnd(doc.content);
      if (start !== null && end) {
        let { a, b } = end;
        // The ends cross where the change repeats what is next to it ("aa" to "aaa"): it is
        // the same change, counted on from the start.
        const over = start - Math.min(a, b);
        if (over > 0) {
          a += over;
          b += over;
        }
        tr.replace(start, a, doc.slice(start, b));
      }
      if (!tr.doc.eq(doc)) {
        // A change the slice did not fit as it is (never seen, but never risked): the whole
        // note, the caret kept by its place, as before.
        const { from, to } = v.state.selection;
        tr = v.state.tr.replaceWith(0, was.content.size, doc.content);
        const size = tr.doc.content.size;
        const near = (at: number) => tr.doc.resolve(Math.max(0, Math.min(at, size)));
        tr.setSelection(TextSelection.between(near(from), near(to)));
      }
      tr.setMeta("addToHistory", false);
      // Taken in quietly: what Apple holds is not an edit to send back.
      const next = v.state.apply(tr);
      v.updateState(next);
      if (keep) keep(tr.mapping);
      tell(next);
    },
    openLink: () => openLinkRef.current(),
    focus: () => view.current?.focus(),
    flushChange: () => {
      if (later.current !== null) report();
    },
    current: () => (view.current ? fromDoc(view.current.state.doc) : null),
    caret: () => {
      const v = view.current;
      if (!v) return null;
      // The text up to the cursor, read the way the whole note is read, is how far in it is.
      return fromDoc(v.state.doc.cut(0, v.state.selection.head)).text.length;
    },
  }));

  const [cellMenu, setCellMenu] = useState<CellMenu | null>(null);
  useEffect(() => {
    openCellMenu = setCellMenu;
    return () => {
      if (openCellMenu === setCellMenu) openCellMenu = null;
    };
  }, []);

  return (
    <>
      <div ref={host} className="note-rich-host" />
      {cellMenu && <RowMenu at={cellMenu.at} entries={cellMenu.entries} onClose={() => setCellMenu(null)} />}
      {linkAsk && (
        <LinkBox
          ask={linkAsk}
          onDone={(href, words) => {
            const v = view.current;
            closeLink(true);
            if (v) setLink(href, words)(v.state, v.dispatch);
          }}
          onRemove={() => {
            const v = view.current;
            closeLink(true);
            if (v) setLink(null)(v.state, v.dispatch);
          }}
          onCancel={closeLink}
        />
      )}
    </>
  );
});

/**
 * A change taken in from elsewhere leaves the view where it was: the caret's line, when the
 * caret is in view, or else the first line in view, stays at the same height on the screen.
 * Measured before the change; the returned function, given how positions moved, scrolls the
 * note back by however far that line went. At the very top with no caret in view nothing is
 * kept: lines added at the top are what is there to be seen.
 */
function steady(v: EditorView): ((mapping: Transaction["mapping"]) => void) | null {
  const scroller = v.dom.closest<HTMLElement>(".note-editor");
  if (!scroller) return null;
  const box = scroller.getBoundingClientRect();
  const seen = (top: number, bottom: number) => bottom > box.top && top < box.bottom;
  let pos: number | null = null;
  const head = v.state.selection.head;
  if (v.hasFocus()) {
    const c = v.coordsAtPos(head);
    if (seen(c.top, c.bottom)) pos = head;
  }
  if (pos === null && scroller.scrollTop > 0) {
    for (const line of v.dom.children) {
      const r = line.getBoundingClientRect();
      if (r.bottom <= box.top) continue;
      try {
        pos = v.posAtDOM(line, 0);
      } catch {
        pos = null;
      }
      break;
    }
  }
  if (pos === null) return null;
  const at = pos;
  const top = v.coordsAtPos(at).top;
  return (mapping) => {
    // Carried past anything put in just where it stood: the line it was on, not the new one.
    const moved = mapping.map(at, 1);
    if (moved < 0 || moved > v.state.doc.content.size) return;
    const by = v.coordsAtPos(moved).top - top;
    if (Math.abs(by) >= 0.5) scroller.scrollTop += by;
  };
}

/**
 * A checklist item ticked or unticked, as one edit. With "Sort Checked Items Automatically"
 * on, the item goes where Notes sorts it in the same edit, so one undo puts both back.
 */
function tickLine(state: EditorState, pos: number, checked: boolean): Transaction {
  const node = state.doc.nodeAt(pos)!;
  const tr = state.tr.setNodeMarkup(pos, undefined, { ...node.attrs, checked });
  if (moveCheckedOn()) {
    const starts: number[] = [];
    state.doc.forEach((_, offset) => starts.push(offset));
    const move = checklistMove(state.doc, starts.indexOf(pos), checked);
    if (move) {
      const at = (i: number) => (i < starts.length ? starts[i] : state.doc.content.size);
      const slice = tr.doc.slice(at(move.from), at(move.to));
      const target = tr.mapping.map(at(move.before));
      if (move.before > move.from) {
        tr.insert(target, slice.content);
        tr.delete(at(move.from), at(move.to));
      } else {
        tr.delete(at(move.from), at(move.to));
        tr.insert(target, slice.content);
      }
    }
  }
  return tr;
}

/**
 * Mark as Checked (Shift-Command-U on a Mac): the checklist items the cursor or selection is
 * in ticked — or, when every one is ticked already, unticked. Lines that are not checklist
 * items are left as they are; the key does nothing outside a checklist.
 */
const tickSelected: Command = (state, dispatch) => {
  const items = linesIn(state).filter(({ node }) => node.attrs.style === STYLE.checklist);
  if (!items.length) return false;
  const checked = !items.every(({ node }) => node.attrs.checked);
  if (!dispatch) return true;
  // One line goes through the same tick a click on its circle makes, sorting included.
  if (items.length === 1) {
    dispatch(tickLine(state, items[0].pos, checked).scrollIntoView());
    return true;
  }
  const tr = state.tr;
  for (const { node, pos } of items) {
    if (Boolean(node.attrs.checked) !== checked) tr.setNodeMarkup(pos, undefined, { ...node.attrs, checked });
  }
  dispatch(tr);
  return true;
};

const toggleChecklistCmd: Command = (state, dispatch) => {
  const all = linesIn(state).every(({ node }) => node.attrs.style === STYLE.checklist);
  return chainCommands(restyle(all ? null : STYLE.checklist))(state, dispatch);
};
