// What the window's chrome costs, frame by frame: the time between frames, as the engine gives
// them, while a person does the things that make glass and motion work hardest — scrolling the
// long lists under the toolbars (Photos, Notes, Drive), opening menus over them, sliding a
// segmented control and switching apps. Prints one line per action (p50, p95, longest, how
// many frames ran late) so a change to the material or the motion can be compared before and
// after, in each engine; and checks the rules the chrome keeps everywhere:
//
//   - no more than a few live blurs on screen at once, none inside anything that scrolls and
//     none on top of another (glass on glass), and none at all in WebKitGTK, which blurs what
//     is behind again on every frame of a scroll (tokens.css "Liquid Glass");
//   - nothing freezes: no frame held past the bound below.
//
//   ENGINE=webkit node .uiverify/glass-frames.mjs
//   ENGINE=gtk    node .uiverify/glass-frames.mjs   (on a headless display only)
//   JSON=1 …      prints the numbers as JSON too, for a table
//   GLASS=live …  the blurred tier in WebKit too, to measure what it would cost there
import http from "node:http";
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const BASE = process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

// Photo-like pictures from the engine's own canvas, served from here (as photos-scroll does).
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
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
  notesFixture: true, calendarFixture: true, contactsFixture: true, driveFixture: true, remindersFixture: true });
await ctx.addInitScript((base) => {
  window.__picBase = base;
  const P = (i, t) => ({ id: `q${i}`, master: `mq${i}`, takenAt: t, addedAt: t, kind: "image", width: 4032, height: 3024,
    duration: null, filename: `IMG_${i}.JPG`, size: 2400000, uti: "public.jpeg", favorite: i % 9 === 0, hidden: false,
    deleted: false, deletedAt: null, subtype: 0, changeTag: null });
  window.__photos = Array.from({ length: 2000 }, (_, i) => P(i, 1792600000 - i * 20000));
  // Long lists: 400 notes and 400 files, invented.
  const words = ["Garden plan", "Trip to the coast", "Recipes", "Book list", "Meeting with Sam", "Gift ideas", "Running log", "Maya's birthday"];
  window.__notesExtra = Array.from({ length: 400 }, (_, i) => ({ guid: `x${i}`, folderGuid: "f1", title: `${words[i % words.length]} ${i}`,
    snippet: "A few lines about it, to read in the list.", body: `${words[i % words.length]} ${i}\nA few lines about it, to read in the list.`,
    locked: false, editable: true, pinned: false, createdAt: null, modifiedAt: 1788400000 - i * 600, etag: null }));
  window.__driveExtra = Array.from({ length: 400 }, (_, i) => ({ drivewsid: `FILE::x${i}`, parentId: null, name: `${words[i % words.length]} ${i}`,
    extension: ["pdf", "pages", "png", "txt"][i % 4], kind: "FILE", size: 20000 + i * 911, childCount: null, createdAt: 1787000000,
    modifiedAt: 1788000000 - i * 600, etag: null }));
}, picBase);
// GLASS=live: the blurred tier even in WebKit — to measure what it would cost there.
if (process.env.GLASS === "live") await ctx.addInitScript(() => {
  new MutationObserver(() => { if (document.documentElement?.dataset.engine) delete document.documentElement.dataset.engine; })
    .observe(document, { subtree: true, attributes: true, attributeFilter: ["data-engine"] });
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); failures++; });
await page.goto(BASE, { waitUntil: "networkidle" });
await page.locator("main.content[data-screen]").first().waitFor();
await page.waitForFunction(() => ["NotesView", "DriveView", "PhotosView"]
  .every((n) => performance.getEntriesByType("resource").some((r) => r.name.includes(`/${n}-`))), null, { timeout: 20000 });

const stats = (gaps) => {
  const s = [...gaps].sort((x, y) => x - y);
  const q = (p) => s[Math.min(s.length - 1, Math.floor(p * s.length))];
  return { frames: s.length, p50: q(0.5), p95: q(0.95), max: s[s.length - 1], late: gaps.filter((g) => g > 25).length };
};
const results = {};
// Headless WebKit paints in software, every frame on the CPU; its bound is looser. A freeze is
// what fails here — a slower frame is for the numbers, compared before and after a change.
const BOUND = engineName === "chromium" ? 250 : 900;
const report = (what, gaps) => {
  const st = stats(gaps);
  results[what] = st;
  console.log(`  ${what}: ${st.frames} frames, p50 ${st.p50.toFixed(1)} ms, p95 ${st.p95.toFixed(1)} ms, max ${st.max.toFixed(1)} ms, ${st.late} late (>25 ms)`);
  ok(st.max < BOUND, `${engineName} ${what}: never freezes (longest frame ${Math.round(st.max)} ms, bound ${BOUND})`);
};

