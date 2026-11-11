/**
 * A note as an editor holds it, and back: Apple's text and attribute runs turned into a
 * ProseMirror document of paragraphs — each with Notes' style, indent and checklist tick —
 * holding text with bold, italic, underline, strikethrough and links, and that document
 * turned back into exactly the text and runs Apple keeps.
 *
 * The round trip is lossless for everything the editor shows: a note read in and written
 * straight back gives the same text and the same runs. What it does not show — fonts,
 * colours, an attachment's insides — the backend keeps from Apple's own copy, so it is never
 * at stake here. An attachment's place in the text (U+FFFC) and a line break inside a
 * paragraph (U+2028, Shift-Return) travel as nodes of their own, so they cannot be typed
 * over by accident or lost.
 */
import { Schema, type DOMOutputSpec, type Node as PMNode, type Mark } from "prosemirror-model";
import type { Run } from "./Formatted";
import { refusedLink, linkedNote, outsideHref } from "../../lib/links.ts";

/** Apple's paragraph styles. */
export const STYLE = {
  title: 0,
  heading: 1,
  subheading: 2,
  body: 3,
  mono: 4,
  bulleted: 100,
  dashed: 101,
  numbered: 102,
  checklist: 103,
} as const;

export const LIST_STYLES: readonly number[] = [STYLE.bulleted, STYLE.dashed, STYLE.numbered, STYLE.checklist];
const ATTACHMENT = "￼";

/** Each table's cells by attachment id, as last drawn: what copying the table puts out. */
export const tableCells = new Map<string, string[][]>();
const LINE_BREAK = " ";

/** A paragraph's settings from the element its toDOM made; nothing set when it is not one. */
function paragraphAttrs(p: HTMLElement) {
  const raw = p.getAttribute("data-style");
  const n = raw === null || raw === "none" ? NaN : Number(raw);
  const style = Number.isInteger(n) && Object.values(STYLE).includes(n as never) ? n : null;
  const css = p.getAttribute("style") ?? "";
  const indent = Number(/--indent:\s*(\d+)/.exec(css)?.[1] ?? 0);
  const start = Number(/counter-set:\s*nf-num\s+(\d+)/.exec(css)?.[1] ?? NaN);
  return {
    style,
    indent,
    checked: style === STYLE.checklist ? p.getAttribute("data-checked") === "true" : null,
    start: style === STYLE.numbered && start > 1 ? start : null,
    quote: p.getAttribute("data-quote") === "true",
  };
}

export const schema = new Schema({
  nodes: {
    doc: { content: "paragraph+" },
    paragraph: {
      content: "inline*",
      attrs: {
        style: { default: null as number | null },
        indent: { default: 0 },
        checked: { default: null as boolean | null },
        /** A numbered list's first number, when not 1. */
        start: { default: null as number | null },
        /** A block quote line. */
        quote: { default: false },
      },
      // Read back exactly what toDOM writes, so a copy pasted within Skykeep keeps each
      // line's style, tick, indent and numbering. A `<p>` from anywhere else has none of these
      // and stays plain body.
      parseDOM: [{ tag: "p", getAttrs: (d) => paragraphAttrs(d as HTMLElement) }],
      toDOM: (node) => [
        "p",
        {
          class: "nf-p",
          "data-style": node.attrs.style === null ? "none" : String(node.attrs.style),
          ...(node.attrs.checked ? { "data-checked": "true" } : {}),
          ...(node.attrs.quote ? { "data-quote": "true" } : {}),
          style: [
            node.attrs.indent ? `--indent: ${node.attrs.indent}` : "",
            // CSS sets a counter after counting the line, so this line shows the start.
            node.attrs.style === 102 && node.attrs.start ? `counter-set: nf-num ${node.attrs.start}` : "",
          ].filter(Boolean).join("; "),
        },
        0,
      ],
    },
    text: { group: "inline" },
    /** Where an attachment sits — a picture, a sketch, a table — kept, not edited. */
    attachment: {
      group: "inline",
      inline: true,
      atom: true,
      selectable: true,
      attrs: { id: { default: null as string | null } },
      // Copied, a table is its cells — as a table, and as tab-separated lines for plain text —
      // never a placeholder word; anything else copies as nothing.
      leafText: (node) => {
        const cells = tableCells.get(node.attrs.id as string);
        return cells ? cells.map((r) => r.map((c) => c.trimEnd()).join("\t")).join("\n") : "";
      },
      toDOM: (node): DOMOutputSpec => {
        const cells = tableCells.get(node.attrs.id as string);
        if (!cells) return ["span", { class: "nf-attachment" }];
        return ["table", ["tbody", ...cells.map((r): DOMOutputSpec => ["tr", ...r.map((c): DOMOutputSpec => ["td", c.trimEnd()])])]];
      },
    },
    /** Shift-Return: a new line in the same paragraph. */
    line_break: {
      group: "inline",
      inline: true,
      selectable: false,
      parseDOM: [{ tag: "br" }],
      toDOM: () => ["br"],
    },
  },
  marks: {
    bold: { parseDOM: [{ tag: "strong" }, { tag: "b" }], toDOM: () => ["strong", 0] },
    italic: { parseDOM: [{ tag: "em" }, { tag: "i" }], toDOM: () => ["em", 0] },
    underline: { parseDOM: [{ tag: "u" }], toDOM: () => ["u", 0] },
    strike: { parseDOM: [{ tag: "s" }, { tag: "del" }], toDOM: () => ["s", 0] },
    /** Text colour set on a phone: 1 purple, 2 pink, 3 orange, 4 mint, 5 blue. */
    emphasis: {
      attrs: { color: {} },
      parseDOM: [{ tag: "span[data-emphasis]", getAttrs: (d) => ({ color: Number((d as HTMLElement).dataset.emphasis) }) }],
      toDOM: (m) => ["span", { class: `nf-em-${m.attrs.color}`, "data-emphasis": String(m.attrs.color) }, 0],
    },
    link: {
      attrs: { href: {} },
      inclusive: false,
      parseDOM: [{ tag: "a[href]", getAttrs: (d) => ({ href: (d as HTMLElement).getAttribute("href") }) }],
      // No target: a click is the editor's to follow (notes/links), never a new window — one
      // opened by WebKitGTK for a link to a note went to the browser.
      // A link Skykeep will not open keeps its address (the note is left as Apple has it), but
      // says so where it stands; a click on it is refused (lib/follow.ts).
      toDOM: (m) =>
        outsideHref(m.attrs.href) || linkedNote(m.attrs.href)
          ? ["a", { href: m.attrs.href, rel: "noreferrer" }, 0]
          : ["a", { href: m.attrs.href, rel: "noreferrer", class: "nf-link-refused", title: refusedLink() }, 0],
    },
  },
});

