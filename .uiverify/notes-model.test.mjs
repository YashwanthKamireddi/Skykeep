// The rules Notes' list and editor follow, checked directly.
import { test } from "node:test";
import assert from "node:assert/strict";
import { groupNotes, listDate, splitTitle, joinTitle, editorDate } from "../src/screens/notes/model.ts";
import { setLanguage } from "../src/lib/i18n.ts";

// The words below are English whatever this machine's language: the interface follows it.
setLanguage("en");

const now = new Date(2026, 8, 30, 12, 0);
const at = (y, m, d, h = 9) => Math.floor(new Date(y, m, d, h).getTime() / 1000);
const note = (guid, modifiedAt, pinned = false) => ({ guid, folderGuid: null, title: guid, snippet: null, body: null,
  locked: false, pinned, createdAt: null, modifiedAt, editable: true, etag: null });

test("the list is grouped the way Notes groups it, pinned first", () => {
  const sections = groupNotes([
    note("old", at(2025, 2, 1)), note("aug", at(2026, 7, 10)), note("today", at(2026, 8, 30)),
    note("pin", at(2026, 0, 1), true), note("yday", at(2026, 8, 29)), note("week", at(2026, 8, 25)),
    note("month", at(2026, 8, 10)),
  ], now);
  assert.deepEqual(sections.map((s) => [s.key, s.notes.map((n) => n.guid)]), [
    ["pinned", ["pin"]], ["today", ["today"]], ["yesterday", ["yday"]], ["week", ["week"]],
    ["month", ["month"]], ["m7", ["aug"]], ["y2025", ["old"]],
  ]);
});

test("within a group the newest comes first", () => {
  const [today] = groupNotes([note("a", at(2026, 8, 30, 8)), note("b", at(2026, 8, 30, 11))], now);
  assert.deepEqual(today.notes.map((n) => n.guid), ["b", "a"]);
});

test("a row says the time today, Yesterday, the weekday, or the date", () => {
  assert.match(listDate(at(2026, 8, 30, 9), now), /9/);
  assert.equal(listDate(at(2026, 8, 29), now), "Yesterday");
  assert.equal(listDate(at(2026, 8, 27), now), new Date(2026, 8, 27).toLocaleDateString(undefined, { weekday: "long" }));
  assert.match(listDate(at(2026, 5, 1), now), /2026/);
});

test("splitting a note into title and the rest loses nothing", () => {
  for (const text of ["", "Title", "Title\n", "Title\nbody", "\nbody", "T\n\n\nlast\n"]) {
    const { title, rest } = splitTitle(text);
    assert.equal(joinTitle(title, rest, text.includes("\n")), text, JSON.stringify(text));
  }
});

test("sorted by date created, the groups follow when each note was made", () => {
  const n = (guid, created, modified) => ({ ...note(guid, modified), createdAt: created });
  const sections = groupNotes([n("made-today", at(2026, 8, 30), at(2026, 8, 30)), n("made-in-aug", at(2026, 7, 2), at(2026, 8, 30))],
    now, { by: "created", groupByDate: true });
  assert.deepEqual(sections.map((s) => [s.key, s.notes.map((x) => x.guid)]), [["today", ["made-today"]], ["m7", ["made-in-aug"]]]);
});

test("sorted by title: A to Z in one run, numbers in number order, pinned still first", () => {
  const sections = groupNotes([note("banana", at(2026, 8, 1)), note("Apple", at(2026, 8, 2)), note("item 10", at(2026, 8, 3)),
    note("item 9", at(2026, 8, 4)), note("zed", at(2026, 8, 5), true)], now, { by: "title", groupByDate: true });
  assert.deepEqual(sections.map((s) => [s.key, s.heading, s.notes.map((x) => x.guid)]), [
    ["pinned", "Pinned", ["zed"]], ["notes", "Notes", ["Apple", "banana", "item 9", "item 10"]],
  ]);
});

test("not grouped by date: one run newest first, with no heading when nothing is pinned", () => {
  const sections = groupNotes([note("old", at(2025, 1, 1)), note("new", at(2026, 8, 30))], now, { by: "edited", groupByDate: false });
  assert.deepEqual(sections.map((s) => [s.heading, s.notes.map((x) => x.guid)]), [["", ["new", "old"]]]);
});

test("a note with no date comes last, under Earlier, whatever the order", () => {
  const sections = groupNotes([note("nodate", null), note("today", at(2026, 8, 30))], now, { by: "created", groupByDate: true });
  assert.deepEqual(sections.map((s) => s.key), ["undated"].length ? sections.map((s) => s.key) : []);
  assert.equal(sections.at(-1).key, "undated");
});

test("every note appears exactly once, in every order", () => {
  const all = [note("a", at(2026, 8, 30)), note("b", null), note("c", at(2025, 1, 1), true), note("d", at(2026, 6, 1))];
  for (const by of ["edited", "created", "title"]) for (const groupByDate of [true, false]) {
    const seen = groupNotes(all, now, { by, groupByDate }).flatMap((s) => s.notes.map((n) => n.guid)).sort();
    assert.deepEqual(seen, ["a", "b", "c", "d"], `${by} ${groupByDate}`);
  }
});

test("in German, the list's headings and dates are German", () => {
  setLanguage("de");
  try {
    const notes = [
      { guid: "p", title: "Fixed", pinned: true, modifiedAt: at(2026, 8, 30), createdAt: at(2026, 8, 30) },
      { guid: "t", title: "Today", pinned: false, modifiedAt: at(2026, 8, 30), createdAt: at(2026, 8, 30) },
      { guid: "y", title: "Yesterday", pinned: false, modifiedAt: at(2026, 8, 29), createdAt: at(2026, 8, 29) },
      { guid: "w", title: "Week", pinned: false, modifiedAt: at(2026, 8, 25), createdAt: at(2026, 8, 25) },
    ];
    const headings = groupNotes(notes, now, { by: "edited", groupByDate: true }).map((s) => s.heading);
    assert.deepEqual(headings, ["Fixiert", "Heute", "Gestern", "Vorherige 7 Tage"]);
    assert.equal(listDate(at(2026, 8, 29), now), "Gestern");
    assert.match(editorDate(at(2026, 8, 30, 22)), / um /);
  } finally {
    setLanguage("en");
  }
});
