// A photo keeps its exact shape and fills the stage at every window size: as large as fits
// (never past 1.5× its pixels), centred, laid out by the engine as the window is resized —
// small, large, tall, wide — with no stretched or cut picture.
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

await page.locator(".px-tile").nth(0).dblclick(); // 4032 × 3024
await page.waitForTimeout(700);
const measure = () => page.evaluate(() => {
  const st = document.querySelector(".px-stage");
  const img = document.querySelector(".px-stage .px-shown");
  const cs = getComputedStyle(st);
  const s = st.getBoundingClientRect();
  const box = { l: s.left + parseFloat(cs.paddingLeft), t: s.top + parseFloat(cs.paddingTop), r: s.right - parseFloat(cs.paddingRight), b: s.bottom - parseFloat(cs.paddingBottom) };
  const r = img.getBoundingClientRect();
  return { box, r: { l: r.left, t: r.top, r: r.right, b: r.bottom, w: r.width, h: r.height } };
});
for (const [w, h] of [[1280, 800], [640, 520], [1920, 1040], [700, 1000], [1500, 600]]) {
  await page.setViewportSize({ width: w, height: h });
  await page.waitForTimeout(150);
  const { box, r } = await measure();
  const bw = box.r - box.l, bh = box.b - box.t;
  const shape = Math.abs(r.w / r.h - 4032 / 3024) < 0.01;
  const inside = r.l >= box.l - 1 && r.t >= box.t - 1 && r.r <= box.r + 1 && r.b <= box.b + 1;
  const fills = Math.abs(r.w - bw) < 1.5 || Math.abs(r.h - bh) < 1.5;
  const centred = Math.abs((r.l + r.r) / 2 - (box.l + box.r) / 2) < 1.5 && Math.abs((r.t + r.b) / 2 - (box.t + box.b) / 2) < 1.5;
  ok(shape && inside && fills && centred, `${w}×${h}: its own shape, inside the stage, filling it, centred  (${r.w.toFixed(0)}×${r.h.toFixed(0)} in ${bw.toFixed(0)}×${bh.toFixed(0)})`);
}
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
