// A library of 50,000 photos paged over IPC: the first screen paints from the first page before
// the index (made slow here) lands; no answer carries more than a page of photos; scrolling
// fetches the pages ahead, so tiles are there when the grid reaches them; the photos held, the
// tiles drawn and (in Chromium) the JS heap stay flat from top to bottom; a fling never freezes.
//
//   URL=http://localhost:4280 node .uiverify/photos-paging.mjs   (ENGINE=webkit for WebKit)
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const N = 50_000;
const PAGE = 240;
const KEEP = 2400;
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
await ctx.addInitScript(([n]) => {
  const P = (i, t) => ({ id: `q${i}`, master: `mq${i}`, takenAt: t, addedAt: t, kind: i % 17 === 4 ? "video" : "image", width: 4032, height: 3024,
    duration: i % 17 === 4 ? 21 : null, filename: `IMG_${i}.JPG`, size: 2400000, uti: "public.jpeg", favorite: i % 9 === 0, hidden: false,
    deleted: false, deletedAt: null, subtype: 0, changeTag: null });
  window.__photos = Array.from({ length: n }, (_, i) => P(i, 1792600000 - i * 3000));
  window.__indexDelay = 1500;
  // Every answer the window gets, by command, with how many photos it carried. The stub's own
  // reads of whole lists (to make an index or a page from) are told apart by depth.
  window.__calls = [];
  let inner = 0;
  const wrap = () => {
    const real = window.__TAURI_INTERNALS__?.invoke;
    if (!real || real.__wrapped) return;
    const w = async (cmd, args, o) => {
      const mine = inner === 0;
      if (cmd === "photos_index" || cmd === "photos_page") inner++;
      try {
        const out = await real(cmd, args, o);
        if (mine) window.__calls.push({ cmd, at: performance.now(), n: Array.isArray(out) ? out.length : out?.ids ? -out.ids.length : 0 });
        return out;
      } finally {
        if (cmd === "photos_index" || cmd === "photos_page") inner--;
      }
    };
    w.__wrapped = true;
    window.__TAURI_INTERNALS__.invoke = w;
  };
  wrap();
  document.addEventListener("DOMContentLoaded", wrap);
}, [N]);
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
const cdp = engineName === "chromium" ? await ctx.newCDPSession(page) : null;
const heap = async () => {
  if (!cdp) return null;
  await cdp.send("HeapProfiler.collectGarbage");
  return (await cdp.send("Runtime.getHeapUsage")).usedSize;
};

const t0 = Date.now();
await openScreen(page, "Photos");
await page.locator(".px-tile").first().waitFor();
const painted = Date.now() - t0;
const indexIn = await page.evaluate(() => (window.__indexReads ?? []).length);
ok(indexIn === 0 || (await page.evaluate(() => !window.__calls.some((c) => c.cmd === "photos_index"))), `the first screen paints from the first page, before the index lands  (${painted} ms)`);
await page.waitForFunction(() => window.__calls.some((c) => c.cmd === "photos_index"), null, { timeout: 8000 }).catch(() => ok(false, "the index is asked for"));
await page.waitForTimeout(500);
const calls = await page.evaluate(() => window.__calls);
const photoCalls = calls.filter((c) => ["photos_list", "photos_page", "photos_get", "photos_search", "photos_in_album"].includes(c.cmd));
ok(!calls.some((c) => c.cmd === "photos_list"), "the whole list is never asked for");
ok(photoCalls.every((c) => c.n <= PAGE), `no answer carries more than a page  (largest ${Math.max(...photoCalls.map((c) => c.n))})`);
const height = await page.evaluate(() => document.querySelector(".px-scroll").scrollHeight);
ok(height > N / 10 * 50, `the grid is laid out for all ${N.toLocaleString()} from the index  (${height.toLocaleString()} px)`);
const heapStart = await heap();

/** Tiles drawn in the window's view, and blocks still waiting for their photos there. */
const inView = () => page.evaluate(() => {
  const el = document.querySelector(".px-scroll");
  const r = el.getBoundingClientRect();
  const tiles = [...el.querySelectorAll(".px-tile")].filter((t) => { const b = t.getBoundingClientRect(); return b.bottom > r.top && b.top < r.bottom; }).length;
  return { tiles, drawn: el.querySelectorAll(".px-tile").length };
});

