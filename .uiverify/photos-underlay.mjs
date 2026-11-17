// Never a black stage: the viewer paints the grid's thumbnail under every picture (photo or
// video still) until the sharper one is decoded; and a month once built stays built.
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

const lazy = await page.locator(".px-tile img[loading=lazy]").count();
ok(lazy === 0, `tile thumbnails are fetched with their month, not on the engine's lazy margin  (${lazy} lazy)`);
await page.locator(".px-tile").nth(0).dblclick();
await page.waitForTimeout(500);
const bg = await page.locator(".px-stage .px-shown").evaluate((el) => getComputedStyle(el).backgroundImage);
ok(/thumb/.test(decodeURIComponent(bg)), `the shown picture has its thumbnail under it  ${bg.slice(0, 80)}`);
const tiles = await page.locator(".px-tile").count();
await page.keyboard.press("Escape");
await page.waitForTimeout(500);
await page.locator(".px-scroll").evaluate((el) => (el.scrollTop = el.scrollHeight));
await page.waitForTimeout(300);
await page.locator(".px-scroll").evaluate((el) => (el.scrollTop = 0));
await page.waitForTimeout(300);
const after = await page.locator(".px-tile").count();
ok(after >= tiles, `months built stay built after scrolling away and back  (${tiles} → ${after})`);
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
