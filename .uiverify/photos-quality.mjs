// A photo at full quality from the first frame: the picture made the size the stage shows it —
// at least as many pixels as it is drawn with, so the engine never stretches or shrinks it —
// decoded before it opens, and for the photos either side while one shows; alone, never a
// smaller copy first; stepping lands on one too.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
// Pictures here cannot be fetched; decoding is made to succeed, as it does in the app.
await ctx.addInitScript(() => { HTMLImageElement.prototype.decode = function () { return Promise.resolve(); }; });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const shown = () => page.locator(".px-stage .px-shown").getAttribute("data-picture");

await page.locator(".px-tile").nth(0).dblclick();
await page.waitForTimeout(80);
const enough = () => page.evaluate(() => {
  const img = document.querySelector(".px-stage .px-shown");
  const step = Number(/fit(\d+)/.exec(decodeURIComponent(img.getAttribute("data-picture")))?.[1] ?? 0);
  // Its laid-out size, not its box on screen: that is still growing as the viewer opens.
  const long = Math.max(img.offsetWidth, img.offsetHeight) * devicePixelRatio;
  return step >= long - 1 && step < long * 2;
});
ok(/fit\d+/.test((await shown()) ?? "") && (await enough()) && (await page.locator(".px-sharp").count()) === 0, `it opens on the picture made its shown size, alone  ${await shown()}`);
await page.keyboard.press("ArrowRight");
await page.waitForTimeout(30);
ok(/p2.*fit\d+/.test(decodeURIComponent((await shown()) ?? "")) && (await enough()), `a step lands on the next one, made its shown size, at once  ${await shown()}`);
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
