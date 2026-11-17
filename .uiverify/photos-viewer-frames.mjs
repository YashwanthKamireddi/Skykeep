// The viewer never shows a black or empty frame: opening a photo, stepping to the next and back
// (photos and a video among them), and a Live Photo playing and ending. Each is captured from
// the screen frame after frame; the middle of the stage must always show the photo's colour —
// its tile's picture, the one before it, or itself — never the stage's bare backdrop or black.
//
// And a Live Photo's moving part lies exactly over its still: the still upright in a box of its
// own shape (Apple records the sensor's sides, 4032 × 3024, for a photo shown portrait), the film
// turned a quarter as its file says, and drawn larger by its margin (clean aperture) so only what
// the still shows is seen — clipped to the still's box, never a smaller box inside it.
import http from "node:http";
import fs from "node:fs";
import { createRequire } from "node:module";
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const { PNG } = createRequire(import.meta.url)("playwright-core/lib/utilsBundle");

const clip = new URL("./clip.webm", import.meta.url);
const len = fs.statSync(clip).size;
const server = http.createServer((req, res) => {
  const m = /bytes=(\d*)-(\d*)/.exec(req.headers.range ?? "");
  const f = m && m[1] ? +m[1] : 0, l = m && m[2] ? Math.min(+m[2], len - 1) : len - 1;
  res.writeHead(m ? 206 : 200, { "Content-Type": "video/webm", "Accept-Ranges": "bytes", "Content-Length": l - f + 1, "Content-Range": `bytes ${f}-${l}/${len}` });
  fs.createReadStream(clip, { start: f, end: l }).pipe(res);
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const base = `http://127.0.0.1:${server.address().port}/s3cret`;

const COLOURS = { p1: [220, 40, 40], p2: [40, 180, 60], p3: [40, 80, 220], p4: [230, 200, 30], p5: [200, 60, 200] };
const PICS = "http://pics.test";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { photosNewestFirst: true, screen: "library", services: SERVICES, twoFactor: null });
await ctx.addInitScript(([v, pics]) => {
  window.__videoBase = v;
  window.__picBase = pics;
  // p4: a Live Photo shown portrait though Apple records it 4032 × 3024; its film a quarter turn
  // with a margin round it, as his are.
  window.__photoExtra = { p4: { kind: "live" } };
  window.__turn = [0, 1, -1, 0];
  window.__zoom = 960 / 872;
}, [base, PICS]);
await ctx.route(`${PICS}/**`, async (route) => {
  const path = decodeURIComponent(route.request().url().slice(PICS.length + 1));
  const [, id, size] = /pic\/([^/~]+)(?:~[0-9a-z]+)?\/(\w+)/.exec(path) ?? [];
  const c = COLOURS[id] ?? [128, 128, 128];
  const long = /^fit(\d+)/.test(size) ? +size.slice(3) : 512;
  // The viewer's pictures are slow to come, as ones made from the network are: what stands in
  // meanwhile is what this proves.
  if (/^fit/.test(size) && long > 700) await new Promise((r) => setTimeout(r, 400));
  const portrait = id === "p4";
  const [w, h] = portrait ? [Math.round((long * 3) / 4), long] : [long, Math.round((long * 3) / 4)];
  await route.fulfill({ contentType: "image/svg+xml", body: `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}"><rect width="${w}" height="${h}" fill="rgb(${c.join(",")})"/></svg>` });
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(800);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

const backdrop = async () => page.evaluate(() => {
  const m = /\d+(\.\d+)?/g;
  return (getComputedStyle(document.querySelector(".px-backdrop") ?? document.body).backgroundColor.match(m) ?? []).slice(0, 3).map(Number);
});
const near = (a, c, d = 40) => Math.abs(a[0] - c[0]) + Math.abs(a[1] - c[1]) + Math.abs(a[2] - c[2]) < d;
/** Frames of the stage's middle while `act` runs and for `ms` after. */
async function capture(act, ms) {
  const frames = [];
  const box = await page.evaluate(() => {
    const s = document.querySelector(".px-stage")?.getBoundingClientRect();
    return s ? { x: s.left + s.width / 2 - 15, y: s.top + s.height / 2 - 15 } : { x: 625, y: 385 };
  });
  const run = act();
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    const png = PNG.sync.read(await page.screenshot({ clip: { x: box.x, y: box.y, width: 30, height: 30 } }));
    let r = 0, g = 0, bl = 0;
    for (let i = 0; i < png.data.length; i += 4) { r += png.data[i]; g += png.data[i + 1]; bl += png.data[i + 2]; }
    const n = png.data.length / 4;
    frames.push([r / n, g / n, bl / n]);
  }
  await run;
  return frames;
}
const judge = (frames, back, what) => {
  const black = frames.filter((c) => c[0] + c[1] + c[2] < 45).length;
  const bare = frames.filter((c) => near(c, back, 12)).length;
  ok(frames.length >= 5 && black === 0 && bare === 0, `${what}: no black or empty frame  (${frames.length} frames, ${black} black, ${bare} bare backdrop)`);
};

// Opening: the tile's own picture stands in until the sharp one is decoded.
let frames = await capture(() => page.locator('.px-tile[data-id="p1"]').dispatchEvent("dblclick"), 1400);
const back = await backdrop();
// The first frames may be the grid itself (the click not yet taken); judged from the viewer on.
const opened = frames.findIndex((c) => near(c, COLOURS.p1, 60));
judge(frames.slice(Math.max(0, opened)), back, "opening a photo");
// Stepping on and back, a video among them.
for (const [key, label] of [["ArrowRight", "to the next photo"], ["ArrowRight", "to a video"], ["ArrowRight", "from a video to a photo"], ["ArrowLeft", "back to the video"], ["ArrowLeft", "back to a photo"]]) {
  frames = await capture(() => page.keyboard.press(key), 1100);
  judge(frames, back, `stepping ${label}`);
}
// The video's own picture stands until a frame of the film is here; the film is never black.
await page.evaluate(() => {
  // Each frame from the step on: whether the film is drawn yet, and whether its picture is there.
  window.__seen = [];
  const tick = () => {
    const v = document.querySelector(".px-stage video");
    if (v) window.__seen.push({ o: v.style.opacity, t: v.currentTime, poster: !!document.querySelector(".px-player-poster") && decodeURIComponent(document.querySelector(".px-player-poster").src).match(/p3(~[0-9a-z]+)?\/fit/) !== null });
    if (window.__seen.length < 120) requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
});
await page.keyboard.press("ArrowRight");
await page.waitForTimeout(1500);
const seen = await page.evaluate(() => window.__seen);
const poster = { first: seen[0], beforeFilm: seen.filter((f) => f.o !== "1").every((f) => f.poster), filmOnlyPlaying: seen.filter((f) => f.o === "1").every((f) => f.t > 0) };
ok(seen.length > 0 && poster.first?.poster && poster.beforeFilm && poster.filmOnlyPlaying, `a video shows its own picture until a frame of its film is here  ${JSON.stringify(poster)}`);

// The Live Photo.
await page.keyboard.press("Escape");
await page.waitForTimeout(500);
await page.locator('.px-tile[data-id="p4"]').dispatchEvent("dblclick");
await page.waitForTimeout(1500);
const live = page.getByRole("button", { name: "Play Live Photo" });
frames = await capture(() => live.click(), 2500);
judge(frames, back, "playing a Live Photo");
const fit = await page.evaluate(() => {
  const still = document.querySelector(".px-pan .px-shown").getBoundingClientRect();
  const frame = document.querySelector(".px-live-frame")?.getBoundingClientRect();
  const v = document.querySelector("video.px-live");
  const r = v?.getBoundingClientRect();
  return v && { still: [still.left, still.top, still.width, still.height].map(Math.round), frame: [frame.left, frame.top, frame.width, frame.height].map(Math.round), film: [r.left, r.top, r.width, r.height].map(Math.round), t: getComputedStyle(v).transform };
});
ok(fit && fit.still[3] > fit.still[2], `the still shows upright, portrait, though Apple records it landscape  ${JSON.stringify(fit?.still)}`);
ok(fit && fit.frame.every((x, i) => Math.abs(x - fit.still[i]) <= 1), `the moving part is clipped to the still's own box  ${JSON.stringify(fit?.frame)}`);
const z = 960 / 872;
ok(fit && Math.abs(fit.film[2] - fit.still[2] * z) <= 2 && Math.abs(fit.film[3] - fit.still[3] * z) <= 2
  && Math.abs(fit.film[0] + fit.film[2] / 2 - (fit.still[0] + fit.still[2] / 2)) <= 1 && Math.abs(fit.film[1] + fit.film[3] / 2 - (fit.still[1] + fit.still[3] / 2)) <= 1,
  `turned and drawn its margin larger, centred on the still — never a smaller box  (film ${JSON.stringify(fit?.film)}, ${fit?.t})`);
// It ends: the still again, with nothing in between.
frames = await capture(() => page.waitForSelector("video.px-live", { state: "detached", timeout: 9000 }), 600);
judge(frames, back, "a Live Photo ending");

await b.close();
server.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
