// Going to a month, as icloud.com's calendar button does: years down the side, the year's
// months as tiles with a photo each in calendar order; a tile takes the grid to that month.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 700 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
// A library across two years, a few photos a month.
await ctx.addInitScript(() => {
  const P = (i, t) => ({ id: `j${i}`, master: `mj${i}`, takenAt: t, addedAt: t, kind: "image", width: 4032, height: 3024, duration: null, filename: `IMG_${i}.JPG`, size: 1, uti: "public.jpeg", favorite: false, hidden: false, deleted: false, deletedAt: null, subtype: 0, changeTag: null });
  const out = [];
  let i = 0;
  for (let m = 0; m < 20; m++) for (let k = 0; k < 9; k++) out.push(P(i++, Date.UTC(2026, 9 - m, 20 - k) / 1000));
  window.__photos = out;
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(500);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

const pick = await page.evaluate(() => new Promise((resolve) => {
  const seen = [];
  const t0 = performance.now();
  const tick = () => {
    const d = document.querySelector(".px-jump");
    if (d) seen.push(Number(getComputedStyle(d).opacity));
    if (performance.now() - t0 < 350) requestAnimationFrame(tick); else resolve({ seen, engine: document.querySelector(".px-jump")?.getAnimations({ subtree: true }).length ?? 0 });
  };
  document.querySelector('[aria-label="Go to a Month"]').click();
  requestAnimationFrame(tick);
}));
ok(pick.seen.length > 0 && pick.seen[0] < 0.6 && pick.seen.at(-1) === 1 && pick.engine === 0,
  `the month picker fades in, drawn by the page — never whole for a frame first, never the engine's animation  (first frame ${pick.seen[0]?.toFixed(2)}, last ${pick.seen.at(-1)}, engine ${pick.engine})`);
const dialog = page.getByRole("dialog", { name: "Go to a month" });
ok((await dialog.count()) === 1, "the calendar button opens the month picker");
ok(JSON.stringify(await dialog.getByRole("tab").allInnerTexts()) === JSON.stringify(["2026", "2025"]), `years down the side, newest first  ${await dialog.getByRole("tab").allInnerTexts()}`);
const months2026 = await dialog.locator(".px-jump-month span").allInnerTexts();
ok(months2026.join() === "Jan,Feb,Mar,Apr,May,Jun,Jul,Aug,Sep,Oct", `the year's months in calendar order, each with a photo  ${months2026}`);
await dialog.getByRole("tab", { name: "2025" }).click();
ok((await dialog.locator(".px-jump-month span").allInnerTexts()).join() === "Mar,Apr,May,Jun,Jul,Aug,Sep,Oct,Nov,Dec", "another year shows its own months");
await dialog.getByRole("button", { name: "June 2025" }).click();
// A smooth scroll, however far: settled once the scroller stops moving.
await page.locator(".px-scroll").evaluate((el) => new Promise((done) => {
  let last = -1, still = 0;
  const tick = () => { if (el.scrollTop === last) still++; else { still = 0; last = el.scrollTop; } if (still > 8) done(); else requestAnimationFrame(tick); };
  tick();
}));
const top = await page.evaluate(() => {
  const s = document.querySelector(".px-scroll").getBoundingClientRect();
  const m = document.querySelector('.px-month[aria-label="June 2025"]').getBoundingClientRect();
  return Math.round(m.top - s.top);
});
ok((await dialog.count()) === 0 && Math.abs(top) <= 8, `a month takes the grid there  (its heading ${top}px from the top)`);
await page.getByRole("button", { name: "Go to a Month" }).click();
await page.waitForTimeout(200);
await page.keyboard.press("Escape");
ok((await page.getByRole("dialog", { name: "Go to a month" }).count()) === 0, "Escape closes it");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
