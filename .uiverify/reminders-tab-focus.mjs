// Renamed and then tucked under the one above with Tab, a reminder's title keeps the cursor,
// as on the Mac, so typing goes on in it. The rename had redrawn the field and dropped it.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); failures++; });
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.locator(".rx-list", { hasText: "Daily" }).click();
await page.waitForTimeout(400);
const field = page.getByLabel("Rename Pick up the parcel");
await field.click();
await page.keyboard.press("End");
await page.keyboard.type(" today");
await page.keyboard.press("Tab");
await page.waitForTimeout(400);
const where = await page.evaluate(() => document.activeElement?.classList.contains("rx-title") ? document.activeElement.value : document.activeElement?.className);
ok(where === "Pick up the parcel today", `the title keeps the cursor after Tab  (${JSON.stringify(where)})`);
await page.keyboard.type("!");
const now = await page.locator(".rx-title").evaluateAll((els) => els.map((e) => e.value));
ok(now.includes("Pick up the parcel today!"), `and typing goes on in it  ${JSON.stringify(now)}`);
await b.close();
process.exitCode = failures ? 1 : 0;
