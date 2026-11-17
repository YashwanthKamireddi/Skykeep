// Sorting, as Photos offers it: Date Captured or Date Added, Newest or Oldest First, with a
// check beside what is chosen; the months follow; each view keeps its own sort, and keeps it
// after a reload. Recents is by date added from the first.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const order = () => page.locator(".px-tile").evaluateAll((t) => t.map((x) => x.dataset.id).join(","));
const months = () => page.locator(".px-month-title").allInnerTexts();

ok((await order()) === "p1,p2,p3,p4,p5", `Library starts newest taken first  ${await order()}`);
await page.getByRole("button", { name: "Sort" }).click();
const checked = await page.locator('.row-menu [aria-checked="true"]').allInnerTexts();
ok(checked.map((t) => t.replace("✓", "").trim()).join() === "Date Captured,Newest First", `the menu checks what is chosen  ${JSON.stringify(checked)}`);
await page.locator(".row-menu").getByText("Oldest First").click();
await page.waitForTimeout(200);
ok((await order()) === "p5,p4,p3,p2,p1", `Oldest First turns it round  ${await order()}`);
ok(/September/.test((await months())[0]), `and the months with it  ${JSON.stringify(await months())}`);

await page.locator(".px-source", { hasText: "Recents" }).click();
await page.waitForTimeout(250);
ok((await order()).startsWith("p4"), `Recents is by date added, newest first  ${await order()}`);
ok(/October/.test((await months())[0]), "with each photo under the month it was added");

await page.reload({ waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(400);
ok((await order()) === "p5,p4,p3,p2,p1", "Library keeps its sort after a reload");
await page.locator(".px-source", { hasText: "Trips" }).click();
await page.waitForTimeout(250);
ok((await page.getByRole("button", { name: "Sort" }).count()) === 0, "an album keeps its own order, with no sort to choose");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
