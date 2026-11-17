// A slideshow started from the viewer has the screen to itself: a film open behind it stops
// playing, and the viewer's own keys (F favourites, H hides, Delete deletes) do nothing to the
// photo behind while the show is up — they used to act on it, unseen.
//
//   URL=http://localhost:4203 ENGINE=webkit node .uiverify/photos-show-over.mjs
import http from "node:http";
import fs from "node:fs";
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };
// The app's loopback video server, as photos-video.mjs plays it.
const clip = new URL("./clip.webm", import.meta.url);
const len = fs.statSync(clip).size;
const server = http.createServer((req, res) => {
  if (!req.url.startsWith("/v/p3/")) return void res.writeHead(404).end();
  const m = /bytes=(\d*)-(\d*)/.exec(req.headers.range ?? "");
  const first = m && m[1] ? +m[1] : 0;
  const last = m && m[2] ? Math.min(+m[2], len - 1) : len - 1;
  res.writeHead(m ? 206 : 200, { "Content-Type": "video/webm", "Accept-Ranges": "bytes", "Content-Length": last - first + 1, "Content-Range": `bytes ${first}-${last}/${len}` });
  fs.createReadStream(clip, { start: first, end: last }).pipe(res);
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
await ctx.addInitScript((v) => { window.__videoBase = v; }, `http://127.0.0.1:${server.address().port}/v`);
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(500);
await page.locator('.px-tile[data-id="p2"]').dblclick();
await page.waitForTimeout(300);
await page.getByRole("button", { name: "Start Slideshow" }).click();
await page.locator(".px-show").waitFor();
await page.waitForTimeout(200);
for (const key of ["f", "h", "Delete", "i"]) await page.keyboard.press(key);
await page.waitForTimeout(300);
const flags = await page.evaluate(() => window.__flags ?? []);
ok(flags.length === 0, `keys pressed during the show change nothing behind it  ${JSON.stringify(flags)}`);
ok((await page.locator(".confirm, [role=alertdialog]").count()) === 0, "and ask nothing");
ok((await page.locator(".px-show").count()) === 1, "the show is still up");
await page.keyboard.press("Escape");
await page.waitForTimeout(200);
ok((await page.locator(".px-show").count()) === 0 && (await page.locator(".px-viewer").count()) === 1, "Escape ends the show, back to the viewer");

// A film open in the viewer is stopped by a show started over it.
await page.reload({ waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(500);
await page.locator('.px-tile[data-id="p3"]').dblclick();
await page.waitForTimeout(800);
const film = page.locator(".px-viewer video");
if ((await film.count()) === 0) ok(false, "the film opens in the viewer");
else {
  await film.first().evaluate((v) => { v.muted = true; return v.play().catch(() => {}); });
  await page.waitForTimeout(200);
  ok(!(await film.first().evaluate((v) => v.paused)), "the film plays in the viewer");
  await page.getByRole("button", { name: "Start Slideshow" }).click();
  await page.locator(".px-show").waitFor();
  await page.waitForTimeout(300);
  ok(await film.first().evaluate((v) => v.paused), "and is stopped when a show starts over it");
}
await b.close();
server.close();
console.log(failures ? `\n${failures} FAILED` : "\nall passed");
process.exit(failures ? 1 : 0);
