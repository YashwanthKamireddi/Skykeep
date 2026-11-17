// A table cell being typed in when a sync lands — a phone's edit to another cell of the same
// table — keeps what is being typed, and the cursor in it: the other cell's change shows once
// this one is left, and what was typed is saved. The redraw had thrown the typing away.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(() => { window.__tableNote = true; });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); failures++; });
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.locator(".note-row", { hasText: "Groceries" }).first().click();
await page.waitForSelector(".nf-table .nf-cell-edit");
const cell = () => page.locator(".nf-table tr").nth(1).locator(".nf-cell-edit").first();
await cell().click();
await page.keyboard.press("End");
await page.keyboard.type(" and eggs");
// A phone's edit to another row arrives meanwhile.
await page.evaluate(() => {
  window.__table[2] = ["Bread", "1"];
  window.__emit("quince://synced", { services: [], asOf: Date.now() / 1000, tookMs: 5 });
});
await page.waitForTimeout(600);
const typing = await cell().textContent();
const focused = await page.evaluate(() => document.activeElement?.classList.contains("nf-cell-edit") ?? false);
ok(typing === "Milk and eggs" && focused, `what is being typed stays, the cursor in it  (${JSON.stringify(typing)}, focused ${focused})`);
await page.keyboard.press("Enter");
await page.waitForTimeout(500);
const sent = await page.evaluate(() => (window.__cellWrites ?? []).map((w) => w.text ?? w.value ?? w));
ok(JSON.stringify(sent).includes("Milk and eggs"), `and is saved once left  ${JSON.stringify(sent)}`);
// The phone's change shows once the cell is left, at the next sync.
await page.evaluate(() => window.__emit("quince://synced", { services: [], asOf: Date.now() / 1000, tookMs: 5 }));
await page.waitForTimeout(600);
const third = await page.locator(".nf-table tr").nth(2).evaluate((r) => [...r.cells].map((c) => c.textContent));
ok(JSON.stringify(third) === JSON.stringify(["Bread", "1"]), `then the other row's change shows  ${JSON.stringify(third)}`);
await b.close();
process.exitCode = failures ? 1 : 0;
