// The keys icloud.com's Photos takes, beyond the arrows: F (as well as "."), H to hide, = and −
// for the thumbnails, I for the details; in a photo, Z to zoom and Ctrl+"." to close.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
// Every question agreed to, as soon as it is asked.
await page.addInitScript(() => addEventListener("DOMContentLoaded", () =>
  new MutationObserver(() => document.querySelector(".confirm .confirm-answer")?.click()).observe(document.body, { childList: true, subtree: true })));
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const across = () => page.locator(".px-grid").first().evaluate((g) => getComputedStyle(g).gridTemplateColumns.split(" ").length);

await page.locator(".px-tile").nth(1).click();
await page.keyboard.press("f");
await page.waitForTimeout(200);
ok((await page.evaluate(() => window.__flags.at(-1))).flag === "favorite", "F favorites the selected photo");

const before = await across();
await page.keyboard.press("=");
await page.waitForTimeout(100);
ok((await across()) < before, `= makes the thumbnails bigger  ${before} → ${await across()}`);
await page.keyboard.press("-");
await page.waitForTimeout(100);
ok((await across()) === before, "and − back");

await page.keyboard.press("i");
await page.waitForTimeout(600);
ok((await page.locator(".px-viewer").count()) === 1 && (await page.locator(".px-info").count()) === 1, "I opens the photo with its details");
await page.keyboard.press("z");
await page.waitForTimeout(400); // the zoom glides
const k = await page.locator(".px-pan").evaluate((el) => new DOMMatrix(getComputedStyle(el).transform).a);
ok(k > 2, `Z zooms in  ×${k.toFixed(2)}`);
await page.keyboard.press("z");
await page.waitForTimeout(400); // the zoom glides
ok((await page.locator(".px-pan").evaluate((el) => new DOMMatrix(getComputedStyle(el).transform).a)) === 1, "and Z again fits it");
await page.keyboard.press("Control+.");
await page.waitForTimeout(650);
ok((await page.locator(".px-viewer").count()) === 0, 'Ctrl+"." closes the photo');

await page.locator(".px-tile").nth(3).click();
await page.keyboard.press("h");
await page.waitForTimeout(300);
ok((await page.evaluate(() => window.__flags.at(-1))).flag === "hidden" && (await page.locator(".px-tile").count()) === 4, "H hides it, asked first");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
