// Apple's Drive busy while a folder kept here is already on screen (as when Skykeep opens and
// every app syncs at once): the folder shows, nothing is said, and it is asked again quietly.
// A folder with nothing kept that can't be read still says so.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (pass, what) => { console.log(`${pass ? "PASS" : "FAIL"} ${what}`); if (!pass) failures++; };
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, driveFixture: true });
await ctx.addInitScript(() => { window.__driveBusy = 1; });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Drive");
await page.waitForTimeout(800);
ok((await page.evaluate(() => (window.__busyAsks ?? []).length)) === 1, "Apple was busy for the first ask");
ok((await page.locator(".drive-row-hit").count()) > 0, "the folder kept here shows");
ok((await page.locator(".toast.is-error").count()) === 0, "and nothing is said about Apple being busy");
await page.waitForTimeout(5600);
ok((await page.evaluate(() => window.__driveBusy)) === 0 && (await page.locator(".toast.is-error").count()) === 0, "asked again quietly, still nothing said");
await b.close();
console.log(failures ? `${failures} FAILED` : "all passed");
process.exit(failures ? 1 : 0);
