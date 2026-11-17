// The slideshow, as Photos plays one: each photo whole on black, dissolving into the next with
// no black or empty frame between them; the next made the size of the screen and decoded before
// it shows; a video shows its picture, then plays, then the show moves on; Space, the arrows and
// Escape; reduced motion cuts.
//
// The proof of "no black frame" is taken from the screen itself: during each dissolve the page
// is captured frame after frame and the middle of the screen — where both photos lie — must
// always show a photo's colour (or a blend of the two), never black.
import http from "node:http";
import fs from "node:fs";
import { createRequire } from "node:module";
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const { PNG } = createRequire(import.meta.url)("playwright-core/lib/utilsBundle");

const clip = new URL("./clip.webm", import.meta.url);
const len = fs.statSync(clip).size;
const films = [];
const server = http.createServer((req, res) => {
  films.push(req.url);
  const m = /bytes=(\d*)-(\d*)/.exec(req.headers.range ?? "");
  const f = m && m[1] ? +m[1] : 0, l = m && m[2] ? Math.min(+m[2], len - 1) : len - 1;
  res.writeHead(m ? 206 : 200, { "Content-Type": "video/webm", "Accept-Ranges": "bytes", "Content-Length": l - f + 1, "Content-Range": `bytes ${f}-${l}/${len}` });
  fs.createReadStream(clip, { start: f, end: l }).pipe(res);
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const base = `http://127.0.0.1:${server.address().port}/s3cret`;

// Every photo a flat colour of its own, at the size asked for (4:3, as the stub's photos are).
const COLOURS = { p1: [220, 40, 40], p2: [40, 180, 60], p3: [40, 80, 220], p4: [230, 200, 30], p5: [200, 60, 200] };
const asked = [];
const PICS = "http://pics.test";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

async function open(reduced) {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 800 }, reducedMotion: reduced ? "reduce" : "no-preference" });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
  await ctx.addInitScript(([v, pics]) => { window.__videoBase = v; window.__picBase = pics; }, [base, PICS]);
  await ctx.route(`${PICS}/**`, async (route) => {
    const path = decodeURIComponent(route.request().url().slice(PICS.length + 1));
    asked.push(path);
    const [, id, size] = /pic\/([^/~]+)(?:~[0-9a-z]+)?\/(\w+)/.exec(path) ?? [];
    const c = COLOURS[id] ?? [128, 128, 128];
    const long = /^fit(\d+)/.test(size) ? +size.slice(3) : 512;
    // A slow picture now and then, as one made from the network is: the show waits for it.
    if (id === "p4" && size.startsWith("fit") && long > 1000) await new Promise((r) => setTimeout(r, 700));
    const w = long, h = Math.round((long * 3) / 4);
    await route.fulfill({ contentType: "image/svg+xml", body: `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}"><rect width="${w}" height="${h}" fill="rgb(${c.join(",")})"/></svg>` });
  });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
  await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await openScreen(page, "Photos");
  await page.waitForTimeout(400);
  await page.locator('.px-tile[data-id="p1"]').dispatchEvent("dblclick");
  await page.waitForTimeout(500);
  await page.getByRole("button", { name: "Start Slideshow" }).click();
  return { ctx, page };
}

/** The colour in the middle of the screen, captured as composited. */
async function middle(page) {
  const png = PNG.sync.read(await page.screenshot({ clip: { x: 620, y: 380, width: 40, height: 40 } }));
  let r = 0, g = 0, bl = 0;
  for (let i = 0; i < png.data.length; i += 4) { r += png.data[i]; g += png.data[i + 1]; bl += png.data[i + 2]; }
  const n = png.data.length / 4;
  return [r / n, g / n, bl / n];
}
const count = (page) => page.locator(".px-show-count").innerText();

const { ctx, page } = await open(false);
await page.waitForFunction(() => document.querySelector(".px-show-count")?.textContent?.startsWith("1 of"), null, { timeout: 5000 });
await page.waitForTimeout(300);
// Two layers, made once: marked now, and the same two elements must still be there later.
await page.evaluate(() => document.querySelectorAll(".px-slide").forEach((l, i) => (l.dataset.mark = String(i))));
ok((await page.locator(".px-slide").count()) === 2, "two layers take turns");
const first = await page.evaluate(() => {
  const l = [...document.querySelectorAll(".px-slide")].find((x) => x.style.opacity === "1");
  const img = l?.querySelector("img");
  return { src: decodeURIComponent(img?.src ?? ""), w: img?.naturalWidth, dpr: devicePixelRatio, W: innerWidth, H: innerHeight };
});
const step = +(/fit(\d+)/.exec(first.src)?.[1] ?? 0);
ok(/p1(~[0-9a-z]+)?\/fit/.test(first.src) && step >= Math.min(first.W, (first.H * 4) / 3) * first.dpr, `the photo is made the size of the screen, never smaller  (${first.src.split("/").pop()} for ${first.W}×${first.H} at ${first.dpr}×)`);

