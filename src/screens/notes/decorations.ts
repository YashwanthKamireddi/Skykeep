/**
 * What the editor draws over a note without writing it in: web addresses as links, each
 * numbered line's number, and the words of a search. Each is worked out over the whole note
 * once, when it opens; after that a change redraws only what it could have changed. Working
 * every one out again on each key read the whole note three times a keystroke — in a long note
 * that was a key drawn late — and a change only ever moves what is around it along.
 *
 * Kept free of the window (no React, no links followed) so the model test can hold what is
 * redrawn against a fresh full drawing (.uiverify/notes-decorations.test.mjs).
 */
import { Plugin, type Transaction } from "prosemirror-state";
import { Decoration, DecorationSet } from "prosemirror-view";
import type { Node as PMNode } from "prosemirror-model";
import { nextNumber, STYLE, type ListLine } from "./rich.ts";
import { foundRanges } from "./found.ts";

/** How much of the note the drawings have read — text nodes and their characters, and lines
 *  counted for numbers — so the model test can show what a keystroke costs. */
export const scanned = { textNodes: 0, chars: 0, lines: 0 };

type Span = { from: number; to: number } | null;
const spans = new WeakMap<Transaction, Span>();

/**
 * The part of the note, after `tr`, that differs from before it: where its steps wrote, and
 * where the document itself differs. Its steps alone miss a mark added or taken away (they
 * move nothing, so their maps are empty); the documents alone miss text replaced by the same
 * text, which still drops what was drawn there. Null when nothing changed.
 */
export function changedSpan(tr: Transaction): Span {
  if (!tr.docChanged) return null;
  // Worked out once a change, for all three drawings.
  if (spans.has(tr)) return spans.get(tr) ?? null;
  const span = spanOf(tr);
  spans.set(tr, span);
  return span;
}

function spanOf(tr: Transaction): Span {
  let from = Infinity;
  let to = -Infinity;
  const maps = tr.mapping.maps;
  maps.forEach((map, i) => {
    map.forEach((_oldFrom, _oldTo, start, end) => {
      const after = tr.mapping.slice(i + 1);
      from = Math.min(from, after.map(start, -1));
      to = Math.max(to, after.map(end, 1));
    });
  });
  const before = tr.before.content;
  const now = tr.doc.content;
  const start = before.findDiffStart(now);
  if (start != null) {
    const end = before.findDiffEnd(now);
    const b = end ? end.b : now.size;
    from = Math.min(from, start, b);
    to = Math.max(to, start, b);
  }
  if (from > to) return null;
  return { from: Math.max(0, from), to: Math.min(now.size, to) };
}

type Scan = (node: PMNode, pos: number) => Decoration[];

/** Every text node of the note read with `scan`. */
function drawAll(doc: PMNode, scan: Scan): DecorationSet {
  const out: Decoration[] = [];
  doc.descendants((node, pos) => {
    if (node.isText) out.push(...scan(node, pos));
    return true;
  });
  return DecorationSet.create(doc, out);
}

/**
 * What was drawn over the text moved along with the change, and the paragraphs the change
 * touched read again — whole paragraphs, as an address or a word can run across the place
 * typed in, and one paragraph either side of a change between them.
 */
function redrawText(tr: Transaction, was: DecorationSet, scan: Scan): DecorationSet {
  const span = changedSpan(tr);
  const set = was.map(tr.mapping, tr.doc);
  if (!span) return set;
  const doc = tr.doc;
  const blocks: { node: PMNode; pos: number }[] = [];
  doc.nodesBetween(Math.max(0, span.from - 1), Math.min(doc.content.size, span.to + 1), (node, pos) => {
    if (!node.isTextblock) return true;
    blocks.push({ node, pos });
    return false;
  });
  if (!blocks.length) return set;
  const from = blocks[0].pos;
  const last = blocks[blocks.length - 1];
  const to = last.pos + last.node.nodeSize;
  const fresh: Decoration[] = [];
  for (const { node, pos } of blocks) {
    node.forEach((child, offset) => {
      if (child.isText) fresh.push(...scan(child, pos + 1 + offset));
    });
  }
  let cleared = set.remove(set.find(from, to));
  // A deletion can map a drawing across the edge of a paragraph (from the gap before it into its
  // text). `find` returns such a one but `remove` can't place it in its tree, and it stayed drawn
  // over text it no longer matched (seed 305 of the model test). Then, and only then, the set is
  // made again without anything over the paragraphs read; no drawing outside them reaches in.
  if (cleared.find(from, to).length) {
    cleared = DecorationSet.create(doc, cleared.find().filter((d) => d.to <= from || d.from >= to));
  }
  return cleared.add(doc, fresh);
}

/** An address: with its scheme, or starting www. — ending before closing punctuation, as a
 * sentence's full stop or a bracket is not part of it. */
