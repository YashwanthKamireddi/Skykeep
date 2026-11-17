// Week and Day as Calendar keeps them: the hours scrolled to stay put from one week to the
// next; the all-day row makes an all-day event on a double-click and carries a bar to another
// day; Ctrl+Z takes back the last move or deletion; and an event that ends as it starts is
// saved that way, not lengthened.
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
    // A deadline: no length at all, as an invitation can send one.
    { guid: "dl1", title: "Forms due", hour: 8, length: 0 },
  ],
});
const page = await ctx.newPage();
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Calendar");
await page.keyboard.press("Control+2");
await page.locator(".cx-scroll").waitFor();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const saves = () => page.evaluate(() => window.__eventSaves ?? []);
const savesReach = (n) => page.waitForFunction((n) => (window.__eventSaves ?? []).length >= n, n, { timeout: 3000 }).catch(() => {});
const midnightIn = (n) => page.evaluate((n) => { const d = new Date(); d.setHours(0, 0, 0, 0); d.setDate(d.getDate() + n); return d.getTime() / 1000; }, n);

// The evening scrolled to stays in view on the next week, and on the way back.
const scroller = page.locator(".cx-scroll");
await scroller.evaluate((el) => { el.scrollTop = 18 * 48; });
const before = await scroller.evaluate((el) => el.scrollTop);
// The week is told by its first day, not the title: the title names only the month, and the
// next week is often in the same month.
const firstDay = () => document.querySelector(".cal-col-head")?.textContent;
const title = await page.evaluate(firstDay);
await page.keyboard.press("ArrowRight");
await page.waitForFunction((t) => document.querySelector(".cal-col-head")?.textContent !== t, title);
ok(await scroller.evaluate((el) => el.scrollTop) === before, "the next week keeps the hours scrolled to");
await page.keyboard.press("ArrowLeft");
await page.waitForFunction((t) => document.querySelector(".cal-col-head")?.textContent === t, title);
ok(await scroller.evaluate((el) => el.scrollTop) === before, "and so does the week before");

// A double-click on a free part of the all-day row: an all-day event on that day.
const lanes = page.locator(".cx-allday-lanes");
const box = await lanes.boundingBox();
const today = await page.evaluate(() => [...document.querySelectorAll(".cal-col-head")].findIndex((h) => h.classList.contains("is-today")));
const other = today === 6 ? 5 : 6;
const colWidth = box.width / 7;
await page.mouse.dblclick(box.x + colWidth * other + colWidth / 2, box.y + box.height - 3);
const made = page.getByRole("dialog", { name: "New event" });
await made.waitFor({ timeout: 2000 }).catch(() => {});
ok(await made.isVisible(), "a double-click on the all-day row starts a new event");
ok(await made.getByRole("checkbox", { name: "all-day" }).isChecked().catch(() => false), "and it is all-day");
// Drawn in the row while it is named, under its name as typed.
await made.getByLabel("Title").fill("Team day");
const draftBar = page.locator(".cx-allday .cx-bar-event.is-draft");
ok(await draftBar.count() === 1 && (await draftBar.textContent()) === "Team day", `it is drawn in the all-day row as it is named  [${await draftBar.count()}]`);
const drawnAt = await draftBar.boundingBox();
ok(drawnAt && Math.abs(drawnAt.x - (box.x + colWidth * other)) < 6, "on the day it was made");
await page.keyboard.press("Escape");
await made.waitFor({ state: "detached" });

// The all-day bar carried one day on.
const bar = page.locator(".cx-allday .cx-bar-event", { hasText: "Offsite" });
const by = today === 6 ? -1 : 1;
const at = await bar.boundingBox();
await page.mouse.move(at.x + 10, at.y + at.height / 2);
await page.mouse.down();
await page.mouse.move(at.x + 10 + (by * colWidth) / 2, at.y + at.height / 2, { steps: 3 });
await page.mouse.move(at.x + 10 + by * colWidth, at.y + at.height / 2, { steps: 4 });
const moved = await bar.boundingBox();
ok(Math.abs(moved.x - at.x - by * colWidth) < 4, `the bar follows the pointer  [${Math.round(moved.x - at.x)}]`);
await page.mouse.up();
await savesReach(1);
let s = await saves();
const start = await midnightIn(0);
ok(s.length === 1 && s[0].guid === "ad1" && s[0].allDay && s[0].startsAt === (await midnightIn(by)) && s[0].endsAt === (await midnightIn(by + 1)),
  `the bar is moved a day  ${JSON.stringify(s[0] && { g: s[0].guid, d: (s[0].startsAt - start) / 3600 })}`);
