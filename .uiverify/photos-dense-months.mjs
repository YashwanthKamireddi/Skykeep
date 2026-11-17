// A large library whose months each hold more photos than the store keeps (a Screenshots
// library: thousands a month), its pages a little slow as over Windows' IPC. Every tile on
// screen keeps its picture: on opening, after a sync lands, after the scrollbar is dragged far
// down and back. Until 0.1.27 the store let go of the photos on screen as the month's own pages
// came in, by distance from where the last month asked, and nothing asked for them again:
// grey boxes that could not be clicked (#84, discussion #72).
//
//   URL=http://localhost:4173 node .uiverify/photos-dense-months.mjs   (ENGINE=webkit for WebKit)
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
import { status } from "./arrival.mjs";

const N = 37_765;
const GAP = 600; // seconds between photos: about 4,300 a month, more than the store keeps
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };

const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
await ctx.addInitScript(([n, gap]) => {
  const P = (id, t) => ({ id, master: `m${id}`, takenAt: t, addedAt: t, kind: "image", width: 4032, height: 3024, duration: null,
    filename: `IMG_${id}.JPG`, size: 2400000, uti: "public.jpeg", favorite: false, hidden: false, deleted: false, deletedAt: null,
    subtype: 0, changeTag: null });
  window.__photos = Array.from({ length: n }, (_, i) => P(`q${i}`, 1792600000 - i * gap));
  window.__pageDelay = 150;
}, [N, GAP]);
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(process.env.URL ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.locator(".px-tile").first().waitFor();

/** The tiles and plain boxes on screen, and how many tiles have their picture. */
const screen = () => page.evaluate(() => {
  const r = document.querySelector(".px-scroll").getBoundingClientRect();
  const on = (sel) => [...document.querySelectorAll(sel)].filter((t) => { const b = t.getBoundingClientRect(); return b.bottom > r.top && b.top < r.bottom; });
  const tiles = on(".px-tile");
  return { tiles: tiles.length, pics: tiles.filter((t) => t.querySelector("img[src]")).length, boxes: on(".px-ph").length, blocks: on(".px-later").length };
});
/** Every tile on screen with its picture, and no plain box: waited for, up to `ms`. */
const whole = async (ms) => {
  const end = Date.now() + ms;
  let s = await screen();
  while (Date.now() < end && !(s.tiles > 0 && s.pics === s.tiles && s.boxes === 0)) {
    await page.waitForTimeout(100);
    s = await screen();
  }
  return s;
};
/** Still whole after `ms`, sampled through it: what went grey after a few seconds shows here. */
const stays = async (ms) => {
  const end = Date.now() + ms;
  let worst = await screen();
  while (Date.now() < end) {
    await page.waitForTimeout(250);
    const s = await screen();
    if (s.boxes > worst.boxes || s.pics < worst.pics) worst = s;
  }
  return worst;
};
const fine = (s) => s.tiles > 0 && s.pics === s.tiles && s.boxes === 0;

let s = await whole(10000);
ok(fine(s), `opened: every tile on screen has its picture  (${JSON.stringify(s)})`);
s = await stays(6000);
ok(fine(s), `and keeps it for six seconds  (worst ${JSON.stringify(s)})`);

await page.evaluate((payload) => window.__emit("quince://synced", payload), status(["photos"]));
s = await stays(4000);
ok(fine(s), `a sync landing keeps every picture on screen  (worst ${JSON.stringify(s)})`);

// The scrollbar dragged half way down, then back to the top.
await page.evaluate(() => { const sc = document.querySelector(".px-scroll"); sc.scrollTop = sc.scrollHeight / 2; });
s = await whole(10000);
ok(fine(s), `dragged half way down: every tile there has its picture  (${JSON.stringify(s)})`);
s = await stays(4000);
ok(fine(s), `and keeps it  (worst ${JSON.stringify(s)})`);
await page.evaluate(() => { document.querySelector(".px-scroll").scrollTop = 0; });
s = await whole(10000);
ok(fine(s), `back at the top: every tile has its picture  (${JSON.stringify(s)})`);

// A tile on screen opens its photo: it is a photo, not a box.
await page.locator(".px-tile").first().dblclick();
const opened = await page.locator(".px-viewer, [role=dialog]").first().waitFor({ timeout: 5000 }).then(() => true, () => false);
ok(opened, "a tile on screen opens its photo");

await b.close();
console.log(failures ? `\n${failures} FAILED` : "\nall passed");
process.exit(failures ? 1 : 0);