const plain = (length: number): Run => ({
  length, style: null, indent: 0, checked: null, bold: false, italic: false, underline: false, strike: false, link: null,
});

/** Whether `runs` describe exactly this text. */
export function runsFit(text: string, runs: Run[] | null): runs is Run[] {
  return !!runs && runs.length > 0 && runs.reduce((n, r) => n + r.length, 0) === text.length;
}

/** The run each UTF-16 unit of the text lies in. */
function perUnit(text: string, runs: Run[]): Run[] {
  const out: Run[] = [];
  for (const r of runs) for (let i = 0; i < r.length; i++) out.push(r);
  while (out.length < text.length) out.push(runs[runs.length - 1] ?? plain(1));
  return out;
}

function marksOf(run: Run): Mark[] {
  const m: Mark[] = [];
  if (run.bold) m.push(schema.marks.bold.create());
  if (run.italic) m.push(schema.marks.italic.create());
  if (run.underline) m.push(schema.marks.underline.create());
  if (run.strike) m.push(schema.marks.strike.create());
  if (run.emphasis) m.push(schema.marks.emphasis.create({ color: run.emphasis }));
  if (run.link) m.push(schema.marks.link.create({ href: run.link }));
  return m;
}

/** Apple's text and runs as a document. Without runs that fit, every line is plain body. */
export function toDoc(text: string, runs: Run[] | null): PMNode {
  const fits = runsFit(text, runs);
  const units = fits ? perUnit(text, runs) : [];
  const paragraphs: PMNode[] = [];
  let start = 0;
  for (;;) {
    const nl = text.indexOf("\n", start);
    const end = nl < 0 ? text.length : nl;
    // A paragraph is set by the run its newline is in — or, for the last line, its last unit.
    // The empty line after a note's closing newline has no run of its own: it is plain text, as
    // Notes shows it. Taking the newline's run before it drew an empty checklist item (or bullet)
    // under the last one, which the phone never shows.
    const trailing = nl < 0 && start === text.length && start > 0;
    const own = fits && !trailing ? (units[nl < 0 ? Math.max(end - 1, start) : nl] ?? units[units.length - 1]) : null;
    const inline: PMNode[] = [];
    let i = start;
    while (i < end) {
      const ch = text[i];
      if (ch === ATTACHMENT) {
        inline.push(schema.nodes.attachment.create({ id: fits ? (units[i]?.attachment ?? null) : null }));
        i += 1;
        continue;
      }
      if (ch === LINE_BREAK) {
        inline.push(schema.nodes.line_break.create());
        i += 1;
        continue;
      }
      let j = i + 1;
      while (j < end && text[j] !== ATTACHMENT && text[j] !== LINE_BREAK && (!fits || units[j] === units[i])) j++;
      inline.push(schema.text(text.slice(i, j), fits ? marksOf(units[i]) : []));
      i = j;
    }
    paragraphs.push(
      schema.nodes.paragraph.create(
        {
          style: own?.style ?? null,
          indent: own?.indent ?? 0,
          checked: own?.style === STYLE.checklist ? Boolean(own.checked) : null,
          start: own?.style === STYLE.numbered ? (own.start ?? null) : null,
          quote: Boolean(own?.quote),
        },
        inline,
      ),
    );
    if (nl < 0) break;
    start = nl + 1;
  }
  return schema.nodes.doc.create(null, paragraphs);
}

