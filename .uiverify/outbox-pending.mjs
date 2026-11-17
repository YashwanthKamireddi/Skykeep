// Edits kept on this computer and not yet at Apple say so ("Waiting to sync"), and one Apple
// turned down is shown with what it held — copied out, sent again or let go, never dropped.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
await ctx.addInitScript(() => {
  window.__outbox = {
    waiting: [{ service: "notes", record: "n1", kind: "save" }],
    refused: [{ id: 3, service: "notes", record: "n1", kind: "save", madeAt: 0,
      reason: "This note was changed on another device while you were writing, in the same place.",
      content: "Curtains\nmy offline line" }],
  };
});
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.locator(".note-row", { hasText: "Curtains" }).click();
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
ok((await page.locator(".note-when").innerText()).includes("Waiting to sync"), "a note kept here and not yet at Apple says so");
const notice = page.locator(".unsent");
ok(/changed on another device/.test(await notice.innerText()), "a refused edit says why");
ok((await notice.innerText()).includes("my offline line"), "and shows what it held");
await notice.getByRole("button", { name: "Try Again" }).click();
await page.waitForTimeout(300);
ok(JSON.stringify(await page.evaluate(() => window.__outboxCalls)) === JSON.stringify([["outbox_resend", 3]]), "Try Again sends it again");
ok((await page.locator(".unsent").count()) === 0, "and it leaves the notice");
// Sent: the backend says the queue moved, and the hint goes.
await page.evaluate(() => { window.__outbox = { waiting: [], refused: [] }; window.__emit("quince://outbox-changed", null); });
await page.waitForTimeout(300);
ok(!(await page.locator(".note-when").innerText()).includes("Waiting to sync"), "once Apple has it, the hint goes");
await b.close();
process.exitCode = failures ? 1 : 0;
