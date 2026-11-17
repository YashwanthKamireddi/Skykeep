// A photo opens and closes at once — no animation at all, nothing moving or fading to flicker:
// the viewer is there the frame after a double-click and gone the frame after Escape; nothing
// on the page animates; the grid is painted again the moment the viewer goes, no tile left
// hidden. The window's own colour follows what fills its edges.
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

for (const round of [1, 2]) {
  const trace = await page.evaluate((n) => new Promise((resolve) => {
    const frames = [];
    let at = null;
    const t0 = performance.now();
    const tick = () => {
      frames.push({
        t: Math.round(performance.now() - t0),
        viewer: !!document.querySelector(".px-viewer"),
        animating: document.getAnimations().filter((a) => a.playState === "running").length,
        gridHidden: getComputedStyle(document.querySelector(".px-scroll")).visibility === "hidden",
        lifted: document.querySelectorAll("[data-lifted]").length,
      });
      if (performance.now() - t0 < 1400) requestAnimationFrame(tick); else resolve({ frames, at });
    };
    requestAnimationFrame(tick);
    document.querySelectorAll(".px-tile")[n].dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    setTimeout(() => { at = Math.round(performance.now() - t0); window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); }, 700);
  }), round);
  const f = trace.frames;
  const opened = f.findIndex((x) => x.viewer);
  const afterClose = f.filter((x) => x.t > trace.at);
  const closedIn = afterClose.findIndex((x) => !x.viewer);
  ok(opened >= 0 && opened <= 12, `open ${round}: the viewer is there at once  (frame ${opened})`);
  ok(closedIn >= 0 && closedIn <= 2, `close ${round}: and gone at once  (${closedIn} frames after Escape)`);
  ok(f.every((x) => x.animating === 0), `${round}: nothing animates as a photo opens or closes  (max ${Math.max(...f.map((x) => x.animating))})`);
  const gone = afterClose.slice(closedIn);
  ok(gone.length > 0 && gone.every((x) => !x.gridHidden && x.lifted === 0), `${round}: the grid is painted again and whole the moment it goes`);
  await page.waitForTimeout(200);
}

// The window's own colour follows what fills its edges, so a resize shows no lighter band.
const rgb = (sel) => page.evaluate((s) => /(\d+), (\d+), (\d+)/.exec(getComputedStyle(document.querySelector(s)).backgroundColor).slice(1).join(","), sel);
const body = await rgb("body");
ok((await page.evaluate(() => window.__tints?.at(-1))) === body, `the window is the app's colour  ${body}`);
await page.locator(".px-tile").nth(1).dblclick();
await page.waitForTimeout(300);
const backdrop = await rgb(".px-backdrop");
ok((await page.evaluate(() => window.__tints.at(-1))) === backdrop, `the viewer's own colour while a photo is open  ${backdrop}`);
await page.keyboard.press("Escape");
await page.waitForTimeout(300);
ok((await page.evaluate(() => window.__tints.at(-1))) === body, "and the app's colour again once it closes");
await page.locator(".px-tile").nth(1).dblclick();
await page.waitForTimeout(300);
ok(await page.locator(".px-viewer").evaluate((v) => getComputedStyle(v).visibility === "visible" && getComputedStyle(v.querySelector(".px-backdrop")).opacity === "1"), "a photo opened again is fully shown");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
