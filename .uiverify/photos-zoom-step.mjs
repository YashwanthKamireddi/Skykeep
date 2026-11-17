// Stepping on from a zoomed photo shows the next one at its fit, and never asks for the next
// one's full-size original: the zoom stayed on for the render that drew the next photo, so the
// whole original (for a HEIC, downloaded and decoded) was asked for and never looked at.
//
//   URL=http://localhost:4203 ENGINE=webkit node .uiverify/photos-zoom-step.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(500);
await page.locator('.px-tile[data-id="p1"]').dblclick();
await page.waitForTimeout(400);
await page.keyboard.press("z");
await page.waitForTimeout(400);
ok(/scale\(/.test(await page.locator(".px-pan").evaluate((e) => e.style.transform)), "Z zooms in");
await page.evaluate(() => {
  window.__sharp = [];
  new MutationObserver((list) => {
    for (const m of list) for (const n of m.addedNodes) {
      if (n.nodeType !== 1) continue;
      for (const img of [n, ...n.querySelectorAll("img")]) if (img.classList?.contains("px-sharp")) window.__sharp.push(img.getAttribute("src"));
    }
  }).observe(document.body, { childList: true, subtree: true });
});
await page.keyboard.press("ArrowRight");
const now = await page.evaluate(() => ({ transform: document.querySelector(".px-pan")?.style.transform ?? null, zoomedClass: document.querySelector(".px-pan")?.classList.contains("is-zoomed") }));
await page.waitForTimeout(400);
const sharp = await page.evaluate(() => window.__sharp);
ok(!sharp.some((s) => /p2/.test(decodeURIComponent(s ?? ""))), `the next photo's original is never asked for  ${JSON.stringify(sharp)}`);
ok(!now.zoomedClass && now.transform === "", `the next photo is not drawn zoomed, from its first frame  ${JSON.stringify(now)}`);
ok((await page.locator(".px-pan").evaluate((e) => e.style.transform)) === "", "and it settles at its fit");
await b.close();
console.log(failures ? `\n${failures} FAILED` : "\nall passed");
process.exit(failures ? 1 : 0);
