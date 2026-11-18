// What the editor draws over a note — addresses as links, list numbers, a search's words — is
// redrawn after each change only where the change could reach. Held here against a fresh full
// drawing of the whole note after every one of thousands of random edits (a fixed seed, so a
// failure comes back the same), and counted: a key typed in a long note reads one paragraph,
// not the note.
import { test } from "node:test";
import assert from "node:assert/strict";
import { EditorState, TextSelection } from "prosemirror-state";
import { schema, STYLE } from "../src/screens/notes/rich.ts";
import { autolink, numbered, found, linksIn, numbering, foundMarks, scanned } from "../src/screens/notes/decorations.ts";

/** A small fixed-seed generator (mulberry32). */
function random(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const WORDS = ["note", "Notebook", "www.", "http://", "https://", "example.org", "/a", ".", " ", " ", "(", ")", "x", "nö", "Spring", "fox"];
const WORDS_SEARCHED = ["note", "spr"];
const STYLES = [null, STYLE.body, STYLE.numbered, STYLE.numbered, STYLE.numbered, STYLE.bulleted, STYLE.title];

function paragraph(rnd) {
  const style = STYLES[Math.floor(rnd() * STYLES.length)];
  const attrs = { style, indent: Math.floor(rnd() * 3), start: style === STYLE.numbered && rnd() < 0.15 ? 1 + Math.floor(rnd() * 9) : null };
  const parts = [];
  const n = Math.floor(rnd() * 5);
  for (let i = 0; i < n; i++) {
    const r = rnd();
    if (r < 0.1) parts.push(schema.nodes.line_break.create());
    else if (r < 0.15) parts.push(schema.nodes.attachment.create());
    else {
      const text = Array.from({ length: 1 + Math.floor(rnd() * 4) }, () => WORDS[Math.floor(rnd() * WORDS.length)]).join("");
      const marks = rnd() < 0.15 ? [schema.marks.link.create({ href: "https://example.org" })] : rnd() < 0.2 ? [schema.marks.bold.create()] : [];
      parts.push(schema.text(text, marks));
    }
  }
  return schema.nodes.paragraph.create(attrs, parts);
}

const docOf = (rnd, lines) => schema.nodes.doc.create(null, Array.from({ length: lines }, () => paragraph(rnd)));

/** A random edit of the kinds typing and the editor's commands make. */
function edit(state, rnd) {
  const tr = state.tr;
  const size = () => tr.doc.content.size;
  const at = () => Math.floor(rnd() * (size() + 1));
  const textPos = () => {
    const $p = tr.doc.resolve(at());
    return $p.parent.inlineContent ? $p.pos : TextSelection.near($p).from;
  };
  const steps = 1 + Math.floor(rnd() * (rnd() < 0.8 ? 1 : 4));
  for (let s = 0; s < steps; s++) {
    const kind = Math.floor(rnd() * 10);
    try {
      if (kind <= 2) tr.insertText(WORDS[Math.floor(rnd() * WORDS.length)], textPos());
      else if (kind === 3) {
        const a = textPos();
        tr.delete(a, Math.min(size(), a + 1 + Math.floor(rnd() * 12)));
      } else if (kind === 4) tr.split(textPos());
      else if (kind === 5) {
        // A line restyled: numbered or not, indented, a new start.
        const target = Math.floor(rnd() * tr.doc.childCount);
        let pos = 0;
        for (let k = 0; k < target; k++) pos += tr.doc.child(k).nodeSize;
        const node = tr.doc.child(target);
        const style = STYLES[Math.floor(rnd() * STYLES.length)];
        tr.setNodeMarkup(pos, null, { ...node.attrs, style, indent: Math.floor(rnd() * 3), start: rnd() < 0.2 ? 2 + Math.floor(rnd() * 5) : null });
      } else if (kind === 6) {
        const a = textPos();
        const b = Math.min(size(), a + Math.floor(rnd() * 20));
        if (rnd() < 0.5) tr.addMark(a, b, schema.marks.link.create({ href: "https://example.org/x" }));
        else tr.removeMark(a, b, schema.marks.link);
      } else if (kind === 7) {
        // Lines removed whole, or pasted in.
        const a = textPos();
        const b = Math.min(size(), a + Math.floor(rnd() * 80));
        if (rnd() < 0.5) tr.delete(a, b);
        else tr.replaceWith(tr.doc.resolve(a).before(1), tr.doc.resolve(a).before(1), [paragraph(rnd), paragraph(rnd)]);
      } else if (kind === 8) tr.join(tr.doc.resolve(textPos()).before(1) || 0);
      else if (rnd() < 0.1) tr.replaceWith(0, size(), tr.doc.content);
      else tr.insertText(" ", textPos());
    } catch {
      // An edit that does not fit where it landed (a join at the first line) is just skipped.
    }
  }
  return tr;
}

/** A set of decorations as plain values, in order, to compare. */
const plain = (set, doc) =>
  set
    .find(0, doc.content.size)
    .map((d) => JSON.stringify([d.from, d.to, d.type.attrs]))
    .sort();

test("redrawing after each edit draws exactly what drawing the whole note again would", () => {
  // Seed 305: a deletion that mapped a search mark across a paragraph's edge (decorations.ts).
  const seeds = [...Array.from({ length: Number(process.env.SEEDS ?? 40) }, (_, i) => i + 1), 305];
  for (const seed of seeds) {
    const rnd = random(seed);
    let state = EditorState.create({ doc: docOf(rnd, 30), plugins: [autolink, numbered, found] });
    state = state.apply(state.tr.setMeta(found, WORDS_SEARCHED));
    for (let n = 0; n < 150; n++) {
      const tr = edit(state, rnd);
      state = state.apply(tr);
      const where = `seed ${seed}, edit ${n}`;
      assert.deepEqual(plain(autolink.getState(state), state.doc), plain(linksIn(state.doc), state.doc), `links, ${where}`);
      assert.deepEqual(plain(numbered.getState(state), state.doc), plain(numbering(state.doc), state.doc), `numbers, ${where}`);
      assert.deepEqual(plain(found.getState(state).marks, state.doc), plain(foundMarks(state.doc, WORDS_SEARCHED), state.doc), `search, ${where}`);
    }
  }
});

test("a key typed in a long note reads the line typed in, not the note", () => {
  // About 50,000 characters, with addresses, and a long numbered list in the middle.
  const line = "The quick brown fox jumps over www.example.org, again, for a long note.";
  const lines = [];
  for (let i = 0; i < 700; i++) {
    const listed = i >= 300 && i < 400;
    lines.push(schema.nodes.paragraph.create({ style: listed ? STYLE.numbered : null, indent: listed && i % 5 === 1 ? 1 : 0 }, schema.text(line)));
  }
  const doc = schema.nodes.doc.create(null, lines);
  let state = EditorState.create({ doc, plugins: [autolink, numbered, found] });
  state = state.apply(state.tr.setMeta(found, ["note"]));
  const chars = doc.textContent.length;
  assert.ok(chars > 49000, `the note is ${chars} characters`);

  const typeAt = (pos, text) => {
    const per = [];
    let full = { textNodes: 0, chars: 0, lines: 0 };
    for (const c of text) {
      Object.assign(scanned, { textNodes: 0, chars: 0, lines: 0 });
      state = state.apply(state.tr.insertText(c, pos));
      per.push({ ...scanned });
      pos += 1;
      // What drawing everything again — as each key did before — reads.
      Object.assign(scanned, { textNodes: 0, chars: 0, lines: 0 });
      linksIn(state.doc);
      numbering(state.doc);
      foundMarks(state.doc, ["note"]);
      full = { ...scanned };
    }
    const most = (k) => Math.max(...per.map((p) => p[k]));
    return { chars: most("chars"), textNodes: most("textNodes"), lines: most("lines"), full };
  };

  const end = typeAt(state.doc.content.size - 1, " and the end");
  // In the numbered list: the start of item 50 of 100.
  let inList = 0;
  for (let i = 0; i < 350; i++) inList += state.doc.child(i).nodeSize;
  const list = typeAt(inList + 1, "abc");
  console.log(`per key, at the end: read ${end.chars} characters in ${end.textNodes} text nodes, counted ${end.lines} lines`);
  console.log(`per key, in a list: read ${list.chars} characters in ${list.textNodes} text nodes, counted ${list.lines} lines`);
  console.log(`per key, drawn whole: read ${end.full.chars} characters in ${end.full.textNodes} text nodes, counted ${end.full.lines} lines`);
  // Two drawings (links, search), each over the line typed in and the one either side.
  assert.ok(end.chars <= 2 * 3 * (line.length + 12), `typing at the end read ${end.chars} characters`);
  assert.ok(list.chars <= 2 * 3 * (line.length + 3), `typing in a list read ${list.chars} characters`);
  assert.ok(end.lines <= 4 && list.lines <= 10, `numbers counted ${end.lines} and ${list.lines} lines`);
  assert.ok(end.full.chars > 2 * 49000);
});
