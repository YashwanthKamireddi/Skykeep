// Photos opens on the newest photos and stays there while the whole library is read. A view
// that opens at its foot (oldest first, as on a Mac) paints a first small page before its whole
// index is here; that page was the OLDEST photos, so on a large library the view first showed
// them and then moved to the newest when the index came (discussion #167: "slides one way,
// then the other"). The first page is the newest now: the photos at the foot before the index
// lands are the ones there after it.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 }, reducedMotion: "no-preference" });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
await ctx.addInitScript(() => {
  const P = (i, t) => ({ id: `q${i}`, master: `mq${i}`, takenAt: t, addedAt: t, kind: "image", width: 4032, height: 3024,
    duration: null, filename: `IMG_${i}.JPG`, size: 2400000, uti: "public.jpeg", favorite: false, hidden: false,
    deleted: false, deletedAt: null, subtype: 0, changeTag: null });
  // q0 is the newest.
  window.__photos = Array.from({ length: 20000 }, (_, i) => P(i, 1792600000 - i * 7200));
  // A large library's whole index takes a moment: the first page is drawn before it.
  const wrap = () => {
    const inner = window.__TAURI_INTERNALS__?.invoke;
    if (!inner || inner.__slow) return;
    const slow = async (cmd, args) => {
      if (cmd === "photos_index") await new Promise((r) => setTimeout(r, 1500));
      return inner(cmd, args);
    };
    slow.__slow = true;
    window.__TAURI_INTERNALS__.invoke = slow;
  };
  wrap();
  document.addEventListener("DOMContentLoaded", wrap);
  // The tiles at the foot of the grid, each frame.
  window.__seen = [];
  const tick = () => {
    const s = document.querySelector(".px-scroll");
    if (s) {
      const box = s.getBoundingClientRect();
      const ids = [...s.querySelectorAll(".px-tile[data-id]")]
        .filter((t) => { const r = t.getBoundingClientRect(); return r.bottom > box.top && r.top < box.bottom; })
        .map((t) => Number(t.dataset.id.slice(1)));
      if (ids.length) window.__seen.push(Math.min(...ids));
    }
    if (window.__seen.length < 300) requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
});
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(3500);
const seen = await page.evaluate(() => window.__seen);
// The newest photo on screen, frame by frame: q0 from the first frame with tiles to the last.
const notNewest = seen.filter((n) => n > 50);
ok(seen.length > 20, `tiles drawn in ${seen.length} frames`);
ok(notNewest.length === 0, `every frame shows the newest photos (${notNewest.length} frames showed older ones, first: q${notNewest[0] ?? "-"})`);
await b.close();
process.exitCode = failures ? 1 : 0;
