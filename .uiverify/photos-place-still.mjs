// Scrolled into the middle of a month and left there, the grid does not move by itself when its
// layout is answered (photos/place): the photo at the top, as the keeper reads it, is where its
// tile really is. Read from the month's height shared out between its rows, it was a part of a
// gap off for every row down — up to the gap itself, 14 px with photos shown whole — and the
// view nudged by that much the first time anything changed size after the person's scroll.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
for (const fit of [true, false]) {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
  await ctx.addInitScript((fit) => {
    window.localStorage.setItem("quince.photos.fit", fit ? "1" : "0");
    const P = (i, t) => ({ id: `q${i}`, master: `mq${i}`, takenAt: t, addedAt: t, kind: "image", width: 4032, height: 3024,
      duration: null, filename: `IMG_${i}.JPG`, size: 2400000, uti: "public.jpeg", favorite: false, hidden: false,
      deleted: false, deletedAt: null, subtype: 0, changeTag: null });
    // About 140 photos a month: twenty rows each.
    window.__photos = Array.from({ length: 1600 }, (_, i) => P(i, 1792600000 - i * 21600 * 0.8));
  }, fit);
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await openScreen(page, "Photos");
  await page.waitForTimeout(1500);
  const name = fit ? "whole photos" : "squares";
  for (const share of [0.35, 0.55, 0.75]) {
    // The person scrolls to a row well down a month, and stops.
    await page.locator(".px-scroll").evaluate((s, share) => {
      const secs = [...s.querySelectorAll(".px-month")];
      const box = s.getBoundingClientRect();
      const sec = secs.find((m) => m.getBoundingClientRect().top - box.top + s.scrollTop > s.scrollHeight * share) ?? secs[1];
      const r = sec.getBoundingClientRect();
      s.scrollTop += r.top - box.top + r.height * 0.8;
    }, share);
    await page.waitForTimeout(700);
    const seen = () => page.evaluate(() => {
      const s = document.querySelector(".px-scroll");
      const box = s.getBoundingClientRect();
      const t = [...s.querySelectorAll(".px-tile")].find((x) => x.getBoundingClientRect().bottom > box.top + 60);
      return { id: t.dataset.id, top: Math.round((t.getBoundingClientRect().top - box.top) * 10) / 10, scroll: Math.round(s.scrollTop) };
    });
    const before = await seen();
    // Something changes size that moves nothing in the grid: the window made a little shorter.
    await page.setViewportSize({ width: 1280, height: 760 });
    await page.waitForTimeout(400);
    const after = await seen();
    await page.setViewportSize({ width: 1280, height: 800 });
    await page.waitForTimeout(400);
    ok(after.id === before.id && Math.abs(after.top - before.top) < 1,
      `${name}, ${share}: left in the middle of a month, the view stays still  (${JSON.stringify(before)} → ${JSON.stringify(after)})`);
  }
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
