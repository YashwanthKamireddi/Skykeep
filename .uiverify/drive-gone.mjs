// A folder deleted on another device while it is open here: Drive steps back out to the
// folder above, says why, and never shows the gone folder as merely empty.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, driveFixture: true });
const page = await ctx.newPage();
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Drive");
await page.waitForTimeout(400);
let failures = 0;
const ok = (pass, what) => { console.log(`${pass ? "PASS" : "FAIL"} ${what}`); if (!pass) failures++; };
const crumbs = () => page.locator(".crumb").allInnerTexts();

await page.evaluate(() => { window.__driveGone = "FOLDER::a"; });
await page.locator(".drive-row-hit", { hasText: "Downloads" }).dblclick();
await page.waitForTimeout(600);
ok(!(await crumbs()).some((t) => t.includes("Downloads")), `stepped back out of the gone folder  ${JSON.stringify(await crumbs())}`);
ok((await page.locator(".toast.is-error", { hasText: "deleted or moved on another device" }).count()) === 1, "and says why");
ok((await page.locator(".drive-row-hit", { hasText: "Semester Plan" }).count()) === 1, "showing the folder above");
await b.close();
process.exitCode = failures ? 1 : 0;