// Scrolled at a steady 3,000 px/s for a while: pages fetched ahead, so the screen is never bare.
const run = (speed, ms) => page.evaluate(([speed, ms]) => new Promise((done) => {
  const el = document.querySelector(".px-scroll");
  const gaps = [];
  let bare = 0;
  let last = performance.now();
  const start = last;
  const step = (now) => {
    gaps.push(now - last);
    el.scrollTop += (speed * (now - last)) / 1000;
    last = now;
    const r = el.getBoundingClientRect();
    const mid = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
    if (!mid?.closest(".px-tile, .px-month-title")) bare++;
    if (now - start < ms && el.scrollTop + el.clientHeight < el.scrollHeight - 2) requestAnimationFrame(step);
    else done({ gaps: gaps.slice(2), bare, frames: gaps.length });
  };
  requestAnimationFrame(step);
}), [speed, ms]);
const stats = (gaps) => {
  const s = [...gaps].sort((x, y) => x - y);
  const q = (p) => s[Math.min(s.length - 1, Math.floor(p * s.length))];
  return { p50: q(0.5), p95: q(0.95), max: s[s.length - 1] };
};
const BOUND = engineName === "chromium" ? 120 : 900;
const steady = await run(3000, 5000);
const st = stats(steady.gaps);
console.log(`  steady scroll: ${steady.frames} frames, p50 ${st.p50.toFixed(1)} ms, p95 ${st.p95.toFixed(1)} ms, max ${st.max.toFixed(1)} ms, ${steady.bare} frames with the middle bare`);
ok(st.max < BOUND, `a steady scroll never freezes  (longest frame ${Math.round(st.max)} ms)`);
ok(steady.bare <= steady.frames * 0.1, `pages land ahead of a steady scroll  (${steady.bare} of ${steady.frames} frames bare in the middle)`);
const fling = await run(9000, 3000);
const sf = stats(fling.gaps);
console.log(`  fling: p50 ${sf.p50.toFixed(1)} ms, p95 ${sf.p95.toFixed(1)} ms, max ${sf.max.toFixed(1)} ms`);
ok(sf.max < BOUND, `a fling never freezes  (longest frame ${Math.round(sf.max)} ms)`);

// Jumped through the whole library, top to bottom: each place fills in, and what is held stays flat.
const drawnAt = [];
for (const at of [0.2, 0.4, 0.6, 0.8, 1]) {
  await page.evaluate((f) => { const el = document.querySelector(".px-scroll"); el.scrollTop = (el.scrollHeight - el.clientHeight) * f; }, at);
  await page.waitForFunction(() => {
    const el = document.querySelector(".px-scroll");
    const r = el.getBoundingClientRect();
    return [...el.querySelectorAll(".px-tile")].some((t) => { const b = t.getBoundingClientRect(); return b.bottom > r.top && b.top < r.bottom; });
  }, null, { timeout: 5000 }).catch(() => {});
  await page.waitForTimeout(300);
  const v = await inView();
  drawnAt.push(v.drawn);
  ok(v.tiles > 20, `${Math.round(at * 100)}% down, the screen fills in  (${v.tiles} tiles in view, ${v.drawn} drawn)`);
}
const fetched = await page.evaluate(() => window.__calls.filter((c) => c.cmd === "photos_get").reduce((a, c) => a + c.n, 0));
ok(fetched > KEEP, `photos fetched as they came near  (${fetched.toLocaleString()} in all)`);
ok(Math.max(...drawnAt) <= KEEP, `tiles drawn stay bounded however far it goes  (${drawnAt.join(", ")})`);
const heapEnd = await heap();
if (heapStart !== null) {
  const mb = (x) => (x / 1024 ** 2).toFixed(1);
  console.log(`  JS heap: ${mb(heapStart)} MB at the top, ${mb(heapEnd)} MB at the foot`);
  ok(heapEnd - heapStart < 25 * 1024 ** 2, `memory stays flat from top to bottom  (${mb(heapStart)} → ${mb(heapEnd)} MB)`);
}

// The last photo opens in the viewer, and steps back across a page boundary.
await page.locator(".px-tile").last().dblclick();
await page.locator(".px-viewer, .px-stage").first().waitFor({ timeout: 5000 }).catch(() => {});
ok(await page.locator(".px-viewer, .px-stage").first().isVisible(), "a photo at the foot of the library opens");
for (let i = 0; i < 5; i++) await page.keyboard.press("ArrowLeft");
await page.waitForTimeout(300);
ok(await page.locator(".px-viewer, .px-stage").first().isVisible(), "and steps on through the photos before it");

await b.close();
process.exitCode = failures ? 1 : 0;
