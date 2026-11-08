// Notes' keys, as on a Mac (Ctrl here): ↑/↓ through the list, Ctrl+N a new note, Ctrl+F this
// folder's search and Ctrl+Shift+F every folder's, Esc out of search, Ctrl+Backspace delete
// (asked first), and the editor's Ctrl+B/I/U and Ctrl+Shift+L. Adding a file stays hidden while
// the backend's switch is off.
//   BASE=http://localhost:4173 ENGINE=webkit node .uiverify/notes-keys.mjs
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const BASE = process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (pass, yes, no) => { console.log(pass ? `PASS ${yes}` : `FAIL ${no}`); if (!pass) failures++; };

async function open(setup = () => {}) {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 820 } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
    calendarFixture: false, notesFixture: true, contactsFixture: false, driveFixture: false, remindersFixture: false });
  await ctx.addInitScript(setup);
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, "", `page error: ${e.message}`));
  await page.goto(BASE, { waitUntil: "networkidle" });
  await openScreen(page, "Notes");
  await page.locator(".note-row").first().waitFor();
  return { ctx, page };
}
/** Until `fn` holds in the page (polled each frame), or 8 s; the check after it says what is there. */
const until = (page, fn, arg) => page.waitForFunction(fn, arg, { timeout: 8000, polling: "raf" }).then(() => true, () => false);
const current = (page) => page.evaluate(() => document.querySelector('.note-row[aria-selected="true"] .note-row-name')?.textContent ?? null);
const focusIn = (page, sel) => page.evaluate((s) => Boolean(document.activeElement?.closest(s)), sel);

{
  const { ctx, page } = await open();
  // The list as shown: the pinned note, then Curtains, then the last.
  await page.locator(".note-row", { hasText: "Curtains" }).click();
  await until(page, () => document.querySelector('.note-row[aria-selected="true"] .note-row-name')?.textContent?.includes("Curtains"));
  await page.keyboard.press("ArrowDown");
  await until(page, () => !document.querySelector('.note-row[aria-selected="true"] .note-row-name')?.textContent?.includes("Curtains"));
  const down = await current(page);
  ok(down?.startsWith("These sources"), "↓ moves to the next note", `↓ landed on ${down}`);
  ok(await focusIn(page, ".note-rows"), "and the list keeps the focus", "focus left the list");
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("ArrowUp");
  await until(page, () => !!document.querySelector('.note-row[aria-selected="true"] .note-row-name')?.textContent?.includes("UNIT 1"));
  const up = await current(page);
  ok(up?.includes("UNIT 1"), "↑ moves back up, pinned notes first", `↑↑ landed on ${up}`);
  await page.keyboard.press("ArrowUp");
  await page.waitForTimeout(150);
  ok((await current(page))?.includes("UNIT 1"), "and stops at the top", "↑ ran past the top");

  // Ctrl+F in a folder: that folder's search; Esc back to the list.
  await page.locator(".notes-folder", { hasText: /^Notes/ }).click();
  await until(page, () => [...document.querySelectorAll(".note-row")].some((r) => r.textContent.includes("Curtains")));
  await page.locator(".note-row", { hasText: "Curtains" }).click();
  await until(page, () => document.querySelector('.note-row[aria-selected="true"] .note-row-name')?.textContent?.includes("Curtains"));
  await page.keyboard.press("Control+f");
  ok(await focusIn(page, ".notes-search"), "Ctrl+F goes to the search field", "Ctrl+F went nowhere");
  ok(await page.getByLabel("Search notes").getAttribute("placeholder") === "Search Notes", "and searches this folder",
    `placeholder ${await page.getByLabel("Search notes").getAttribute("placeholder")}`);
  await page.keyboard.type("curt");
  await until(page, () => document.querySelector('.notes-scope .segment[aria-pressed="true"]')?.textContent === "Notes" && document.querySelectorAll(".note-row").length === 1);
  ok(await page.locator('.notes-scope .segment[aria-pressed="true"]').textContent() === "Notes",
    "the scope shows the folder", "scope not shown");
  ok(await page.locator(".note-row").count() === 1, "results are the folder's matches", `${await page.locator(".note-row").count()} rows`);
  await page.keyboard.press("Escape");
  await until(page, () => document.querySelector('[aria-label="Search notes"]')?.value === "" && !!document.activeElement?.closest(".note-rows"));
  ok(await page.getByLabel("Search notes").inputValue() === "" && await focusIn(page, ".note-rows"),
    "Esc clears the search and returns to the list", "Esc left the search as it was");
  await page.keyboard.press("Control+Shift+F");
  await page.keyboard.type("curt");
  await until(page, () => document.querySelector('.notes-scope .segment[aria-pressed="true"]')?.textContent === "All iCloud");
  ok(await page.locator('.notes-scope .segment[aria-pressed="true"]').textContent() === "All iCloud",
    "Ctrl+Shift+F searches every folder", "Ctrl+Shift+F kept the folder scope");
  await page.keyboard.press("Escape");
  await until(page, () => document.querySelector('[aria-label="Search notes"]')?.value === "");

  // Ctrl+Backspace asks first; No keeps the note.
  await page.locator(".note-row", { hasText: "Curtains" }).click();
  await until(page, () => document.querySelector('.note-row[aria-selected="true"] .note-row-name')?.textContent?.includes("Curtains"));
  await page.keyboard.press("Control+Backspace");
  await until(page, () => !!document.querySelector(".confirm h2"));
  const asked = await page.locator(".confirm h2").textContent().catch(() => null);
  ok(asked === "Delete “Curtains”?", "Ctrl+Backspace asks before deleting", `asked ${asked}`);
  await page.keyboard.press("Escape");
  await until(page, () => !document.querySelector(".confirm"));
  ok(await page.evaluate(() => (window.__noteDeletes ?? []).length) === 0 && await page.locator(".note-row", { hasText: "Curtains" }).count() === 1,
    "and keeping it keeps it", "the note went anyway");
  await page.locator(".note-row", { hasText: "Curtains" }).click();
  await page.keyboard.press("Control+Backspace");
  await until(page, () => !!document.querySelector(".confirm .confirm-answer"));
  await page.locator(".confirm .confirm-answer").click();
  await until(page, () => (window.__noteDeletes ?? []).length > 0);
  ok(JSON.stringify(await page.evaluate(() => window.__noteDeletes)) === '["n1"]', "Delete deletes it", "nothing was deleted");

  // No paperclip while the backend's switch is off.
  ok(await page.getByRole("button", { name: "Add a picture or file" }).count() === 0, "adding a file stays hidden while it is off",
    "the paperclip shows while the backend is off");
  await ctx.close();
}

