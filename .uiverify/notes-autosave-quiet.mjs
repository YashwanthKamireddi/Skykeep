// A pause in typing saves the note without anything else happening around it: the controls
// stay usable, the list is not read again for it, and the note's row says what it now says.
// A save that fails keeps the edit, and it goes with the next one.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
// Each command the window sends, in order; a save held a moment, as the library's write is.
await ctx.addInitScript(() => {
  const wrap = () => {
    const t = window.__TAURI_INTERNALS__;
    if (!t || t.__logged) return;
    const inner = t.invoke;
    t.invoke = async (cmd, args) => {
      (window.__calls ??= []).push(cmd);
      if (cmd === "note_save") {
        await new Promise((r) => setTimeout(r, 300));
        if (window.__saveFails) { window.__saveFails -= 1; throw "The library couldn’t be written."; }
      }
      return inner(cmd, args);
    };
    t.__logged = true;
  };
  wrap(); document.addEventListener("DOMContentLoaded", wrap);
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

await page.locator(".note-row", { hasText: "Curtains" }).first().click();
await page.waitForTimeout(400);
await page.locator(".note-rich p").first().click();
await page.keyboard.press("End");
await page.evaluate(() => {
  window.__calls = [];
  window.__disabled = 0;
  // Watched every frame through the save: New Note and Pin never greyed out by it.
  const look = () => {
    const off = [...document.querySelectorAll('.notes button[aria-label="New note"], .notes button[aria-label="Pin Note"], .notes button[aria-label="Unpin Note"]')]
      .some((x) => x.disabled);
    if (off) window.__disabled += 1;
    if (!window.__stopLook) requestAnimationFrame(look);
  };
  requestAnimationFrame(look);
});
await page.keyboard.type(" and blinds");
// Through the pause and the save.
await page.waitForTimeout(1700);
const calls = await page.evaluate(() => window.__calls);
ok(calls.filter((c) => c === "note_save").length === 1, `one save  ${JSON.stringify(calls)}`);
const after = calls.slice(calls.indexOf("note_save") + 1);
ok(!after.some((c) => ["notes_in", "note_folders", "note_counts", "note_body", "note_runs"].includes(c)),
  `nothing is read again for it  ${JSON.stringify(after)}`);
ok((await page.evaluate(() => window.__disabled)) === 0, "New Note and Pin stay usable while it saves");
ok(/Saved|Edited|Today|\d/.test(await page.locator(".note-when").innerText()), `the bar says it is saved  "${await page.locator(".note-when").innerText()}"`);
// The first line was typed in: the row's title follows it, without the list read again.
ok((await page.locator(".note-row", { hasText: "Curtains and blinds" }).count()) === 1,
  `the row's title follows the first line  ${JSON.stringify(await page.locator(".note-row-name").allInnerTexts())}`);

// And again.
await page.locator(".note-rich p").first().click();
await page.keyboard.press("End");
await page.keyboard.type(" Shopping");
await page.waitForTimeout(1700);
ok((await page.locator(".note-row", { hasText: "Curtains and blinds Shopping" }).count()) === 1,
  `and again  ${JSON.stringify(await page.locator(".note-row-name").allInnerTexts())}`);

// A save that fails keeps the edit, which goes with the next.
await page.evaluate(() => { window.__saveFails = 1; window.__noteSaves = []; });
await page.keyboard.type(" X");
await page.waitForTimeout(1700);
await page.keyboard.type("Y");
await page.waitForTimeout(1700);
const last = await page.evaluate(() => (window.__noteSaves ?? []).at(-1)?.body ?? "");
ok(last.startsWith("Curtains and blinds Shopping XY"), `a failed save's edit goes with the next one  ${JSON.stringify(last.split("\n")[0])}`);
await page.evaluate(() => { window.__stopLook = true; });
await b.close();
process.exitCode = failures ? 1 : 0;
