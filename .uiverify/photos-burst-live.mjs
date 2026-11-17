// A burst shows as its chosen photo; in the viewer its BURST mark goes through all of it —
// "Burst · 2 of 3" — and Done (or closing) brings the library back on the chosen one. A Live
// Photo's LIVE mark plays its moving part over the still while the pointer rests on it.
import http from "node:http";
import fs from "node:fs";
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
// The moving part: a short clip from a server of the app's kind.
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
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
await ctx.addInitScript((v) => {
  window.__videoBase = v;
  // A selfie Live Photo's moving part: a quarter turn and a mirror, as the front camera writes it.
  window.__turn = [0, 1, 1, 0];
  // p2 is a burst's chosen photo (of 3), p4 a Live Photo.
  // p3 a video just over an hour long, 0.4 s short of 1:01:00: written "1:01:00", never "60:60".
  window.__photoExtra = { p2: { burst: "B1", burstPick: true, burstCount: 3 }, p3: { duration: 3659.6 }, p4: { kind: "live" } };
}, base);
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const tiles = () => page.locator(".px-tile").count();
const before = await tiles();

// In the grid: the burst is one tile, marked as a stack with how many it holds; the video
// shows its length, whole.
const badge = page.locator('.px-tile[data-id="p2"] .px-burst');
ok((await badge.count()) === 1 && (await badge.innerText()).trim() === "3" && (await badge.locator("svg").count()) === 1, "a burst's tile is marked as a stack of 3");
ok(/burst of 3 photos/.test(await page.locator('.px-tile[data-id="p2"]').getAttribute("aria-label")), "…and says so to a screen reader");
ok((await page.locator(".px-burst").count()) === 1, "no other tile is marked as a burst");
const length = (await page.locator('.px-tile[data-id="p3"] .px-duration').innerText()).trim();
ok(length === "1:01:00", `a video over an hour shows hours, minutes and seconds (${length})`);

await page.locator('.px-tile[data-id="p2"]').dblclick();
await page.waitForTimeout(600);
const burst = page.getByRole("button", { name: /View all 3 photos in this burst/ });
ok((await burst.count()) === 1 && /BURST/.test(await burst.innerText()), "a burst's chosen photo is marked BURST, with how many it holds");
await burst.click();
await page.waitForTimeout(400);
ok(/Burst · 2 of 3/.test(await page.locator(".px-mark.is-burst").innerText()), "BURST goes through the burst: the chosen one is 2 of 3");
await page.keyboard.press("ArrowLeft");
await page.waitForTimeout(250);
ok(/1 of 3/.test(await page.locator(".px-mark.is-burst").innerText()) && /p2-s0/.test(decodeURIComponent(await page.locator(".px-stage .px-shown").getAttribute("src"))),
  "the arrows step through its shots");
await page.getByRole("button", { name: "Done" }).click();
await page.waitForTimeout(500);
ok((await page.locator(".px-mark.is-burst").count()) === 0 && /p2/.test(decodeURIComponent(await page.locator(".px-stage .px-shown").getAttribute("src"))) && !/p2-s/.test(decodeURIComponent(await page.locator(".px-stage .px-shown").getAttribute("src"))),
  "Done brings the library back, on the chosen photo");
// Into the burst again, then closing the viewer: the library is whole again behind it.
await page.getByRole("button", { name: /View all 3 photos/ }).click();
await page.waitForTimeout(300);
await page.keyboard.press("Escape");
await page.waitForTimeout(900);
ok((await page.locator(".px-viewer").count()) === 0 && (await tiles()) === before && (await page.locator('.px-tile[data-id^="p2-s"]').count()) === 0,
  `closing inside a burst leaves the library as it was  (${await tiles()} tiles)`);

// A Live Photo: LIVE, and resting on it plays the moving part over the still.
await page.locator('.px-tile[data-id="p4"]').dblclick();
await page.waitForTimeout(600);
const live = page.getByRole("button", { name: "Play Live Photo" });
ok((await live.count()) === 1 && /LIVE/.test(await live.innerText()), "a Live Photo is marked LIVE");
const place = await page.evaluate(() => {
  const m = document.querySelector(".px-mark").getBoundingClientRect(), p = document.querySelector(".px-pan").getBoundingClientRect();
  return { dx: Math.round(m.left - p.left), dy: Math.round(m.top - p.top) };
});
ok(place.dx >= 4 && place.dx <= 20 && place.dy >= 4 && place.dy <= 20, `LIVE sits on the photo's own top-left corner  (${JSON.stringify(place)} from it)`);
await live.hover();
await page.waitForTimeout(200);
const v = page.locator(".px-pan video.px-live");
ok((await v.count()) === 1 && (await v.getAttribute("src")) === `${base}/p4/video`, `resting on LIVE plays its moving part over the still  ${await v.getAttribute("src").catch(() => "")}`);
// Turned as its file records it (a selfie's quarter turn and mirror), in the still's own box:
// the film laid out in its own shape and turned into the photo's, exactly over the still.
const asked = await page.evaluate(() => window.__turnAsks?.at(-1));
ok(asked?.id === "p4" && asked?.which === "video", `its turn is asked for the moving part itself  ${JSON.stringify(asked)}`);
const fit = await page.evaluate(() => {
  const v = document.querySelector("video.px-live"), p = document.querySelector(".px-pan").getBoundingClientRect(), r = v.getBoundingClientRect();
  return { t: getComputedStyle(v).transform, vis: getComputedStyle(v).visibility, dw: Math.round(r.width - p.width), dh: Math.round(r.height - p.height), dx: Math.round(r.left - p.left), dy: Math.round(r.top - p.top) };
});
ok(fit.t.replace(/\s/g, "") === "matrix(0,1,1,0,0,0)" && fit.vis === "visible", `turned by the file's own matrix  ${fit.t}`);
ok(Math.abs(fit.dw) <= 1 && Math.abs(fit.dh) <= 1 && Math.abs(fit.dx) <= 1 && Math.abs(fit.dy) <= 1, `and lying exactly over the still  ${JSON.stringify(fit)}`);
await page.keyboard.press("z");
await page.waitForTimeout(400);
ok((await page.getByRole("button", { name: "Play Live Photo" }).count()) === 0, "zoomed in, the mark is not over the picture");
await page.keyboard.press("z");
await page.waitForTimeout(400);
await page.keyboard.press("ArrowRight");
await page.waitForTimeout(250);
ok((await page.locator("video.px-live").count()) === 0, "and the moving part stops when the photo changes");
await b.close();
server.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
