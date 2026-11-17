// Day view on the morning the clocks go forward: a meeting at nine is drawn on the 9 line, and
// an event made by double-clicking at nine — or dragging from nine — is made at nine. Counted as
// time since midnight, both were an hour off after the change.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const HOUR_PX = 48;
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

// 8 March 2026, New York: 2 AM becomes 3 AM. Nine o'clock that morning is 13:00 UTC.
const nine = Date.UTC(2026, 2, 8, 13) / 1000;
const ctx = await b.newContext({ viewport: { width: 1300, height: 900 }, timezoneId: "America/New_York", locale: "en-US" });
await ctx.addInitScript((e) => { window.__fixedEvents = e; }, [
  { guid: "dst1", calendarGuid: "work", title: "Spring standup", location: null, startsAt: nine, endsAt: nine + 3600,
    allDay: false, timezone: "America/New_York", url: null, notes: null, etag: null },
  // A day out, on 1 March: midnight to midnight, New York.
  { guid: "dst2", calendarGuid: "work", title: "Museum day", location: null, startsAt: Date.UTC(2026, 2, 1, 5) / 1000,
    endsAt: Date.UTC(2026, 2, 2, 5) / 1000, allDay: true, timezone: null, url: null, notes: null, etag: null },
]);
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
const page = await ctx.newPage();
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Calendar");
await page.keyboard.press("Control+1");
await page.keyboard.press("Shift+Control+T");
await page.getByRole("dialog", { name: "Go to Date" }).getByLabel("Date").fill("2026-03-08");
await page.keyboard.press("Enter");
await page.waitForTimeout(600);

const block = page.locator(".cx-block", { hasText: "Spring standup" });
const top = await block.evaluate((el) => parseFloat(el.style.top)).catch(() => NaN);
ok(top === 9 * HOUR_PX, `the nine o'clock meeting is drawn on the 9 line  (top ${top}px, wanted ${9 * HOUR_PX}px)`);

// A double-click at half past eleven, well clear of the meeting.
const columns = page.locator(".cx-columns");
await page.locator(".cx-scroll").evaluate((el) => { el.scrollTop = 8 * 48; });
await page.waitForTimeout(100);
const box = await columns.boundingBox();
await page.mouse.dblclick(box.x + box.width / 2, box.y + 11.5 * HOUR_PX + 4);
await page.waitForTimeout(300);
const made = page.locator(".cx-block.is-draft .cx-block-time");
const said = (await made.count()) ? (await made.first().innerText()).trim() : "(none)";
ok(/^11:30\s?AM$/i.test(said), `a double-click at 11:30 makes an event at 11:30  (${said})`);
const drawn = await page.locator(".cx-block.is-draft").first().evaluate((el) => parseFloat(el.style.top)).catch(() => NaN);
ok(drawn === 11.5 * HOUR_PX, `and draws it where it was made  (top ${drawn}px)`);
await page.keyboard.press("Escape");
await page.waitForTimeout(200);

// The day out moved to 8 March, the 23-hour day: still that one day, not on into the 9th.
await page.keyboard.press("Shift+Control+T");
await page.getByRole("dialog", { name: "Go to Date" }).getByLabel("Date").fill("2026-03-01");
await page.keyboard.press("Enter");
await page.waitForTimeout(400);
await page.locator(".cx-allday .cx-bar-event", { hasText: "Museum day" }).click();
await page.waitForTimeout(300);
await page.getByLabel("Starts date").fill("2026-03-08");
await page.waitForTimeout(150);
const ends = await page.getByLabel("Ends date").inputValue().catch(() => "(none)");
ok(ends === "2026-03-08", `an all-day event moved onto the clock's change keeps its one day  (ends ${ends})`);
await page.keyboard.press("Escape");

await b.close();
process.exitCode = failures ? 1 : 0;
