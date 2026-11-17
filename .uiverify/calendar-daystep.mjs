// Stepping through days in Day view: the day's list changes in one draw, never empty in
// between, and the days around are already read — no read per step.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript((n) => { window.__manyEvents = n; }, 400);
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Calendar");
await page.getByRole("button", { name: "Day", exact: true }).click();
await page.waitForSelector(".cx-pane");
await page.waitForTimeout(600);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const reads = await page.evaluate(() => window.__eventReads);
await page.evaluate(() => {
  window.__blank = 0;
  window.__watch = true;
  const tick = () => { if (document.querySelector(".cx-pane-empty")) window.__blank++; if (window.__watch) requestAnimationFrame(tick); };
  requestAnimationFrame(tick);
});
for (let i = 0; i < 6; i++) {
  await page.getByRole("button", { name: "Next", exact: true }).click();
  await page.waitForTimeout(120);
}
const blank = await page.evaluate(() => { window.__watch = false; return window.__blank; });
const after = await page.evaluate(() => window.__eventReads);
ok(blank === 0, `the day's list never draws empty between days that have something  [${blank} empty frames]`);
ok(after === reads, `stepping a day inside the months read asks for nothing again  [${after - reads} reads]`);
await b.close();
process.exitCode = failures ? 1 : 0;
