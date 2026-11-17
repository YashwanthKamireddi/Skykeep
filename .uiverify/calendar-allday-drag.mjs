// Carried between the all-day row and the hours, as in Calendar on the Mac: a timed event let
// go over the all-day row becomes all-day on that day; an all-day event let go over the hours
// becomes an hour long from where it was let go. Drawn where it would land while held, saved at
// once, and Ctrl+Z puts it back as it was. Escape while held leaves it where it was.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1300, height: 850 } });
await ctx.addInitScript(installStub, {
  screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true,
  extraEvents: [
    // Today, all day: from this midnight (9 AM less nine hours) to the next.
    { guid: "ad1", title: "Offsite", hour: -9, length: 24, allDay: true },
    // Today from 11 to noon.
    { guid: "t1", title: "Budget talk", hour: 2, length: 1 },
  ],
});
// Apple slow to answer, as on a slow network: what is drawn meanwhile is the app's own.
await ctx.addInitScript(() => {
  const wrap = () => {
    const t = window.__TAURI_INTERNALS__;
    if (!t || t.__slow) return;
    const inner = t.invoke;
    t.invoke = async (cmd, args) => {
      if (cmd === "event_save") await new Promise((r) => setTimeout(r, 1200));
      return inner(cmd, args);
    };
    t.__slow = true;
  };
  wrap(); document.addEventListener("DOMContentLoaded", wrap);
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Calendar");
await page.keyboard.press("Control+2");
await page.locator(".cx-scroll").waitFor();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const saves = () => page.evaluate(() => window.__eventSaves ?? []);
const savesReach = (n) => page.waitForFunction((n) => (window.__eventSaves ?? []).length >= n, n, { timeout: 3000 }).catch(() => {});
const midnightIn = (n) => page.evaluate((n) => { const d = new Date(); d.setHours(0, 0, 0, 0); d.setDate(d.getDate() + n); return d.getTime() / 1000; }, n);
const clockIn = (n, h) => page.evaluate(([n, h]) => { const d = new Date(); d.setHours(h, 0, 0, 0); d.setDate(d.getDate() + n); return d.getTime() / 1000; }, [n, h]);
await page.locator(".cx-scroll").evaluate((el) => { el.scrollTop = 8 * 48; });

const today = await page.evaluate(() => [...document.querySelectorAll(".cal-col-head")].findIndex((h) => h.classList.contains("is-today")));
const by = today === 6 ? -1 : 1;
const lanes = page.locator(".cx-allday-lanes");
const row = await lanes.boundingBox();
const colWidth = row.width / 7;

// 1. The timed event carried up into the all-day row, and a day on.
const block = page.locator(".cx-block", { hasText: "Budget talk" });
const from = await block.boundingBox();
await page.mouse.move(from.x + 20, from.y + 10);
await page.mouse.down();
await page.mouse.move(from.x + 20, from.y - 40, { steps: 3 });
await page.mouse.move(from.x + 20 + by * colWidth, row.y + row.height - 6, { steps: 6 });
const carried = page.locator(".cx-allday .cx-bar-event.is-carried", { hasText: "Budget talk" });
await carried.waitFor({ timeout: 2000 }).catch(() => {});
ok(await carried.isVisible(), "held over the all-day row, it is drawn there");
const carriedAt = await carried.boundingBox();
const dayLeft = (await lanes.boundingBox()).x + colWidth * (today + by);
ok(carriedAt && Math.abs(carriedAt.x - dayLeft) < 6, `on the day it is over  [${carriedAt && Math.round(carriedAt.x - dayLeft)}]`);
ok(!(await block.isVisible()), "and not in the hours meanwhile");
const scrolledBefore = await page.locator(".cx-scroll").evaluate((el) => el.scrollTop);
await page.waitForTimeout(500);
ok(await page.locator(".cx-scroll").evaluate((el) => el.scrollTop) === scrolledBefore, "held there, the hours do not scroll under it");
await page.mouse.up();
// Drawn as all-day at once, before Apple has answered.
const asBar = page.locator(".cx-allday .cx-bar-event:not(.is-carried)", { hasText: "Budget talk" });
await asBar.waitFor({ timeout: 1000 }).catch(() => {});
ok(await asBar.isVisible() && (await page.locator(".cx-block", { hasText: "Budget talk" }).count()) === 0, "let go, it is an all-day event at once");
await savesReach(1);
let s = await saves();
const start = await midnightIn(0);
ok(s.length === 1 && s[0].guid === "t1" && s[0].allDay === true && s[0].startsAt === (await midnightIn(by)) && s[0].endsAt === (await midnightIn(by + 1)),
  `saved all-day on that day  ${JSON.stringify(s[0] && { g: s[0].guid, a: s[0].allDay, d: (s[0].startsAt - start) / 3600, len: (s[0].endsAt - s[0].startsAt) / 3600 })}`);
ok(await page.getByRole("dialog").count() === 0, "nothing opens");

// Ctrl+Z: back in the hours, as it was.
await page.keyboard.press("Control+z");
await savesReach(2);
s = await saves();
ok(s.length === 2 && s[1].guid === "t1" && s[1].allDay === false && s[1].startsAt === (await clockIn(0, 11)) && s[1].endsAt === (await clockIn(0, 12)),
  `Ctrl+Z puts it back in the hours  ${JSON.stringify(s[1] && { g: s[1].guid, a: s[1].allDay, h: (s[1].startsAt - start) / 3600 })}`);
await page.locator(".cx-block", { hasText: "Budget talk" }).waitFor({ timeout: 2000 }).catch(() => {});
ok(await page.locator(".cx-block", { hasText: "Budget talk" }).isVisible(), "and drawn there again");

// 2. The all-day bar carried down into the hours, a day on, at 2 PM.
const bar = page.locator(".cx-allday .cx-bar-event", { hasText: "Offsite" });
const at = await bar.boundingBox();
const columns = await page.locator(".cx-columns").boundingBox();
const twoPm = columns.y + 14 * 48 + 6;
ok(twoPm < 850, `2 PM is on screen  [${Math.round(twoPm)}]`);
await page.mouse.move(at.x + 10, at.y + at.height / 2);
await page.mouse.down();
await page.mouse.move(at.x + 10, at.y + 40, { steps: 3 });
await page.mouse.move(columns.x + colWidth * (today + by) + colWidth / 2, twoPm, { steps: 6 });
const hour = page.locator(".cx-column .cx-block.is-carried", { hasText: "Offsite" });
await hour.waitFor({ timeout: 2000 }).catch(() => {});
ok(await hour.isVisible(), "held over the hours, it is drawn there as an hour");
const hourAt = await hour.boundingBox();
ok(hourAt && Math.abs(hourAt.y - (columns.y + 14 * 48)) < 3 && Math.abs(hourAt.height - 46) < 3,
  `from the quarter hour it is over, an hour long  [${hourAt && Math.round(hourAt.y - columns.y)}, ${hourAt && Math.round(hourAt.height)}]`);
ok(!(await bar.isVisible()), "and not in the all-day row meanwhile");
await page.mouse.up();
const asBlock = page.locator(".cx-block:not(.is-carried)", { hasText: "Offsite" });
await asBlock.waitFor({ timeout: 1000 }).catch(() => {});
ok(await asBlock.isVisible() && (await page.locator(".cx-allday .cx-bar-event", { hasText: "Offsite" }).count()) === 0, "let go, it is an hour in the day at once");
await savesReach(3);
s = await saves();
ok(s.length === 3 && s[2].guid === "ad1" && s[2].allDay === false && s[2].startsAt === (await clockIn(by, 14)) && s[2].endsAt === (await clockIn(by, 15)),
  `saved as an hour from 2 PM that day  ${JSON.stringify(s[2] && { g: s[2].guid, a: s[2].allDay, h: (s[2].startsAt - start) / 3600, len: (s[2].endsAt - s[2].startsAt) / 3600 })}`);
ok(await page.getByRole("dialog").count() === 0, "nothing opens");

await page.keyboard.press("Control+z");
await savesReach(4);
s = await saves();
ok(s.length === 4 && s[3].guid === "ad1" && s[3].allDay === true && s[3].startsAt === start && s[3].endsAt === (await midnightIn(1)),
  `Ctrl+Z makes it all-day again, on its day  ${JSON.stringify(s[3] && { g: s[3].guid, a: s[3].allDay, d: (s[3].startsAt - start) / 3600 })}`);
await page.locator(".cx-allday .cx-bar-event", { hasText: "Offsite" }).waitFor({ timeout: 2000 }).catch(() => {});
ok(await page.locator(".cx-allday .cx-bar-event", { hasText: "Offsite" }).isVisible(), "and drawn in the all-day row again");

// 3. Escape while held over the all-day row: nothing moves, nothing is saved.
const again = await page.locator(".cx-block", { hasText: "Budget talk" }).boundingBox();
await page.mouse.move(again.x + 20, again.y + 10);
await page.mouse.down();
await page.mouse.move(again.x + 20, row.y + row.height - 6, { steps: 6 });
await page.keyboard.press("Escape");
await page.mouse.up();
await page.waitForTimeout(300);
ok((await saves()).length === 4 && await page.locator(".cx-block", { hasText: "Budget talk" }).isVisible()
  && (await page.locator(".cx-bar-event.is-carried").count()) === 0, "Escape leaves it where it was");

await b.close();
console.log(failures ? `${failures} failed` : "all passed");
process.exitCode = failures ? 1 : 0;
