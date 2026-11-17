// Scrolling a library of 2,000 photos, with real pictures to decode, at a steady fling's speed:
// the time between frames, as the engine gives them. Every month is built as it comes near and
// kept, so the scroll is the work a user's own scroll does. Prints the numbers, and fails on a
// freeze: one frame held past the bound below. Headless WebKit paints in software, every frame
// on the CPU, so its bound is looser; what it guards against there is the old way of building a
// month whole, all at once, which held single frames for 1.6–4.3 s in a fling.
//
//   ENGINE=webkit node .uiverify/photos-scroll.mjs
import http from "node:http";
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

// Twelve photo-like pictures, made once by the engine's own canvas and served from here — a
// page.route per picture is a round trip through the driver, and that was what got measured.
const maker = await b.newPage();
const pics = await maker.evaluate(() => Array.from({ length: 12 }, (_, i) => {
  const c = document.createElement("canvas"); c.width = 384; c.height = 288;
  const g = c.getContext("2d");
  const grad = g.createLinearGradient(0, 0, 384, 288);
  grad.addColorStop(0, `hsl(${i * 30} 70% 60%)`); grad.addColorStop(1, `hsl(${i * 30 + 120} 60% 35%)`);
  g.fillStyle = grad; g.fillRect(0, 0, 384, 288);
  for (let k = 0; k < 60; k++) { g.fillStyle = `hsla(${(i * 30 + k * 13) % 360} 80% 60% / 0.35)`; g.beginPath(); g.arc((k * 97) % 384, (k * 53) % 288, 10 + (k % 7) * 6, 0, 7); g.fill(); }
  return c.toDataURL("image/jpeg", 0.8).split(",")[1];
}));
await maker.close();
const bufs = pics.map((p) => Buffer.from(p, "base64"));
const server = http.createServer((q, r) => {
  const n = Number(decodeURIComponent(q.url).match(/q(\d+)/)?.[1] ?? 0);
  r.writeHead(200, { "content-type": "image/jpeg", "cache-control": "max-age=3600", "access-control-allow-origin": "*" });
  r.end(bufs[n % bufs.length]);
}).listen(0);
const picBase = `http://127.0.0.1:${server.address().port}`;

const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
await ctx.addInitScript((base) => {
  window.__picBase = base;
  const P = (i, t) => ({ id: `q${i}`, master: `mq${i}`, takenAt: t, addedAt: t, kind: i % 17 === 4 ? "video" : "image", width: 4032, height: 3024,
    duration: i % 17 === 4 ? 21 : null, filename: `IMG_${i}.JPG`, size: 2400000, uti: "public.jpeg", favorite: i % 9 === 0, hidden: false,
    deleted: false, deletedAt: null, subtype: 0, changeTag: null });
  window.__photos = Array.from({ length: 2000 }, (_, i) => P(i, 1792600000 - i * 20000));
}, picBase);
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); failures++; });
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.locator(".px-tile").first().waitFor();
await page.waitForTimeout(800);

/** Scroll the library at `speed` px/s for `ms`, one step a frame; the gaps between frames. */
const run = (speed, ms) => page.evaluate(([speed, ms]) => new Promise((done) => {
  const el = document.querySelector(".px-scroll");
  const gaps = [];
  let last = performance.now();
  const start = last;
  const step = (now) => {
    gaps.push(now - last);
    el.scrollTop += (speed * (now - last)) / 1000;
    last = now;
    if (now - start < ms && el.scrollTop + el.clientHeight < el.scrollHeight - 2) requestAnimationFrame(step);
    else done({ gaps: gaps.slice(2), reached: Math.round(el.scrollTop) });
  };
  requestAnimationFrame(step);
}), [speed, ms]);
const stats = (gaps) => {
  const s = [...gaps].sort((x, y) => x - y);
  const q = (p) => s[Math.min(s.length - 1, Math.floor(p * s.length))];
  return { frames: s.length, p50: q(0.5), p95: q(0.95), max: s[s.length - 1], late: gaps.filter((g) => g > 25).length };
};
const BOUND = engineName === "chromium" ? 120 : 900;
const fmt = (r) => `${r.frames} frames, p50 ${r.p50.toFixed(1)} ms, p95 ${r.p95.toFixed(1)} ms, max ${r.max.toFixed(1)} ms, ${r.late} late (>25 ms)`;

for (const [what, speed] of [["a steady scroll (3,000 px/s)", 3000], ["a fling (9,000 px/s)", 9000]]) {
  await page.evaluate(() => { document.querySelector(".px-scroll").scrollTop = 0; });
  await page.waitForTimeout(400);
  const r = await run(speed, 4000);
  const st = stats(r.gaps);
  console.log(`  ${what}: ${fmt(st)}  (to ${r.reached}px)`);
  ok(st.max < BOUND, `${what} never freezes  (longest frame ${Math.round(st.max)} ms, bound ${BOUND})`);
}

// Zoomed in to days, the same.
await page.evaluate(() => { document.querySelector(".px-scroll").scrollTop = 0; });
await page.keyboard.press("Control+=");
await page.waitForTimeout(600);
const r = stats((await run(3000, 3000)).gaps);
console.log(`  by day, 5 across: ${fmt(r)}`);
ok(r.max < BOUND, `grouped by day, never either  (longest frame ${Math.round(r.max)} ms)`);

await b.close();
server.close();
process.exitCode = failures ? 1 : 0;
