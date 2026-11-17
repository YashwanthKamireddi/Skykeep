// A note in Recently Deleted is read, not edited, as on the phone and icloud.com: there is no
// box to type into, and nothing typed there is saved — recovering it is how to edit it again.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(() => {
  window.__notesExtra = [{ guid: "nTr", folderGuid: "TrashFolder-CloudKit", title: "Old packing list", snippet: "", body: "Old packing list\nSocks",
    locked: false, editable: true, pinned: false, createdAt: null, modifiedAt: 1789900000, etag: null }];
});
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); failures++; });
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.locator('.note-row[data-guid="nTr"]').click();
await page.waitForTimeout(500);
ok((await page.locator(".note-rich[contenteditable='true']").count()) === 0, "no box to type into");
ok(/Socks/.test(await page.locator(".note-editor").innerText()), "its words are shown");
ok((await page.getByRole("button", { name: "Recover" }).count()) === 1, "Recover is offered");
await page.locator(".note-editor").click();
await page.keyboard.type("xyz");
await page.waitForTimeout(1300);
const saves = await page.evaluate(() => (window.__noteSaves ?? []).filter((s) => s.guid === "nTr").length);
ok(saves === 0, `nothing typed is saved  (${saves} saves)`);
await b.close();
process.exitCode = failures ? 1 : 0;
