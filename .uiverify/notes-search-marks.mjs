// Notes' search as on a Mac: each result previews the line its words are in, the words marked
// in yellow there and in its title, and the note opened from it marks them too — gone once the
// search is left. Each row is one element. Shift+Ctrl+N makes a folder; Shift+Ctrl+U ticks the
// checklist item the cursor is in.
//   BASE=http://localhost:4173 ENGINE=webkit node .uiverify/notes-search-marks.mjs
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const BASE = process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (pass, yes, no) => { console.log(pass ? `PASS ${yes}` : `FAIL ${no}`); if (!pass) failures++; };
const until = (page, fn, arg) => page.waitForFunction(fn, arg, { timeout: 8000, polling: "raf" }).then(() => true, () => false);

const ctx = await b.newContext({ viewport: { width: 1280, height: 820 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
  calendarFixture: false, notesFixture: true, contactsFixture: false, driveFixture: false, remindersFixture: false });
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, "", `page error: ${e.message}`));
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.locator(".note-row").first().waitFor();

// One element a row: the option itself sits in its section's group, with no box around it.
const shape = await page.evaluate(() => [...document.querySelectorAll(".note-row")].map((r) => r.parentElement?.getAttribute("role")));
ok(shape.length > 0 && shape.every((r) => r === "group"), "each note row is a single element in its group", `row parents: ${shape}`);

// Searched for a word deep in a note: the row shows that line, the word marked.
await page.keyboard.press("Control+Shift+f");
await page.keyboard.type("clock");
ok(await until(page, () => document.querySelectorAll(".note-row").length === 1), "the search finds the one note", "search did not narrow");
const snippet = await page.locator(".note-row .note-row-snippet").first().innerText();
ok(snippet.includes("wall clock"), "its preview is the line the word is in", `preview: ${snippet}`);
ok(await page.locator(".note-row .note-row-snippet mark.note-found").first().innerText() === "clock", "the word is marked in the preview", "no mark in the preview");

// Opened from the result, the note marks it too.
await page.locator(".note-row").first().click();
ok(await until(page, () => [...document.querySelectorAll(".note-rich .note-found")].some((m) => m.textContent === "clock")),
  "the note opened from it marks the word", "no mark in the note");
// Marks are drawn, never written: nothing was saved for them.
ok(await page.evaluate(() => (window.__noteSaves ?? []).length) === 0, "and marking it saves nothing", "a save went out for a mark");

// A word that begins a title is marked there, from where it starts, as the index matches it.
await page.getByLabel("Search notes").fill("curt");
ok(await until(page, () => document.querySelector(".note-row .note-row-name mark.note-found")?.textContent === "Curt"),
  "a title's matching start is marked", "no mark in the title");

// Leaving the search takes the marks out of the note.
await page.getByLabel("Search notes").fill("");
ok(await until(page, () => document.querySelector(".note-rich") && !document.querySelector(".note-rich .note-found")),
  "leaving the search clears the marks from the note", "marks stayed after the search");

// Shift+Ctrl+U on a checklist item ticks it — or unticks one already ticked.
await page.locator(".note-row", { hasText: "Curtains" }).click();
await until(page, () => document.querySelector('.note-rich p[data-style="103"]'));
const ticked = () => page.evaluate(() => document.querySelector('.note-rich p[data-style="103"]')?.getAttribute("data-checked") === "true");
// The caret put in the item's text first (at its end, clear of the tick circle), and only then
// is its state read: a click that strayed onto the circle ticked it and read as the key failing.
await page.locator('.note-rich p[data-style="103"]').first().click({ position: { x: 120, y: 8 } });
await page.keyboard.press("End");
// Two frames for the caret's move to settle: the shortcut ten milliseconds after End — faster
// than anyone types — was handled before the editor had taken in where End put the caret.
await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
const was = await ticked();
await page.keyboard.press("Control+Shift+u");
ok(await until(page, (w) => (document.querySelector('.note-rich p[data-style="103"]')?.getAttribute("data-checked") === "true") !== w, was),
  "Shift+Ctrl+U ticks or unticks the checklist item", "the item stayed as it was");
await page.keyboard.press("Control+Shift+u");
ok(await until(page, (w) => (document.querySelector('.note-rich p[data-style="103"]')?.getAttribute("data-checked") === "true") === w, was),
  "and again puts it back", "a second press did not put it back");

// Shift+Ctrl+N: a new folder, ready to be named.
await page.locator(".note-row", { hasText: "Curtains" }).focus();
await page.keyboard.press("Control+Shift+n");
ok(await until(page, () => (window.__folderCalls ?? []).some((c) => c[0] === "create")), "Shift+Ctrl+N makes a new folder", "no folder was made");
ok(await until(page, () => document.activeElement?.classList.contains("notes-folder-rename")), "and its name is ready to type", "the name field did not take focus");

await ctx.close();
await b.close();
console.log(failures ? `\n${failures} FAILED` : "\nall passed");
process.exit(failures ? 1 : 0);
