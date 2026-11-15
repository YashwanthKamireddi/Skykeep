// A view opens on its newest photos: at its top when Newest First, at its foot when Oldest
// First, as Photos on a Mac and an iPhone open on the latest at the bottom (discussion #39:
// with Oldest First, All Photos opened on 2013 and the newest were a long scroll away).
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
await page.goto(process.env.URL ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(600);
const where = () => page.evaluate(() => {
  const el = [...document.querySelectorAll(".px-tile")].map((t) => t.closest("[class*=scroll], .px-grid-scroll, .px-scroll") ?? t.parentElement).find(Boolean);
  let s = document.querySelector(".px-tile");
  while (s && !(s.scrollHeight > s.clientHeight + 4 && getComputedStyle(s).overflowY !== "visible")) s = s.parentElement;
  return s ? { top: s.scrollTop, max: s.scrollHeight - s.clientHeight } : null;
});
const atTop = await where();
ok(atTop && atTop.max > 1000 && atTop.top === 0, `Newest First opens at the top, on the newest  ${JSON.stringify(atTop)}`);
await page.getByRole("button", { name: "Sort" }).click();
await page.locator(".row-menu").getByText("Oldest First").click();
await page.waitForTimeout(500);
const foot = await where();
ok(foot && foot.max - foot.top < 4, `Oldest First opens at the foot, on the newest  ${JSON.stringify(foot)}`);
const shown = await page.locator(".px-tile").evaluateAll((t) => t.map((x) => x.dataset.id));
ok(shown.includes("q0"), `and the newest photo is in sight  ${shown.slice(-3)}`);
// Another view and back: still opens on the newest.
await page.locator(".px-source", { hasText: "Favorites" }).click();
await page.waitForTimeout(300);
await page.locator(".px-source", { hasText: "Library" }).first().click();
await page.waitForTimeout(500);
const again = await where();
ok(again && again.max - again.top < 4, `back in the Library, at its foot again  ${JSON.stringify(again)}`);
await b.close();
process.exitCode = failures ? 1 : 0;