/** Scroll `sel` at `speed` px/s for `ms`, one step a frame; the gaps between frames. */
const scroll = (sel, speed, ms) => page.evaluate(([sel, speed, ms]) => new Promise((done) => {
  const el = document.querySelector(sel);
  el.scrollTop = 0;
  const gaps = [];
  requestAnimationFrame((t0) => {
    let last = t0;
    const step = (now) => {
      gaps.push(now - last);
      el.scrollTop += (speed * (now - last)) / 1000;
      last = now;
      if (now - t0 < ms && el.scrollTop + el.clientHeight < el.scrollHeight - 2) requestAnimationFrame(step);
      else done(gaps.slice(2));
    };
    requestAnimationFrame(step);
  });
}), [sel, speed, ms]);

/** Run `act` in the page and gather the gaps of the `n` frames after it. */
const framesAfter = (act, n = 10) => page.evaluate(async ([act, n]) => {
  await new Promise((r) => requestAnimationFrame(r));
  let last = performance.now();
  (0, eval)(act);
  const gaps = [];
  for (let i = 0; i < n; i++) {
    const now = await new Promise((r) => requestAnimationFrame(r));
    gaps.push(now - last);
    last = now;
  }
  return gaps;
}, [act, n]);

/** The live blurs on screen: how many, and whether any sits in a scroller or on another. */
const blurs = () => page.evaluate(() => {
  const live = [...document.querySelectorAll("body *")].filter((el) => {
    const cs = getComputedStyle(el);
    const f = cs.backdropFilter || cs.webkitBackdropFilter || "none";
    if (f === "none") return false;
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0 && cs.visibility !== "hidden";
  });
  const scrolls = (el) => { for (let p = el.parentElement; p; p = p.parentElement) {
    const o = getComputedStyle(p).overflowY; if ((o === "auto" || o === "scroll") && p.scrollHeight > p.clientHeight) return true; } return false; };
  return {
    count: live.length,
    area: Math.round(live.reduce((a, el) => { const r = el.getBoundingClientRect(); return a + r.width * r.height; }, 0) / (innerWidth * innerHeight) * 100),
    inScroller: live.filter(scrolls).map((el) => el.className).slice(0, 3),
    nested: live.filter((el) => live.some((o) => o !== el && o.contains(el))).map((el) => el.className).slice(0, 3),
  };
});
const gtkLike = await page.evaluate(() => document.documentElement.dataset.engine === "gtk");
const checkBlurs = async (where) => {
  const bl = await blurs();
  ok(bl.count <= 4 && bl.area <= 25, `${where}: ${bl.count} live blur(s), ${bl.area}% of the window (at most 4, 25%)`);
  ok(bl.inScroller.length === 0, `${where}: no blur inside a scroller ${bl.inScroller.join(", ")}`);
  ok(bl.nested.length === 0, `${where}: no blur on a blur ${bl.nested.join(", ")}`);
  if (gtkLike) ok(bl.count === 0, `${where}: WebKitGTK draws no live blur`);
};

// ---- Photos ----------------------------------------------------------------------------------
await openScreen(page, "Photos");
await page.locator(".px-tile").first().waitFor();
await page.waitForTimeout(800);
await checkBlurs("Photos");
report("scrolling Photos", await scroll(".px-scroll", 3000, 3000));

// ---- Notes ----------------------------------------------------------------------------------
await openScreen(page, "Notes");
await page.locator(".note-row").nth(40).waitFor({ state: "attached" });
await page.waitForTimeout(400);
await checkBlurs("Notes");
report("scrolling Notes' list", await scroll(".note-rows", 2000, 2500));

// ---- Drive ----------------------------------------------------------------------------------
await openScreen(page, "Drive");
await page.locator(".drive-row").nth(40).waitFor({ state: "attached" });
await page.waitForTimeout(400);
await checkBlurs("Drive");
report("scrolling Drive's list", await scroll(".drive-rows", 2000, 2500));
await page.evaluate(() => { document.querySelector(".drive-rows").scrollTop = 0; });
await page.waitForTimeout(200);
const menuGaps = [];
for (let i = 0; i < 8; i++) {
  menuGaps.push(...await framesAfter(`(() => { const r = document.querySelectorAll(".drive-row")[${4 + i}]; const b = r.getBoundingClientRect();
    r.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: b.left + 60, clientY: b.top + b.height / 2, button: 2 })); })()`, 10));
  ok(await page.locator(".row-menu").isVisible(), `${engineName} the row's menu opens (${i + 1})`);
  if (i === 0) await checkBlurs("Drive, a menu open");
  await page.keyboard.press("Escape");
  await page.locator(".row-menu").waitFor({ state: "detached" });
  await page.waitForTimeout(120);
}
report("opening a menu", menuGaps);

// ---- Calendar's view switch: the segmented control's thumb ---------------------------------
await openScreen(page, "Calendar");
await page.locator(".cx").first().waitFor();
await page.waitForTimeout(400);
const thumbGaps = [];
for (let i = 0; i < 8; i++) {
  thumbGaps.push(...await framesAfter(`document.querySelectorAll('.segmented[aria-label="View"] .segment')[${i % 2 ? 1 : 2}].click()`, 14));
  await page.waitForTimeout(200);
}
report("sliding a segmented control", thumbGaps);

if (process.env.JSON) console.log(JSON.stringify({ engine: engineName, results }));
await b.close();
server.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