const ADDRESS = /\b(?:https?:\/\/|www\.)[^\s<>"'`]*[^\s<>"'`.,;:!?)\]}]/gi;

export type Found = { from: number; to: number; href: string };

/** The addresses in a piece of text, with where each starts and ends, and what it opens. */
export function addressesIn(text: string): Found[] {
  const out: Found[] = [];
  for (const m of text.matchAll(ADDRESS)) {
    const from = m.index ?? 0;
    const href = /^www\./i.test(m[0]) ? `https://${m[0]}` : m[0];
    out.push({ from, to: from + m[0].length, href });
  }
  return out;
}

const addressScan: Scan = (node, pos) => {
  if (!node.text) return [];
  // A link of its own (made on the phone, or here) is drawn as that already.
  if (node.marks.some((m) => m.type.name === "link")) return [];
  scanned.textNodes++;
  scanned.chars += node.text.length;
  return addressesIn(node.text).map((a) =>
    Decoration.inline(pos + a.from, pos + a.to, { nodeName: "a", href: a.href, class: "nf-autolink" }),
  );
};

/** Every address in the note, drawn as a link. */
export const linksIn = (doc: PMNode): DecorationSet => drawAll(doc, addressScan);

/** Addresses found again only where the text changed; a selection moving keeps them. */
export const autolink = new Plugin<DecorationSet>({
  state: {
    init: (_, state) => linksIn(state.doc),
    apply: (tr, was) => (tr.docChanged ? redrawText(tr, was, addressScan) : was),
  },
  props: {
    decorations(state) {
      return this.getState(state);
    },
  },
});

const lineOf = (node: PMNode) => node.attrs as ListLine;
const numberAt = (n: number, from: number, to: number) => Decoration.node(from, to, { "data-n": String(n) }, { n });

/** Each numbered line's number (`data-n`, which the stylesheet draws), counted as Notes counts
 *  them (rich.ts listNumbers). */
export function numbering(doc: PMNode): DecorationSet {
  const counts: number[] = [];
  const marks: Decoration[] = [];
  doc.forEach((node, offset) => {
    scanned.lines++;
    const n = nextNumber(counts, lineOf(node));
    if (n !== null) marks.push(numberAt(n, offset, offset + node.nodeSize));
  });
  return DecorationSet.create(doc, marks);
}

/** The number drawn on the line at `pos`, if there is one. */
function drawnNumber(set: DecorationSet, pos: number, size: number): Decoration | undefined {
  return set.find(pos, pos + size).find((d) => d.from === pos && d.to === pos + size);
}

/**
 * The numbers counted again from the lines the change touched, and down past them only as far
 * as they come out different. A line's number depends only on the lines above it back to the
 * nearest top-level item (whose number is drawn already) or the nearest line that is not
 * numbered; below the change, a line that is not numbered, or a top-level item that keeps its
 * number, leaves every line after it as it was.
 */
function renumber(tr: Transaction, was: DecorationSet): DecorationSet {
  const span = changedSpan(tr);
  let set = was.map(tr.mapping, tr.doc);
  if (!span) return set;
  const doc = tr.doc;
  if (!doc.childCount) return DecorationSet.empty;
  const index = (pos: number) => Math.min(doc.childCount - 1, doc.resolve(pos).index(0));
  // A line either side, as a change at a line's edge may be read as the next line's.
  const first = Math.max(0, index(span.from) - 1);
  const last = Math.min(doc.childCount - 1, index(span.to) + 1);
  let pos = doc.resolve(0).posAtIndex(first, 0);

  // Back to a line whose counts are known.
  let i = first;
  let counts: number[] = [];
  let at = pos;
  while (i > 0) {
    const above = doc.child(i - 1);
    const abovePos = at - above.nodeSize;
    scanned.lines++;
    const line = lineOf(above);
    if (line.style !== STYLE.numbered) break;
    if (Math.max(0, line.indent | 0) === 0) {
      const drawn = drawnNumber(set, abovePos, above.nodeSize);
      if (drawn) {
        counts = [drawn.spec.n as number];
        break;
      }
    }
    i--;
    at = abovePos;
  }
  pos = at;

  const stale: Decoration[] = [];
  const fresh: Decoration[] = [];
  for (; i < doc.childCount; i++) {
    const node = doc.child(i);
    scanned.lines++;
    const line = lineOf(node);
    const n = nextNumber(counts, line);
    const drawn = drawnNumber(set, pos, node.nodeSize);
    const had = drawn ? (drawn.spec.n as number) : null;
    if (had !== n) {
      if (drawn) stale.push(drawn);
      if (n !== null) fresh.push(numberAt(n, pos, pos + node.nodeSize));
    }
    pos += node.nodeSize;
    if (i >= last && had === n && (n === null || Math.max(0, line.indent | 0) === 0)) break;
  }
  if (stale.length) set = set.remove(stale);
  return fresh.length ? set.add(doc, fresh) : set;
}

export const numbered: Plugin<DecorationSet> = new Plugin<DecorationSet>({
  state: {
    init: (_, state) => numbering(state.doc),
    apply: (tr, was) => (tr.docChanged ? renumber(tr, was) : was),
  },
  props: { decorations: (state): DecorationSet | undefined => numbered.getState(state) },
});

/**
 * The words of a search marked in the note, as Notes marks them in a note opened from its
 * results. Only drawn — never part of the note. A new search marks the whole note again; a
 * change to the text marks again only the paragraphs it touched; with no search, nothing is
 * walked at all.
 */
type FoundState = { words: string[]; marks: DecorationSet };
const wordScan =
  (words: string[]): Scan =>
  (node, pos) => {
    if (!node.text) return [];
    scanned.textNodes++;
    scanned.chars += node.text.length;
    return foundRanges(node.text, words).map(([a, b]) => Decoration.inline(pos + a, pos + b, { class: "note-found" }));
  };

export function foundMarks(doc: PMNode, words: string[]): DecorationSet {
  return words.length ? drawAll(doc, wordScan(words)) : DecorationSet.empty;
}

export const found: Plugin<FoundState> = new Plugin<FoundState>({
  state: {
    init: () => ({ words: [], marks: DecorationSet.empty }),
    apply: (tr, was) => {
      const words = tr.getMeta(found) as string[] | undefined;
      if (words) return { words, marks: foundMarks(tr.doc, words) };
      if (!tr.docChanged || !was.words.length) return was;
      return { words: was.words, marks: redrawText(tr, was.marks, wordScan(was.words)) };
    },
  },
  props: { decorations: (state): DecorationSet | undefined => found.getState(state)?.marks },
});
