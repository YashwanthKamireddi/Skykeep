// Photos holds pictures only near the view (#11). Every picture drawn is decoded memory the
// engine keeps while it is there; built 4000px either side and never let go, a scroll through
// twenty thousand photos kept thirteen screens of them (2,300, about 400 MB) and kept them while
// another app was in front. Now: pictures from a screen away, let go two screens away, the ones
// on screen always drawn once a scroll settles, only those on and beside the screen kept while
// another app is in front, and the screen drawn at once on coming back.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
await ctx.addInitScript(() => {
  window.__picBase = "http://localhost:4173/test-pictures";
  const P = (id, t) => ({ id, master: `m${id}`, takenAt: t, addedAt: t, kind: "image", width: 4032, height: 3024, duration: null,
    filename: `IMG_${id}.JPG`, size: 2400000, uti: "public.jpeg", favorite: false, hidden: false, deleted: false, deletedAt: null, subtype: 0, changeTag: null });
  window.__photos = Array.from({ length: 20000 }, (_, i) => P(`q${i}`, 1792600000 - i * 40000));
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
// A small real picture for every request, as the disk cache answers.
const jpeg = Buffer.from(
  "/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAAAAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AVN//2Q==",
  "base64",
);
await page.route("**/test-pictures/**", (route) => route.fulfill({ contentType: "image/jpeg", body: jpeg }));
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
await openScreen(page, "Photos");
await page.locator(".px-tile img[src]").first().waitFor();

// What is drawn, against what is on screen.
const census = () => page.evaluate(() => {
  const view = document.querySelector(".px-scroll").getBoundingClientRect();
  let onScreen = 0, onScreenDrawn = 0, drawn = 0;
  for (const t of document.querySelectorAll(".px-tile")) {
    const r = t.getBoundingClientRect();
    const has = t.querySelector("img")?.hasAttribute("src");
    if (has) drawn++;
    if (r.bottom > view.top && r.top < view.bottom) { onScreen++; if (has) onScreenDrawn++; }
  }
  return { onScreen, onScreenDrawn, drawn, perScreen: onScreen };
});
const first = await census();
ok(first.onScreen > 0 && first.onScreenDrawn === first.onScreen, `the first screen is drawn at once  (${first.onScreenDrawn} of ${first.onScreen})`);

// Down through the whole library a screen at a time, as a steady scroll goes: the most drawn
// at any point, and every stop's screen drawn once it settles.
let most = 0, blank = 0, stops = 0;
const H = await page.locator(".px-scroll").evaluate((el) => el.scrollHeight);
for (let y = 0; y < H; y += 700) {
  await page.locator(".px-scroll").evaluate((el, y) => { el.scrollTop = y; }, y);
  await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
  if (++stops % 25) continue;
  await page.waitForTimeout(120);
  const c = await census();
  most = Math.max(most, c.drawn);
  blank += c.onScreen - c.onScreenDrawn;
}
ok(blank === 0, `every screen it stops on is drawn  (${blank} tiles left blank)`);
// A month at a time (one here is about a screen and a half), let go once the screen is idle: a
// few screens either side, never the 2,300 of before.
ok(most <= first.perScreen * 10, `pictures stay near the view  (at most ${most} drawn; a screen holds ${first.perScreen})`);

// Another app in front: only what was on and beside the screen is kept.
await openScreen(page, "Notes");
await page.waitForTimeout(400);
const away = await page.evaluate(() => document.querySelectorAll(".px-tile img[src]").length);
ok(away <= first.perScreen * 4, `another app in front keeps only the screen and beside it  (${away} pictures)`);
// Back: drawn at once, from what was kept.
await openScreen(page, "Photos");
const again = await census();
ok(again.onScreen > 0 && again.onScreenDrawn === again.onScreen, `coming back, the screen is drawn at once  (${again.onScreenDrawn} of ${again.onScreen})`);
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
