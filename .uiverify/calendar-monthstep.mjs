// Stepping Month to the next month and back shows its events at once, from what is already
// read, never an empty month that fills in a moment later. The library read is made slow here
// so a month drawn only once its own read came back would show empty.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1300, height: 850 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
// The 15th of next month and of last month, at noon: never in this month's six weeks, always
// in theirs.
await ctx.addInitScript(() => {
  const at = (by) => { const d = new Date(); d.setDate(1); d.setMonth(d.getMonth() + by); d.setDate(15); d.setHours(12, 0, 0, 0); return d.getTime() / 1000; };
  window.__fixedEvents = [
    { guid: "next1", calendarGuid: "work", title: "Board meeting", location: null, startsAt: at(1), endsAt: at(1) + 3600, allDay: false, timezone: null, url: null, notes: null, etag: null },
    { guid: "last1", calendarGuid: "work", title: "Quarter review", location: null, startsAt: at(-1), endsAt: at(-1) + 3600, allDay: false, timezone: null, url: null, notes: null, etag: null },
  ];
});
const page = await ctx.newPage();
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Calendar");
await page.keyboard.press("Control+3");
await page.locator(".cal-grid").waitFor();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
// Everything this month asked for is in; from here on every read takes a second and a half.
await page.waitForFunction(() => (window.__eventReads ?? 0) >= 1);
await page.waitForTimeout(500);
await page.evaluate(() => { window.__eventsSlow = 1500; });

const title = () => page.locator(".cx-title").getAttribute("aria-label");
const was = await title();
await page.keyboard.press("ArrowRight");
await page.waitForFunction((t) => document.querySelector(".cx-title")?.getAttribute("aria-label") !== t, was);
await page.waitForTimeout(200);
ok(await page.locator(".cx-item", { hasText: "Board meeting" }).count() === 1, "next month shows its events at once");

await page.keyboard.press("ArrowLeft");
await page.keyboard.press("ArrowLeft");
await page.waitForTimeout(200);
ok(await page.locator(".cx-item", { hasText: "Quarter review" }).count() === 1, "and so does last month");

await b.close();
console.log(failures ? `${failures} failed` : "all passed");
process.exitCode = failures ? 1 : 0;