const same = (a: Run, b: Run) =>
  a.style === b.style && a.indent === b.indent && a.checked === b.checked && a.bold === b.bold &&
  a.italic === b.italic && a.underline === b.underline && a.strike === b.strike && a.link === b.link &&
  (a.attachment ?? null) === (b.attachment ?? null) && (a.emphasis ?? null) === (b.emphasis ?? null) &&
  (a.start ?? null) === (b.start ?? null) && Boolean(a.quote) === Boolean(b.quote);

/** A document as Apple's text and runs. */
export function fromDoc(doc: PMNode): { text: string; runs: Run[] } {
  let text = "";
  const runs: Run[] = [];
  const push = (piece: string, run: Run) => {
    if (!piece) return;
    text += piece;
    const last = runs[runs.length - 1];
    if (last && same(last, run)) last.length += piece.length;
    else runs.push({ ...run, length: piece.length });
  };
  doc.forEach((para, _offset, index) => {
    const p = {
      style: para.attrs.style as number | null,
      indent: para.attrs.indent as number,
      checked: para.attrs.style === STYLE.checklist ? Boolean(para.attrs.checked) : null,
      ...(para.attrs.style === STYLE.numbered && para.attrs.start ? { start: para.attrs.start as number } : {}),
      ...(para.attrs.quote ? { quote: true } : {}),
    };
    const base: Run = { ...plain(0), ...p };
    para.forEach((child) => {
      if (child.type === schema.nodes.attachment) return push(ATTACHMENT, { ...base, attachment: child.attrs.id ?? null });
      if (child.type === schema.nodes.line_break) return push(LINE_BREAK, base);
      const has = (name: string) => child.marks.some((m) => m.type.name === name);
      const emphasis = child.marks.find((m) => m.type.name === "emphasis")?.attrs.color as number | undefined;
      push(child.text ?? "", {
        ...base,
        ...(emphasis ? { emphasis } : {}),
        bold: has("bold"),
        italic: has("italic"),
        underline: has("underline"),
        strike: has("strike"),
        link: (child.marks.find((m) => m.type.name === "link")?.attrs.href as string | undefined) ?? null,
      });
    });
    // The newline carries the paragraph's setting, as Apple stores it.
    if (index < doc.childCount - 1) push("\n", base);
  });
  return { text, runs };
}

/** The one stretch `to` changed from `from`, in UTF-16 units: where, removed, inserted. */
function change(from: string, to: string): [number, number, number] {
  let pre = 0;
  const max = Math.min(from.length, to.length);
  while (pre < max && from[pre] === to[pre]) pre++;
  let suf = 0;
  while (suf < max - pre && from[from.length - 1 - suf] === to[to.length - 1 - suf]) suf++;
  return [pre, from.length - pre - suf, to.length - pre - suf];
}

/**
 * `runs`, which describe `from`, laid over `to` — the same text with another device's edit
 * merged in. What is new takes the setting of what is just before it. The backend does the
 * same with the same rule, so the screen and Apple agree.
 */
export function carryRuns(from: string, runs: Run[], to: string): Run[] {
  if (!runsFit(from, runs)) return [plain(to.length)];
  const units = perUnit(from, runs);
  const [at, removed, inserted] = change(from, to);
  const fill = at > 0 ? units[at - 1] : (units[at + removed] ?? plain(1));
  const next = [...units.slice(0, at), ...Array<Run>(inserted).fill(fill), ...units.slice(at + removed)];
  const out: Run[] = [];
  for (const r of next) {
    const last = out[out.length - 1];
    if (last && same(last, r)) last.length += 1;
    else out.push({ ...r, length: 1 });
  }
  return out;
}

/**
 * `runs` over `text` in one shape, so two descriptions of the same formatting compare equal
 * however they are cut — Apple's runs come in pieces (a word split in two), the editor's
 * merged. Each unit takes its own character attributes and the setting of the unit that ends
 * its line; neighbours differing only in length are one run; an attachment's run is never
 * joined to anything. The backend has the same rule (notes.rs `canonical_runs`). Null when
 * the runs do not describe the text.
 */