// Each frame of the dissolve into the next photo, as the page sets it: the layers' opacities
// and whether the picture of each is decoded. The screen behind is never left uncovered.
const watch = () => page.evaluate(() => {
  window.__frames = [];
  const tick = () => {
    const ls = [...document.querySelectorAll(".px-slide")].map((l) => ({ o: +l.style.opacity, z: +l.style.zIndex, ready: l.querySelector("img").complete && l.querySelector("img").naturalWidth > 0 }));
    window.__frames.push(ls);
    if (window.__frames.length < 600) requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
});
await watch();
// Captured from the screen while it changes: never black in the middle, where both photos are.
const shots = [];
const t0 = Date.now();
while (Date.now() - t0 < 5200 && (await count(page)).startsWith("1 of")) shots.push(await middle(page));
for (let i = 0; i < 8; i++) shots.push(await middle(page));
const black = shots.filter(([r, g, bl]) => r + g + bl < 60);
ok(shots.length > 8 && black.length === 0, `no black frame on the screen as it moves to the next  (${shots.length} frames captured, ${black.length} black)`);
ok(/^2 of/.test(await count(page)), "on to the next by itself");
const frames = await page.evaluate(() => window.__frames);
const covered = frames.every((ls) => {
  const shown = ls.filter((l) => l.o > 0);
  // Whatever shows is decoded, and together the layers cover the screen: never a dip.
  const cover = 1 - shown.reduce((k, l) => k * (1 - l.o), 1);
  return shown.every((l) => l.ready) && cover >= 0.9;
});
const mid = frames.filter((ls) => ls.some((l) => l.o > 0.05 && l.o < 0.95)).length;
ok(covered, `every frame of the dissolve shows decoded pictures covering the screen  (${frames.length} frames)`);
ok(mid >= 5, `and it dissolves over several frames, not a cut  (${mid} in between)`);
ok(await page.evaluate(() => [...document.querySelectorAll(".px-slide")].every((l) => l.dataset.mark !== undefined)), "the same two layers, never torn down and made again");

// The video: its picture first, then the film from the app's server over it, then on.
await page.waitForFunction(() => document.querySelector(".px-show-count")?.textContent?.startsWith("3 of"), null, { timeout: 9000 });
const film = await page.waitForFunction(() => {
  const v = [...document.querySelectorAll(".px-slide video")].find((x) => x.getAttribute("src"));
  return v && { src: v.getAttribute("src"), o: v.style.opacity, t: v.currentTime, poster: decodeURIComponent(v.closest(".px-slide").querySelector("img").src) };
}, null, { timeout: 5000 }).then((h) => h.jsonValue());
ok(film.src === `${base}/p3/video` && /p3(~[0-9a-z]+)?\/fit/.test(film.poster), `a video shows its picture and plays its film from the app's server  ${film.src}`);
const shownFilm = await page.waitForFunction(() => {
  const v = [...document.querySelectorAll(".px-slide video")].find((x) => x.getAttribute("src"));
  return v && v.style.opacity === "1" && v.currentTime > 0 ? v.currentTime : null;
}, null, { timeout: 6000 }).then((h) => h.jsonValue()).catch(() => null);
if (shownFilm === null && engineName !== "chromium") {
  // An engine with no decoder for the clip: its picture stands its time instead.
  ok(true, "this engine cannot play the clip: its picture stands in");
} else ok(shownFilm > 0, `the film is drawn over its picture only once it is playing  (at ${shownFilm?.toFixed?.(2)}s)`);
await page.waitForFunction(() => document.querySelector(".px-show-count")?.textContent?.startsWith("4 of"), null, { timeout: 12000 });
ok(true, "when the film ends the show moves on");

// The keys: Space pauses (it stays), the arrows step, Escape ends.
await page.keyboard.press(" ");
await page.waitForTimeout(300);
ok((await page.getByRole("button", { name: "Play" }).count()) === 1, "Space pauses it");
const held = await count(page);
await page.waitForTimeout(4600);
ok((await count(page)) === held, `paused, it stays  (${held})`);
await page.keyboard.press("ArrowRight");
await page.waitForFunction(() => document.querySelector(".px-show-count")?.textContent?.startsWith("5 of"), null, { timeout: 4000 });
ok(true, "→ steps to the next");
await page.keyboard.press("ArrowLeft");
await page.waitForFunction(() => document.querySelector(".px-show-count")?.textContent?.startsWith("4 of"), null, { timeout: 4000 });
ok(true, "← steps back");
await page.keyboard.press("Escape");
await page.waitForTimeout(300);
ok((await page.locator(".px-show").count()) === 0, "Escape ends it");
await ctx.close();

// Reduced motion: the change is a cut — no frame in between.
const r = await open(true);
await r.page.waitForFunction(() => document.querySelector(".px-show-count")?.textContent?.startsWith("1 of"), null, { timeout: 5000 });
await r.page.evaluate(() => {
  window.__frames = [];
  const tick = () => { window.__frames.push([...document.querySelectorAll(".px-slide")].map((l) => +l.style.opacity)); if (window.__frames.length < 400) requestAnimationFrame(tick); };
  requestAnimationFrame(tick);
});
await r.page.keyboard.press("ArrowRight");
await r.page.waitForFunction(() => document.querySelector(".px-show-count")?.textContent?.startsWith("2 of"), null, { timeout: 4000 });
await r.page.waitForTimeout(300);
const between = await r.page.evaluate(() => window.__frames.filter((f) => f.some((o) => o > 0.02 && o < 0.98)).length);
ok(between === 0, `with reduced motion, a cut  (${between} frames in between)`);
await r.ctx.close();

await b.close();
server.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
