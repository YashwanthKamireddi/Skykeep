// Photos opens on the newest photos and stays there while a large library is read, in either
// order, and a scroll made while it is read stays among the newest.
//
// A view paints a small first page before its whole index is here, then builds its first
// months, fetches their photos and draws their pictures. All of that started from the list's
// start, which with Oldest First (the default, as on a Mac) is the oldest end, while the view
// opens at its foot on the newest: the first page was the oldest photos (discussion #167), and
// on a dense library the three oldest months were built, fetched and drawn on every open,
// thousands of tiles past what the store holds, so the newest on screen were let go and came
// last (#186). Everything now starts from the end the view opens on.
//
// A library shaped like #186's (37,765 photos, ~1,080 a month), every read slow as on a busy
// Windows machine. Checked in both orders, with and without a wheel scroll during the load:
// every frame shows only the newest photos, and nothing of the oldest months is fetched first.
// The same once the view is read whole and the other order is chosen from its sort menu (#224).
//
//   URL=http://localhost:4173 node .uiverify/photos-open-on-newest.mjs   (ENGINE=webkit for WebKit)
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const URL = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const COUNT = 37765;
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

async function open({ newestFirst, scroll, resort }) {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 800 }, reducedMotion: "no-preference" });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, photosNewestFirst: newestFirst });
  await ctx.addInitScript((count) => {
    const P = (i, t) => ({ id: `q${i}`, master: `mq${i}`, takenAt: t, addedAt: t, kind: "image", width: 4032, height: 3024,
      duration: null, filename: `IMG_${i}.JPG`, size: 2400000, uti: "public.jpeg", favorite: false, hidden: false,
      deleted: false, deletedAt: null, subtype: 0, changeTag: null });
    // q0 is the newest; about 1,080 a month.
    window.__photos = Array.from({ length: count }, (_, i) => P(i, 1792600000 - i * 2400));
    // Every read slow, as a large library's are on a busy machine; what is asked for is kept.
    const slow = { photos_index: 2000, photos_page: 1200, photos_get: 400 };
    window.__asked = [];
    const wrap = () => {
      const inner = window.__TAURI_INTERNALS__?.invoke;
      if (!inner || inner.__slow) return;
      const f = async (cmd, args) => {
        if (cmd === "photos_get") window.__asked.push({ at: performance.now(), ids: args.ids });
        if (slow[cmd]) await new Promise((r) => setTimeout(r, slow[cmd]));
        return inner(cmd, args);
      };
      f.__slow = true;
      window.__TAURI_INTERNALS__.invoke = f;
    };
    wrap();
    document.addEventListener("DOMContentLoaded", wrap);
    // Each frame: the oldest photo on screen, and when the newest were first drawn.
    window.__frames = [];
    const tick = () => {
      const s = document.querySelector(".px-scroll");
      if (s) {
        const box = s.getBoundingClientRect();
        const ids = [...s.querySelectorAll(".px-tile[data-id]")]
          .filter((t) => { const r = t.getBoundingClientRect(); return r.bottom > box.top && r.top < box.bottom; })
          .map((t) => Number(t.dataset.id.slice(1)));
        if (ids.length) window.__frames.push(Math.max(...ids));
      }
      if (window.__frames.length < 360) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  }, COUNT);
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(URL, { waitUntil: "networkidle" });
  await openScreen(page, "Photos");
  if (scroll) {
    // Toward the older photos, a few notches, while the library is still being read.
    await page.waitForTimeout(900);
    const box = await page.locator(".px-scroll").boundingBox();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    for (let i = 0; i < 4; i++) {
      await page.mouse.wheel(0, newestFirst ? 400 : -400);
      await page.waitForTimeout(80);
    }
  }
  await page.waitForTimeout(5000);
  if (resort) {
    // Read whole and settled, then the other order chosen from the sort menu: the view is read
    // again in that order, and it is opened again as it was (#224: the old photos showed).
    await page.evaluate(() => {
      window.__asked = [];
      window.__frames = [];
      const s = document.querySelector(".px-scroll");
      const tick = () => {
        const box = s.getBoundingClientRect();
        const ids = [...s.querySelectorAll(".px-tile[data-id]")]
          .filter((t) => { const r = t.getBoundingClientRect(); return r.bottom > box.top && r.top < box.bottom; })
          .map((t) => Number(t.dataset.id.slice(1)));
        if (ids.length) window.__frames.push(Math.max(...ids));
        if (window.__frames.length < 360) requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    });
    await page.getByRole("button", { name: "Sort" }).click();
    await page.locator(".row-menu").getByText(newestFirst ? "Oldest First" : "Newest First").click();
    await page.waitForTimeout(5000);
  }
  const r = await page.evaluate((count) => ({
    frames: window.__frames,
    asked: window.__asked,
    // Tiles built for the oldest months (nobody is looking there).
    builtOld: [...document.querySelectorAll(".px-tile[data-id]")].filter((t) => Number(t.dataset.id.slice(1)) > count - 3500).length,
  }), COUNT);
  await ctx.close();
  return r;
}

const cases = [false, true].flatMap((newestFirst) => [
  { newestFirst, scroll: false },
  { newestFirst, scroll: true },
  // Read whole, then the order changed: the same view, opened again in the other order.
  { newestFirst, scroll: false, resort: true },
]);
for (const { newestFirst, scroll, resort } of cases) {
  {
    const how = `${newestFirst ? "Newest First" : "Oldest First"}${scroll ? ", scrolled during the load" : ""}${resort ? `, then ${newestFirst ? "Oldest" : "Newest"} First chosen` : ""}`;
    const { frames, asked, builtOld } = await open({ newestFirst, scroll, resort });
    // Opened unscrolled, every frame is the first screen's newest; scrolled, a few screens of
    // the newest. Anything older than a month back is the view on the wrong photos.
    const limit = scroll ? 1080 : 200;
    const old = frames.filter((n) => n > limit);
    ok(frames.length > 20, `${how}: tiles drawn in ${frames.length} frames`);
    ok(old.length === 0, `${how}: every frame shows the newest photos (${old.length} showed older ones, oldest q${Math.max(0, ...old)})`);
    // The cause, not only what showed: nothing of the oldest months, three years from where the
    // view is, is fetched or built (it was the list's first page and its first three months,
    // past what the store holds, so the newest on screen were let go).
    const oldest = asked.flatMap((a) => a.ids).map((id) => Number(id.slice(1))).filter((n) => n > COUNT - 3500);
    ok(oldest.length === 0, `${how}: none of the oldest months' photos is fetched (${oldest.length} were)`);
    ok(builtOld === 0, `${how}: none of the oldest months' tiles is built (${builtOld} were)`);
  }
}
await b.close();
process.exitCode = failures ? 1 : 0;