ok(await page.getByRole("dialog").count() === 0, "a carried bar does not open");

// Ctrl+Z: the move taken back.
await page.keyboard.press("Control+z");
await savesReach(2);
s = await saves();
ok(s.length === 2 && s[1].guid === "ad1" && s[1].startsAt === start && s[1].endsAt === (await midnightIn(1)), `Ctrl+Z puts it back  ${JSON.stringify(s[1] && { g: s[1].guid, d: (s[1].startsAt - start) / 3600 })}`);
await page.keyboard.press("Control+z");
await page.waitForTimeout(300);
ok((await saves()).length === 2, "a second Ctrl+Z has nothing more to take back");

// A held-still press on the bar opens it.
await bar.click();
const opened = page.getByRole("dialog", { name: "Offsite" });
await opened.waitFor({ timeout: 2000 }).catch(() => {});
ok(await opened.isVisible(), "a click on the bar opens it");
await page.keyboard.press("Escape");
await opened.waitFor({ state: "detached" });

// Its end stretched a day on (or, on the week's last day, its start a day back).
const ends = await bar.boundingBox();
const grabX = by === 1 ? ends.x + ends.width - 2 : ends.x + 2;
await page.mouse.move(grabX, ends.y + ends.height / 2);
await page.mouse.down();
await page.mouse.move(grabX + (by * colWidth) / 2, ends.y + ends.height / 2, { steps: 3 });
await page.mouse.move(grabX + by * colWidth, ends.y + ends.height / 2, { steps: 4 });
const stretched = await bar.boundingBox();
ok(Math.abs(stretched.width - ends.width - colWidth) < 4, `the bar stretches with the pointer  [${Math.round(stretched.width - ends.width)}]`);
await page.mouse.up();
await savesReach(3);
s = await saves();
const [wantFrom, wantTo] = by === 1 ? [start, await midnightIn(2)] : [await midnightIn(-1), await midnightIn(1)];
ok(s.length === 3 && s[2].guid === "ad1" && s[2].startsAt === wantFrom && s[2].endsAt === wantTo,
  `stretched, it spans two days  ${JSON.stringify(s[2] && { g: s[2].guid, from: (s[2].startsAt - start) / 3600, to: (s[2].endsAt - start) / 3600 })}`);
ok(await page.getByRole("dialog").count() === 0, "a stretched bar does not open");

// Deleted from the keyboard, then Ctrl+Z makes it again.
await page.locator(".cx-block", { hasText: "Dentist" }).click();
await page.getByRole("dialog", { name: "Dentist" }).waitFor();
await page.keyboard.press("Escape");
await page.keyboard.press("Delete");
await page.getByRole("button", { name: "Delete", exact: true }).click();
await page.waitForFunction(() => (window.__eventDeletes ?? []).length === 1);
await page.keyboard.press("Control+z");
await savesReach(4);
s = await saves();
ok(s.length === 4 && s[3].guid === null && s[3].title === "Dentist" && JSON.stringify(s[3].alerts) === "[15]",
  `Ctrl+Z makes the deleted event again, its alert with it  ${JSON.stringify(s[3] && { g: s[3].guid, t: s[3].title, a: s[3].alerts })}`);

// An event that ends as it starts keeps no length when its title is changed.
await page.locator(".cx-block", { hasText: "Forms due" }).click();
const deadline = page.getByRole("dialog", { name: "Forms due" });
await deadline.waitFor();
await deadline.getByLabel("Title").fill("Forms in");
// Named by its title, the dialog is "Forms in" now.
await page.getByRole("dialog", { name: "Forms in" }).getByRole("button", { name: "Save" }).click();
await savesReach(5);
s = await saves();
ok(s.length === 5 && s[4].guid === "dl1" && s[4].endsAt === s[4].startsAt, `a deadline stays a moment  ${JSON.stringify(s[4] && { g: s[4].guid, len: s[4].endsAt - s[4].startsAt })}`);

await b.close();
console.log(failures ? `${failures} failed` : "all passed");
process.exitCode = failures ? 1 : 0;