export function canonicalRuns(text: string, runs: Run[] | null): Run[] | null {
  if (!runsFit(text, runs)) return null;
  const units = perUnit(text, runs);
  const out: Run[] = [];
  let end = text.length - 1;
  const ends: number[] = new Array(text.length);
  for (let i = text.length - 1; i >= 0; i--) {
    if (text[i] === "\n") end = i;
    ends[i] = end;
  }
  for (let i = 0; i < text.length; i++) {
    const own = units[i];
    const line = units[ends[i]];
    const style = line.style ?? null;
    const unit: Run = {
      length: 1,
      style,
      indent: line.indent ?? 0,
      checked: style === STYLE.checklist ? Boolean(line.checked) : null,
      bold: Boolean(own.bold),
      italic: Boolean(own.italic),
      underline: Boolean(own.underline),
      strike: Boolean(own.strike),
      link: own.link || null,
      ...(style === STYLE.numbered && line.start && line.start !== 1 ? { start: line.start } : {}),
      ...(line.quote ? { quote: true } : {}),
      ...(own.emphasis ? { emphasis: own.emphasis } : {}),
      ...(own.attachment ? { attachment: own.attachment } : {}),
    };
    const last = out[out.length - 1];
    if (last && !last.attachment && !unit.attachment && same(last, unit)) last.length += 1;
    else out.push(unit);
  }
  return out;
}

/** Whether `a` and `b` set `text` the same way, however each is cut. */
export function sameFormatting(text: string, a: Run[] | null, b: Run[] | null): boolean {
  const x = canonicalRuns(text, a);
  const y = canonicalRuns(text, b);
  return x !== null && y !== null && JSON.stringify(x) === JSON.stringify(y);
}

/**
 * The number each line of a numbered list shows, as Notes numbers them (and as Markdown export
 * does, markdown.rs): a list nested in another counts from 1 on its own, and the list around it
 * carries on after it; a line that is not numbered ends every list; a list's first item may
 * start it elsewhere. Null for a line that is not numbered. One count for every depth had a
 * nested list's items counted into the outer list, so it went on from 4 where the phone shows 2.
 */
export function listNumbers(lines: { style: number | null; indent: number; start?: number | null }[]): (number | null)[] {
  const counts: number[] = [];
  return lines.map((line) => {
    if (line.style !== STYLE.numbered) {
      counts.length = 0;
      return null;
    }
    const depth = Math.max(0, line.indent | 0);
    counts.length = Math.min(counts.length, depth + 1);
    while (counts.length <= depth) counts.push(0);
    counts[depth] = counts[depth] === 0 && line.start ? line.start : counts[depth] + 1;
    return counts[depth];
  });
}

/**
 * Where a checklist item goes when it is ticked or unticked with "Sort Checked Items
 * Automatically" on, as Notes sorts them: ticked, the item — with the items nested under it — goes
 * to the end of its own checklist; unticked, back up to just above the ticked items. Only
 * within the run of checklist lines it sits in, and only for a top-level item (a nested one
 * moves with its parent). `index` is the item's place among the document's lines; `checked`
 * its state after the change. The answer is which lines move (`from` up to `to`, not
 * including it) and the line they go before (`before`, measured in the document as it was);
 * `null` when nothing needs to move.
 */
export function checklistMove(doc: PMNode, index: number, checked: boolean): { from: number; to: number; before: number } | null {
  const lines: PMNode[] = [];
  doc.forEach((n) => lines.push(n));
  const isItem = (i: number) => lines[i]?.type.name === "paragraph" && lines[i].attrs.style === STYLE.checklist;
  const indent = (i: number) => Number(lines[i]?.attrs.indent ?? 0);
  if (!isItem(index) || indent(index) !== 0) return null;
  let start = index;
  while (start > 0 && isItem(start - 1)) start--;
  let end = index + 1;
  while (end < lines.length && isItem(end)) end++;
  // The item and its nested items.
  let to = index + 1;
  while (to < end && indent(to) > 0) to++;
  // Top-level items of the run other than this one, by their first line.
  const tops: number[] = [];
  for (let i = start; i < end; i++) if (indent(i) === 0 && (i < index || i >= to)) tops.push(i);
  if (checked) {
    // To the end of the run, unless it is already below every other top-level item.
    if (tops.every((i) => i < index)) return null;
    return { from: index, to, before: end };
  }
  // Just above the first ticked item; where none is above it, it stays.
  const firstTicked = tops.find((i) => lines[i].attrs.checked === true);
  if (firstTicked === undefined || firstTicked > index) return null;
  return { from: index, to, before: firstTicked };
}
