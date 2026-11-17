// The viewer with pictures of real size: the one made for the screen inside the stage; zoomed
// in, the whole one (4000×3000) laid exactly over it however large it is — before, its own
// size stretched the frame: the photo tore into two and ran off the window. The same after
// the window is resized, and after stepping to the next photo.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
await ctx.addInitScript(() => {
  const big = (w, h, hue) => { const c = document.createElement("canvas"); c.width = w; c.height = h; const g = c.getContext("2d");
    g.fillStyle = `hsl(${hue} 60% 50%)`; g.fillRect(0, 0, w, h); return c.toDataURL("image/jpeg", 0.5); };
  const full = big(4000, 3000, 210), medium = big(1024, 768, 30);
  new MutationObserver(() => document.querySelectorAll(".px-stage img").forEach((i) => {
    if (i.src.startsWith("data:")) return;
    i.src = i.classList.contains("px-sharp") ? full : medium;
  })).observe(document, { subtree: true, childList: true, attributes: true, attributeFilter: ["src"] });
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const boxes = () => page.evaluate(() => {
  const r = (s) => { const e = document.querySelector(s); if (!e) return null; const b = e.getBoundingClientRect(); return { x: Math.round(b.x), y: Math.round(b.y), w: Math.round(b.width), h: Math.round(b.height) }; };
  return { shown: r(".px-shown"), sharp: r(".px-sharp.is-in"), stage: r(".px-stage") };
});
const lies = (b) => b.sharp && b.shown && Math.abs(b.sharp.x - b.shown.x) <= 1 && Math.abs(b.sharp.y - b.shown.y) <= 1 &&
  Math.abs(b.sharp.w - b.shown.w) <= 1 && Math.abs(b.sharp.h - b.shown.h) <= 1;
const inside = (b) => b.shown.x >= b.stage.x - 1 && b.shown.y >= b.stage.y - 1 &&
  b.shown.x + b.shown.w <= b.stage.x + b.stage.w + 1 && b.shown.y + b.shown.h <= b.stage.y + b.stage.h + 1;

// Opening fetches no whole picture: only the one made for the screen.
await page.locator(".px-tile").nth(1).dblclick();
await page.waitForTimeout(1200);
let b1 = await boxes();
ok((await page.locator(".px-sharp").count()) === 0 && inside(b1), `a photo opens on the picture made for the screen, inside the stage  ${JSON.stringify(b1)}`);
// Zoomed in, the whole picture lies exactly on it.
const zoomIn = async () => {
  await page.locator(".px-pan").dblclick({ position: { x: 30, y: 30 } });
  await page.waitForTimeout(900);
};
await zoomIn();
b1 = await boxes();
ok(lies(b1), `zoomed in, the whole picture lies exactly on it  ${JSON.stringify(b1)}`);
await page.keyboard.press("Escape");
await page.waitForTimeout(500);
await page.setViewportSize({ width: 900, height: 620 });
await page.waitForTimeout(500);
await zoomIn();
b1 = await boxes();
ok(lies(b1), `after a resize too  ${JSON.stringify(b1)}`);
await page.keyboard.press("Escape");
await page.waitForTimeout(500);
await page.setViewportSize({ width: 1280, height: 800 });
await page.keyboard.press("ArrowLeft");
await page.waitForTimeout(1200);
b1 = await boxes();
ok(inside(b1), `and on the next photo  ${JSON.stringify(b1)}`);
await page.keyboard.press("Escape");
await page.waitForTimeout(700);
// A narrow window, a photo opened in it: inside, never wider than the window lets it be.
await page.setViewportSize({ width: 760, height: 700 });
await page.waitForTimeout(300);
await page.locator(".px-tile").nth(0).dblclick();
await page.waitForTimeout(1200);
b1 = await boxes();
const win = await page.evaluate(() => window.innerWidth);
ok(inside(b1) && b1.shown.x + b1.shown.w <= win, `in a narrow window the photo stays inside it  ${JSON.stringify(b1)}`);
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
