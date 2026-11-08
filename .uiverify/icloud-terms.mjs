// Issue #5: Apple has new iCloud terms the person has not agreed to, and opens only part of a
// web session until they do — every service fails, Calendar and Contacts too. One calm page
// says what to do at icloud.com, with Try Again; agreed, it goes and the library is back.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.BASE ?? process.env.URL ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

async function open(setup) {
  const ctx = await b.newContext({ viewport: { width: 1180, height: 800 } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
  await ctx.addInitScript(setup);
  const page = await ctx.newPage();
  await page.goto(BASE, { waitUntil: "networkidle" });
  await page.waitForTimeout(400);
  return { ctx, page };
}

{
  const { ctx, page } = await open(() => { window.__termsNeeded = true; window.__termsAfterRefresh = true; });
  const heading = page.getByRole("heading", { name: "Agree to the new iCloud terms" });
  ok(await heading.isVisible(), "new terms waiting: the page says to agree to them");
  ok((await page.locator(".icloud-com-waiting").innerText()).includes("icloud.com"), "and where");
  // Calendar's own window: the notice is in the calendar's place, so it never draws meanwhile.
  await openScreen(page, "Calendar").catch(() => {});
  await page.waitForTimeout(200);
  ok(await heading.isVisible(), "Calendar too: every service waits on it");
  await page.getByRole("button", { name: "Try Again" }).click();
  await page.waitForTimeout(250);
  ok((await page.evaluate(() => window.__accountRefreshes)) === 1, "Try Again asks Apple for the account again");
  ok(await heading.isVisible(), "not agreed yet: the page stays");

  // Agreed at icloud.com, then Try Again.
  await page.evaluate(() => { window.__termsAfterRefresh = false; });
  await page.getByRole("button", { name: "Try Again" }).click();
  await page.waitForTimeout(300);
  ok(!(await heading.isVisible()), "agreed: the page goes");
  ok(await page.locator(".cx").first().isVisible(), "and the calendar is there");
  await ctx.close();
}

{
  const { ctx, page } = await open(() => { window.__termsNeeded = true; });
  await openScreen(page, "Notes").catch(() => {});
  // Agreed meanwhile; the next pass finds it and says so.
  await page.evaluate(() => { window.__termsNeeded = false; window.__emit?.("quince://synced", { services: [], asOf: null, tookMs: 0, changed: [] }); });
  await page.waitForTimeout(300);
  ok(!(await page.getByRole("heading", { name: "Agree to the new iCloud terms" }).isVisible()), "agreed elsewhere: the next pass clears it by itself");
  await ctx.close();
}

{
  const { ctx, page } = await open(() => { window.__repairNeeded = true; });
  ok(await page.getByRole("heading", { name: "Finish setting up iCloud" }).isVisible(), "an account Apple wants finished says so");
  await ctx.close();
}

{
  const { ctx, page } = await open(() => {});
  ok(!(await page.locator(".icloud-com-waiting").count()), "nothing waiting: nothing shown");
  await ctx.close();
}

await b.close();
if (failures) { console.log(`${failures} failed`); process.exit(1); }
console.log("all passed");