{
  const { ctx, page } = await open();
  // Ctrl+N from anywhere: a new note, cursor in it.
  await page.locator(".note-row", { hasText: "Curtains" }).click();
  await page.keyboard.press("Control+n");
  await until(page, () => !!document.activeElement?.closest(".note-rich"));
  ok(await focusIn(page, ".note-rich"), "Ctrl+N starts a new note with the cursor in it", "Ctrl+N did nothing");
  await page.keyboard.type("Plan\nbuy milk");
  await page.keyboard.press("Control+Shift+L");
  await until(page, () => !!document.querySelector('.note-rich .nf-p[data-style="103"]')?.textContent?.includes("buy milk"));
  ok(await page.locator('.note-rich .nf-p[data-style="103"]', { hasText: "buy milk" }).count() === 1,
    "Ctrl+Shift+L makes the line a checklist item", "no checklist");
  // The line chosen whole before it is marked — and the editor told of it: Shift+Home is the
  // browser's, and the editor takes the new selection from its selectionchange, a moment on.
  // A key before that marks the caret, not the line.
  await page.evaluate(() => { window.__selChanged = false; document.addEventListener("selectionchange", () => { window.__selChanged = true; }, { once: true }); });
  await page.keyboard.press("Shift+Home");
  await until(page, () => String(getSelection()) === "buy milk" && window.__selChanged);
  await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => r())));
  await page.keyboard.press("Control+b");
  await page.keyboard.press("Control+i");
  await page.keyboard.press("Control+u");
  await until(page, () => {
    const p = [...document.querySelectorAll(".note-rich .nf-p")].find((x) => x.textContent === "buy milk");
    return ["strong", "em", "u"].every((t) => p?.querySelector(t)?.textContent === "buy milk");
  });
  const marks = await page.evaluate(() => {
    const p = [...document.querySelectorAll(".note-rich .nf-p")].find((x) => x.textContent === "buy milk");
    return ["strong", "em", "u"].filter((t) => p?.querySelector(t)?.textContent === "buy milk");
  });
  ok(marks.length === 3, "Ctrl+B, Ctrl+I and Ctrl+U set bold, italic and underline", `marks ${marks}`);
  // In the editor Ctrl+Backspace is the editor's (a word back), never a delete.
  await page.keyboard.press("End");
  await page.keyboard.press("Control+Backspace");
  await page.waitForTimeout(250);
  ok(await page.locator(".confirm").count() === 0, "Ctrl+Backspace while typing stays the editor's", "it asked to delete the note");
  await ctx.close();
}

{
  // The backend's switch on: the paperclip appears, and adds at the cursor.
  const { ctx, page } = await open(() => { window.__attachOn = true; window.__pick = "/home/test/Pictures/beach.jpg"; });
  await page.locator(".note-row", { hasText: "Curtains" }).click();
  await until(page, () => !!document.querySelector('[aria-label="Add a picture or file"]'));
  const clip = page.getByRole("button", { name: "Add a picture or file" });
  ok(await clip.count() === 1, "with the switch on, the paperclip shows", "no paperclip with the switch on");
  await page.locator(".note-rich .nf-p", { hasText: "Fan" }).click();
  await page.keyboard.press("End");
  await clip.click();
  await until(page, () => (window.__attaches ?? []).length > 0);
  const sent = await page.evaluate(() => window.__attaches ?? []);
  const base = sent[0]?.base ?? "";
  ok(sent.length === 1 && sent[0].guid === "n1" && sent[0].path === "/home/test/Pictures/beach.jpg"
    && base.slice(0, sent[0].atOffset).endsWith("Fan"), "it adds the file where the cursor is", `sent ${JSON.stringify(sent)}`);
  await ctx.close();
}

await b.close();
process.exitCode = failures === 0 ? 0 : 1;
