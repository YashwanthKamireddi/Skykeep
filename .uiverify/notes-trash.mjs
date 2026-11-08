import { openScreen } from "./stub.mjs";
// Recently Deleted behaves as it does on the phone: a deleted note can be recovered, and the
// only final delete is the one asked about first.
import { engine as chromium } from "./engine.mjs";

const b = await chromium.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 820 } });
await ctx.addInitScript(() => {
  window.__deletes = 0;
  window.__moves = [];
  const trashed = { guid: "T1", folderGuid: "TrashFolder-CloudKit", title: "Old list", snippet: "eggs",
    body: null, locked: false, pinned: false, createdAt: 1, modifiedAt: 1, editable: true, etag: "e" };
  window.__TAURI_INTERNALS__ = {
    transformCallback: (cb) => cb,
    invoke: async (cmd, args) => {
      if (cmd === "auth_status") return { hasStoredSession: true, signedIn: true };
      if (cmd === "auth_resume") return "library";
      if (cmd === "account_services") return ["notes", "ckdatabasews"];
      if (cmd === "note_folders")
        return [{ guid: "TrashFolder-CloudKit", title: "Recently Deleted", sortOrder: 9, parentGuid: null }];
      if (cmd === "notes_in") return args.folderGuid === "TrashFolder-CloudKit" ? [trashed] : [];
      if (cmd === "note_body") return "Old list\neggs";
      if (cmd === "note_delete") { window.__deletes++; return null; }
      if (cmd === "note_move") { window.__moves.push(args); return null; }
      if (cmd === "sync_all" || cmd === "sync_status") return { services: [], asOf: 1, tookMs: 0 };
      if (cmd === "plugin:event|listen") return 0;
      return [];
    },
  };
});
const page = await ctx.newPage();
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.getByRole("button", { name: "Recently Deleted" }).click();
await page.waitForTimeout(500);

let failures = 0;
const ok = (pass, yes, no) => { console.log(pass ? `PASS ${yes}` : `FAIL ${no}`); if (!pass) failures++; };

const offered = await page.locator(".note-bar-actions button").allInnerTexts();
ok(offered.includes("Recover") && offered.includes("Delete Now"),
  "a deleted note offers Recover and Delete Now", `it offered ${JSON.stringify(offered)}`);

// Nothing is begun in Recently Deleted: New Note is dimmed there, and Ctrl+N starts nothing.
ok(await page.getByRole("button", { name: "New note" }).isDisabled(), "New Note is dimmed in Recently Deleted",
  "New Note could be pressed in Recently Deleted");
await page.locator(".note-row").first().focus();
await page.keyboard.press("Control+n");
await page.waitForTimeout(300);
ok(await page.locator('[aria-label="New note"][role="textbox"]').count() === 0, "and Ctrl+N begins no note there",
  "Ctrl+N began a note inside Recently Deleted");

await page.getByRole("button", { name: "Delete Now" }).click();
await page.locator(".confirm").getByRole("button", { name: "Cancel" }).click();
await page.waitForTimeout(300);
ok(await page.evaluate(() => window.__deletes === 0),
  "Delete Now does nothing until it is confirmed", "Delete Now deleted without asking");

await page.getByRole("button", { name: "Delete Now" }).click();
await page.locator(".confirm .confirm-answer").click();
await page.waitForTimeout(300);
ok(await page.evaluate(() => window.__deletes === 1), "confirmed, it deletes", "confirmed, nothing happened");

await page.getByRole("button", { name: "Recover" }).click();
await page.waitForTimeout(300);
const moves = await page.evaluate(() => window.__moves);
ok(moves.length === 1 && moves[0].guid === "T1" && !moves[0].folderGuid,
  "Recover puts it back in Notes", `Recover sent ${JSON.stringify(moves)}`);
await b.close();
process.exitCode = failures === 0 ? 0 : 1;
