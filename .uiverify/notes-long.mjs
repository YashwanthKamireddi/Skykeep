import { openScreen } from "./stub.mjs";
// A very long note types as a short one does, and saves exactly what was typed.
//
// A keystroke must change the line typed in and nothing else: an editor that redrew the note
// — React rewriting a controlled field, or the whole document set again — made each key a
// relayout of 300,000 characters. Watched directly: no line of the note but the one typed
// in is added, removed or replaced while typing, and each key is handled well inside a frame.
import { engine } from "./engine.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 820 } });
await ctx.addInitScript(() => {
  const line = "The quick brown fox jumps over the lazy dog, again and again, for a long note.\n";
  const body = "Long note\n" + line.repeat(3800);
  window.__bodyLength = body.length;
  window.__saves = [];
  const note = { guid: "L", folderGuid: null, title: "Long note", snippet: "The quick", body: null, locked: false,
    pinned: false, createdAt: 1, modifiedAt: 1788500000, editable: true, etag: "e" };
  window.__TAURI_INTERNALS__ = {
    transformCallback: (cb) => cb,
    invoke: async (cmd, args) => {
      if (cmd === "auth_status") return { hasStoredSession: true, signedIn: true };
      if (cmd === "auth_resume") return "library";
      if (cmd === "account_services") return ["notes", "ckdatabasews"];
      if (cmd === "notes_in") return [{ ...note }];
      if (cmd === "note_counts") return [1, []];
      if (cmd === "note_body") return body;
      if (cmd === "note_save") { window.__saves.push(args); return "L"; }
      if (cmd === "sync_all" || cmd === "sync_status") return { services: [], asOf: 1, tookMs: 0 };
      if (cmd === "plugin:event|listen") return 0;
      return [];
    },
  };
});
const page = await ctx.newPage();
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.waitForTimeout(600);

let failures = 0;
const ok = (pass, yes, no) => { console.log(pass ? `PASS ${yes}` : `FAIL ${no}`); if (!pass) failures++; };

await page.locator(".note-rich .nf-p").last().click();
await page.keyboard.press("Control+End");
await page.waitForTimeout(50);
await page.evaluate(() => {
  window.__rewrites = 0;
  new MutationObserver((m) => { window.__rewrites += m.length; })
    .observe(document.querySelector(".note-rich"), { childList: true });
  // How long each key takes to be handled, start to the next frame.
  window.__keyMs = [];
  document.addEventListener("keydown", () => {
    const t = performance.now();
    requestAnimationFrame(() => window.__keyMs.push(performance.now() - t));
  }, true);
});
await page.keyboard.type(" and the end", { delay: 20 });
await page.waitForTimeout(100);
const rewrites = await page.evaluate(() => window.__rewrites);
ok(rewrites === 0, "typing in a long note touches only the line typed in",
  `lines of the note were replaced ${rewrites} times while typing twelve characters`);
const slowest = await page.evaluate(() => Math.max(...window.__keyMs));
ok(slowest < 50, `and each key is drawn quickly  (slowest ${slowest.toFixed(1)} ms)`, `a key took ${slowest.toFixed(1)} ms to draw`);

// Leaving the note saves it: every character there, and the new ones at the end.
await page.locator(".notes-search").click();
await page.waitForTimeout(600);
const saved = await page.evaluate(() => {
  const last = window.__saves.at(-1);
  return last && { length: last.body?.length, tail: last.body?.slice(-24), expected: window.__bodyLength + 12 };
});
ok(saved && saved.length === saved.expected && saved.tail.endsWith("note.\n and the end"),
  "and what is saved is the whole note with what was typed", `saved ${JSON.stringify(saved)}`);

await b.close();
process.exitCode = failures === 0 ? 0 : 1;
