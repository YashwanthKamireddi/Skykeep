// Moving through notes and typing in one draw only what changed.
//
// Stepping from note to note with the arrow keys: no "Opening…" between them and no plain note
// drawn first and formatted after — the text and its formatting arrive together, and the notes
// beside the open one are read ahead. Typing, and moving the cursor, draw the note and its bar:
// never every row of the list again. Counted from React's own commits (the DevTools hook).
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
// React tells a DevTools hook of every commit; this one counts the note rows each commit drew.
await ctx.addInitScript(() => {
  window.__rowRenders = 0;
  window.__commits = 0;
  window.__REACT_DEVTOOLS_GLOBAL_HOOK__ = {
    supportsFiber: true,
    renderers: new Map(),
    inject(renderer) { this.renderers.set(1, renderer); return 1; },
    checkDCE() {},
    onScheduleFiberRoot() {},
    onPostCommitFiberRoot() {},
    onCommitFiberUnmount() {},
    onCommitFiberRoot(_id, root) {
      window.__commits += 1;
      // A row drawn in this commit has props it has never had before: a row left alone keeps
      // the very props object of the last time it was drawn.
      const seen = (window.__seenProps ??= new WeakSet());
      const walk = (f) => {
        for (let n = f; n; n = n.sibling) {
          if (n.type === "button" && n.memoizedProps?.className === "note-row" && !seen.has(n.memoizedProps)) {
            seen.add(n.memoizedProps);
            window.__rowRenders += 1;
          }
          if (n.child) walk(n.child);
        }
      };
      walk(root.current.child);
    },
  };
});
// Every read of the library takes a moment, as a real one does.
await ctx.addInitScript(() => {
  const wrap = () => {
    const t = window.__TAURI_INTERNALS__;
    if (!t || t.__slowReads) return;
    const inner = t.invoke;
    t.invoke = async (cmd, args) => {
      if (cmd === "note_body" || cmd === "note_runs") {
        (window.__reads ??= []).push(cmd);
        await new Promise((r) => setTimeout(r, cmd === "note_runs" ? 70 : 30));
      }
      return inner(cmd, args);
    };
    t.__slowReads = true;
  };
  wrap(); document.addEventListener("DOMContentLoaded", wrap);
});
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.waitForTimeout(500);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

// Everything drawn in the note pane while stepping, frame by frame.
await page.evaluate(() => {
  window.__seen = { opening: 0, readonly: 0, plainCurtains: 0 };
  const look = () => {
    const pane = document.querySelector(".notes .note-body");
    if (pane?.querySelector(".note-loading")) window.__seen.opening += 1;
    if (pane?.querySelector(".note-editor.is-readonly")) window.__seen.readonly += 1;
    // Curtains is set as a phone sets it: a title line. Drawn without it is the plain flash.
    const rich = pane?.querySelector(".note-rich");
    if (rich && /Curtains/.test(rich.textContent ?? "") && !rich.querySelector('[data-style="0"]')) window.__seen.plainCurtains += 1;
  };
  new MutationObserver(look).observe(document.querySelector(".notes .note-body"), { childList: true, subtree: true });
});
const rows = await page.locator(".note-row").count();
await page.locator(".note-row").first().click();
await page.waitForTimeout(400);
const stepped = [];
for (let i = 0; i < Math.min(rows - 1, 4); i++) {
  await page.keyboard.press("ArrowDown");
  // As someone reading down a list: a glance at each.
  await page.waitForTimeout(260);
  stepped.push(await page.locator('.note-row[aria-selected="true"] .note-row-name').innerText());
}
for (let i = 0; i < 2; i++) {
  await page.keyboard.press("ArrowUp");
  await page.waitForTimeout(260);
}
const seen = await page.evaluate(() => window.__seen);
ok(seen.opening === 0, `stepping through notes never shows "Opening…" between them  ${JSON.stringify(seen)} ${JSON.stringify(stepped)}`);
ok(seen.readonly === 0, "nor a note drawn read-only for a moment while its formatting comes");
ok(seen.plainCurtains === 0, "nor a formatted note drawn plain first");

// A note slow to come still says it is opening, once it is slow.
await page.evaluate(() => { window.__slowBody = true; });
await page.evaluate(() => {
  const t = window.__TAURI_INTERNALS__;
  const inner = t.invoke;
  t.invoke = async (cmd, args) => {
    if (window.__slowBody && cmd === "note_body") await new Promise((r) => setTimeout(r, 900));
    return inner(cmd, args);
  };
});
// The library changed (an edit sent): what was read ahead is let go of, so the last note —
// read ahead while stepping past it — has to be read again.
await page.evaluate(() => window.__emit("quince://outbox-changed", { services: ["notes"] }));
await page.waitForTimeout(100);
await page.locator(".note-row").last().click();
await page.waitForTimeout(100);
const early = await page.locator(".note-loading").count();
await page.waitForTimeout(400);
const later = await page.locator(".note-loading").count();
ok(early === 0 && later === 1, `a slow note says "Opening…" only once it is slow  (at 100 ms: ${early}, at 500 ms: ${later})`);
await page.evaluate(() => { window.__slowBody = false; });
await page.waitForTimeout(800);

// Typing draws the note, not the list.
await page.locator(".note-row", { hasText: "Curtains" }).first().click();
await page.waitForTimeout(500);
await page.locator(".note-rich p").last().click();
await page.keyboard.press("End");
await page.waitForTimeout(200);
const before = await page.evaluate(() => ({ rows: window.__rowRenders, commits: window.__commits }));
await page.keyboard.type(" and blinds", { delay: 30 });
await page.waitForTimeout(300);
const typed = await page.evaluate(() => ({ rows: window.__rowRenders, commits: window.__commits }));
ok(typed.rows - before.rows === 0, `typing eleven characters draws no row of the list again  (${typed.rows - before.rows} rows drawn)`);

// Moving the cursor within a line changes nothing on the bar: nothing is drawn for it.
await page.keyboard.press("Home");
await page.waitForTimeout(100);
const moved0 = await page.evaluate(() => window.__commits);
for (let i = 0; i < 6; i++) await page.keyboard.press("ArrowRight");
await page.waitForTimeout(150);
const moved = await page.evaluate(() => window.__commits) - moved0;
ok(moved === 0, `moving the cursor along a line commits nothing  (${moved} commits)`);

// And what was typed is saved, once, as typed.
await page.waitForTimeout(1300);
const saves = await page.evaluate(() => (window.__noteSaves ?? []).map((s) => s.body));
ok(saves.length === 1 && saves[0].includes("and blinds"), `what was typed is saved once  ${JSON.stringify(saves)}`);
await b.close();
process.exitCode = failures ? 1 : 0;
