// Photos opens as on a Mac and an iPhone: oldest at the top, and the view at its foot, on the
// newest (discussions #39 and #102: All Photos opened on 2013 and the newest were a long scroll
// away). Newest First, chosen, opens at the top — on the newest either way.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
await ctx.addInitScript(() => {
  const P = (i, t) => ({ id: `q${i}`, master: `mq${i}`, takenAt: t, addedAt: t, kind: "image", width: 4032, height: 3024,
    duration: null, filename: `IMG_${i}.JPG`, size: 2400000, uti: "public.jpeg", favorite: i % 9 === 0, hidden: false,
    deleted: false, deletedAt: null, subtype: 0, changeTag: null });
  window.__photos = Array.from({ length: 400 }, (_, i) => P(i, 1792600000 - i * 86400));
});
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(600);
const where = () => page.evaluate(() => {
  const el = [...document.querySelectorAll(".px-tile")].map((t) => t.closest("[class*=scroll], .px-grid-scroll, .px-scroll") ?? t.parentElement).find(Boolean);
  let s = document.querySelector(".px-tile");
  while (s && !(s.scrollHeight > s.clientHeight + 4 && getComputedStyle(s).overflowY !== "visible")) s = s.parentElement;
  return s ? { top: s.scrollTop, max: s.scrollHeight - s.clientHeight } : null;
});
// Nothing chosen: Apple's order, at the foot, the newest in sight.
const foot = await where();
ok(foot && foot.max > 1000 && foot.max - foot.top < 4, `it opens at the foot, on the newest  ${JSON.stringify(foot)}`);
const onScreen = (id) => page.evaluate((id) => {
  const r = document.querySelector(`.px-tile[data-id="${id}"]`)?.getBoundingClientRect();
  return Boolean(r && r.bottom > 0 && r.top < innerHeight);
}, id);
ok((await onScreen("q0")) && !(await onScreen("q399")), "the newest photo is on screen, the oldest far above");
await page.getByRole("button", { name: "Sort" }).click();
await page.locator(".row-menu").getByText("Newest First").click();
await page.waitForTimeout(500);
const atTop = await where();
ok(atTop && atTop.top === 0, `Newest First opens at the top, on the newest  ${JSON.stringify(atTop)}`);
await page.getByRole("button", { name: "Sort" }).click();
await page.locator(".row-menu").getByText("Oldest First").click();
await page.waitForTimeout(500);
// Another view and back: still opens on the newest.
await page.locator(".px-source", { hasText: "Favorites" }).click();
await page.waitForTimeout(300);
await page.locator(".px-source", { hasText: "Library" }).first().click();
await page.waitForTimeout(500);
const again = await where();
ok(again && again.max - again.top < 4, `back in the Library, at its foot again  ${JSON.stringify(again)}`);
// As the real app: the library's index and pages come over IPC a while after the window has
// drawn. Held for half a second, the view sat at its top (2019) once they came (18 Nov).
const slow = await b.newContext({ viewport: { width: 1280, height: 800 } });
await slow.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
await slow.addInitScript(() => {
  const P = (i, t) => ({ id: `q${i}`, master: `mq${i}`, takenAt: t, addedAt: t, kind: "image", width: 4032, height: 3024,
    duration: null, filename: `IMG_${i}.JPG`, size: 2400000, uti: "public.jpeg", favorite: false, hidden: false,
    deleted: false, deletedAt: null, subtype: 0, changeTag: null });
  window.__photos = Array.from({ length: 400 }, (_, i) => P(i, 1792600000 - i * 86400));
  window.__indexDelay = 1500;
  window.__pageDelay = 400;
});
const sp = await slow.newPage();
sp.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await sp.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(sp, "Photos");
await sp.waitForTimeout(4000);
const late = await sp.evaluate(() => {
  const s = document.querySelector(".px-scroll");
  return { top: Math.round(s.scrollTop), max: s.scrollHeight - s.clientHeight };
});
ok(late.max > 1000 && late.max - late.top < 4, `photos arriving late, it still opens at the foot  ${JSON.stringify(late)}`);
// And the person scrolling up is never pulled back down.
await sp.locator(".px-scroll").evaluate((s) => s.dispatchEvent(new WheelEvent("wheel", { deltaY: -600, bubbles: true })));
await sp.locator(".px-scroll").evaluate((s) => { s.scrollTop -= 1500; });
await sp.waitForTimeout(800);
const up = await sp.evaluate(() => { const s = document.querySelector(".px-scroll"); return s.scrollHeight - s.clientHeight - s.scrollTop; });
ok(up > 1000, `scrolled up, it stays where the person put it  (${Math.round(up)}px above the foot)`);
// Made full screen at the foot, it stays at the foot (a wider tile is a taller one: the grid
// grows, and the view landed mid-library, 18 Nov); made small again, still there.
await sp.locator(".px-scroll").evaluate((s) => { s.scrollTop = s.scrollHeight; });
await sp.waitForTimeout(300);
for (const [w, h] of [[1920, 1080], [900, 600], [1600, 1000]]) {
  await sp.setViewportSize({ width: w, height: h });
  await sp.waitForTimeout(500);
  const f = await sp.evaluate(() => { const s = document.querySelector(".px-scroll"); return Math.round(s.scrollHeight - s.clientHeight - s.scrollTop); });
  ok(f <= 2, `resized to ${w}×${h} at the foot, it stays at the foot  (${f}px above it)`);
}
// Scrolled up to a photo, a resize keeps that photo where it was on the screen.
await sp.locator(".px-scroll").evaluate((s) => { s.scrollTop = Math.round(s.scrollHeight / 3); });
await sp.waitForTimeout(400);
const before = await sp.evaluate(() => {
  const s = document.querySelector(".px-scroll"); const box = s.getBoundingClientRect();
  const t = [...s.querySelectorAll(".px-tile")].find((x) => x.getBoundingClientRect().bottom > box.top + 120);
  return { id: t.dataset.id, top: Math.round(t.getBoundingClientRect().top - box.top) };
});
await sp.setViewportSize({ width: 1920, height: 1080 });
await sp.waitForTimeout(500);
const after = await sp.evaluate((id) => {
  const s = document.querySelector(".px-scroll"); const t = s.querySelector(`.px-tile[data-id="${id}"]`);
  return t ? Math.round(t.getBoundingClientRect().top - s.getBoundingClientRect().top) : null;
}, before.id);
ok(after !== null && Math.abs(after - before.top) <= 2, `scrolled up, a resize keeps the same photo in place  (${before.id}: ${before.top} → ${after})`);
await slow.close();
await b.close();
process.exitCode = failures ? 1 : 0;
