// Years and Months on a library of fifty thousand photos over twenty years: each level is up in
// a frame or two, holds a tile a year or a month (never a box a photo), fetches pictures only for
// the tiles near the screen, scrolls without a long frame, and All Photos comes back as it was —
// the grid kept, not built again — or lands exactly on a month twenty years down.
//
// Headless WebKit paints in software, every frame on the CPU: its bounds are looser, as in
// photos-scroll, and guard against a level that freezes, not against its software paint.
import http from "node:http";
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const SWITCH = engineName === "chromium" ? 250 : 600;
const BOUND = engineName === "chromium" ? 120 : 900;
// Photo-like pictures made once by the engine's own canvas and served from here, as
// photos-scroll serves them: a page.route per picture is a round trip through the driver, and
// that was what got measured.
const maker = await b.newPage();
const pics = await maker.evaluate(() => Array.from({ length: 12 }, (_, i) => {
  const c = document.createElement("canvas"); c.width = 384; c.height = 288;
  const g = c.getContext("2d");
  const grad = g.createLinearGradient(0, 0, 384, 288);
  grad.addColorStop(0, `hsl(${i * 30} 70% 60%)`); grad.addColorStop(1, `hsl(${i * 30 + 120} 60% 35%)`);
  g.fillStyle = grad; g.fillRect(0, 0, 384, 288);
  return c.toDataURL("image/jpeg", 0.8).split(",")[1];
}));
await maker.close();
const bufs = pics.map((p) => Buffer.from(p, "base64"));
const served = [];
const server = http.createServer((q, r) => {
  const path = decodeURIComponent(q.url);
  served.push(path);
  const n = Number(path.match(/q(\d+)/)?.[1] ?? 0);
  r.writeHead(200, { "content-type": "image/jpeg", "cache-control": "max-age=3600", "access-control-allow-origin": "*" });
  r.end(bufs[n % bufs.length]);
}).listen(0);
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
await ctx.addInitScript((base) => { window.__picBase = base; }, `http://127.0.0.1:${server.address().port}`);
await ctx.addInitScript(() => {
  const P = (id, t) => ({ id, master: `m${id}`, takenAt: t, addedAt: t, kind: "image", width: 4032, height: 3024, duration: null,
    filename: `IMG_${id}.JPG`, size: 2400000, uti: "public.jpeg", favorite: false, hidden: false, deleted: false, deletedAt: null, subtype: 0, changeTag: null });
  window.__photos = Array.from({ length: 50000 }, (_, i) => P(`q${i}`, 1792000000 - i * 12600));
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
await openScreen(page, "Photos");
await page.locator(".px-tile").first().waitFor();
await page.waitForTimeout(600);
// The stub's own answer made before anything is measured (it walks fifty thousand in script,
// where the app's library answers from SQLite).
const months = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("photos_months").then((m) => m.length));
const years = await page.evaluate(() => new Set(window.__photos.map((p) => new Date(p.takenAt * 1000).getFullYear())).size);

// A tile on the grid, marked: still the same element after Years and back means it was kept.
await page.evaluate(() => { document.querySelector(".px-tile").__kept = true; });
const tilesBefore = await page.locator(".px-tile").count();

/** A click on the switch until its level is drawn and two frames are painted. */
const timed = (name, sel) => page.evaluate(async ([name, sel]) => {
  const t = performance.now();
  [...document.querySelectorAll(".px-levels button")].find((x) => x.textContent === name).click();
  while (!document.querySelector(sel)) await new Promise((r) => requestAnimationFrame(r));
  await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  return performance.now() - t;
}, [name, sel]);

const yearsMs = await timed("Years", '.px-periods[data-level="years"] .px-period');
ok(yearsMs < SWITCH, `Years is up at once on fifty thousand (${Math.round(yearsMs)}ms, two frames included)`);
const yearTiles = await page.locator(".px-periods-grid").getAttribute("data-count").then(Number);
ok(yearTiles === years, `…a tile a year (${yearTiles} for ${years} years, ${await page.locator(".px-period").count()} made)`);
served.length = 0;
const monthsMs = await timed("Months", '.px-periods[data-level="months"] .px-period');
ok(monthsMs < SWITCH, `Months is up at once (${Math.round(monthsMs)}ms)`);
const tiles = await page.locator(".px-periods-grid").getAttribute("data-count").then(Number);
const made = await page.locator(".px-period").count();
ok(tiles === months && made < 60, `…a tile a month, never one a photo, only those near the screen made (${tiles} tiles for ${months} months, ${made} made)`);
await page.waitForTimeout(500);
const fetched = await page.evaluate(() => [...document.querySelectorAll(".px-period img")].filter((i) => i.complete && i.naturalWidth > 0).length);
// The tiles' own pictures (the grid's carry a version mark, "~"), asked for since Months came up.
const asked = served.filter((u) => /fit\d+/.test(u) && !u.includes("~"));
ok(fetched > 0 && asked.length <= made * 4, `…pictures fetched only for the tiles made (${asked.length} asked for, ${made} tiles)`);
ok(asked.every((u) => Number(/fit(\d+)/.exec(u)[1]) <= 1024), `…each at most 1024px, never an original (${[...new Set(asked.map((u) => /fit\d+/.exec(u)[0]))].join(", ")})`);

// All Photos again: back at once, and the grid is the one it was, kept beneath — not built anew.
const allMs = await page.evaluate(async () => {
  const t = performance.now();
  [...document.querySelectorAll(".px-levels button")].find((x) => x.textContent === "All Photos").click();
  while (document.querySelector(".px-periods")) await new Promise((r) => requestAnimationFrame(r));
  await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  return performance.now() - t;
});
ok(allMs < SWITCH, `All Photos is back at once (${Math.round(allMs)}ms)`);
ok(await page.evaluate(() => document.querySelector(".px-tile").__kept === true), `…the very grid it was, kept beneath (${await page.locator(".px-tile").count()} tiles, ${tilesBefore} before)`);
await timed("Months", '.px-periods[data-level="months"] .px-period');

// Flung from top to bottom at 9,000px a second, a step a frame, its pictures decoding as they
// come: no frame held.
const scroll = await page.evaluate(() => new Promise((done) => {
  const el = document.querySelector(".px-periods");
  const gaps = [];
  let last = performance.now();
  const step = (now) => {
    gaps.push(now - last);
    el.scrollTop += (9000 * (now - last)) / 1000;
    last = now;
    if (el.scrollTop + el.clientHeight < el.scrollHeight - 2) requestAnimationFrame(step);
    else {
      gaps.shift();
      gaps.sort((a, b) => a - b);
      done({ frames: gaps.length, p95: gaps[Math.floor(gaps.length * 0.95)], worst: gaps[gaps.length - 1] });
    }
  };
  requestAnimationFrame((t) => { last = t; requestAnimationFrame(step); });
}));
ok(scroll.worst < BOUND, `Months flings end to end without a held frame (${scroll.frames} frames, p95 ${Math.round(scroll.p95)}ms, longest ${Math.round(scroll.worst)}ms, bound ${BOUND})`);

// A month nineteen years down (a year above the last, which cannot scroll to the top): All
// Photos lands exactly on it.
const t = Date.now();
// Read and clicked in one step, on the one element: the list is windowed, and a locator read
// twice re-resolves `nth(-13)` — on a busy machine the tiles still settling after the fling made
// the label one month's and the click another's. Clicked where it is (a pointer click at a point
// lands on whichever tile a scroll brought there).
const label = await page.evaluate(() => {
  const tiles = document.querySelectorAll(".px-period");
  const el = tiles[tiles.length - 13];
  const text = el.querySelector("strong").textContent;
  el.click();
  return text;
});
await page.waitForFunction(() => document.querySelector('.px-levels [aria-pressed="true"]')?.textContent === "All Photos");
const landMs = Date.now() - t;
await page.waitForTimeout(300);
const top = await page.evaluate(() => {
  const el = document.querySelector(".px-scroll");
  const edge = el.getBoundingClientRect().top + parseFloat(getComputedStyle(el).scrollPaddingTop) + 4;
  return [...el.querySelectorAll(".px-month")].find((s) => s.getBoundingClientRect().bottom > edge)?.dataset.month ?? null;
});
ok(top === label, `a month twenty years down opens All Photos exactly there (${top} for ${label}, ${landMs}ms)`);
// Its photos come in where it landed.
await page.waitForFunction((l) => document.querySelector(`.px-month[data-month="${l}"] .px-tile`), label, { timeout: 5000 }).catch(() => {});
ok(await page.locator(`.px-month[data-month="${label}"] .px-tile`).count() > 0, "…and its photos are built there");

await b.close();
server.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
