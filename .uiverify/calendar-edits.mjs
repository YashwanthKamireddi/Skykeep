// Working with events as in Calendar: the picked event opens on Return, deletes on Delete
// (asked first), duplicates on Ctrl+D, copies and pastes onto another day, moves calendar from
// its menu, drags to another day in Month, stretches from its top edge, and opens its place in
// Maps. A calendar's default alert goes on each new event made in it.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1300, height: 850 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
const page = await ctx.newPage();
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Calendar");
await page.keyboard.press("Control+2");
await page.waitForTimeout(300);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const saves = () => page.evaluate(() => window.__eventSaves ?? []);
const local = (n, h, m = 0) => page.evaluate(([n, h, m]) => { const d = new Date(); d.setHours(h, m, 0, 0); d.setDate(d.getDate() + n); return d.getTime() / 1000; }, [n, h, m]);
const fullDate = (n) => page.evaluate((n) => { const d = new Date(); d.setDate(d.getDate() + n); return new Intl.DateTimeFormat(undefined, { dateStyle: "full" }).format(d); }, n);
const ten = await local(0, 10);

// Picked, then Return opens it again.
const review = page.locator(".cx-block", { hasText: "Design review" });
await review.click();
await page.waitForTimeout(200);
const pop = page.getByRole("dialog", { name: "Design review" });
ok(await pop.isVisible(), "a click opens the event");
ok((await pop.getByRole("link", { name: "Open in Maps" }).getAttribute("href")) === "geo:0,0?q=Studio", "its place has Open in Maps");
await page.keyboard.press("Escape");
ok(await pop.count() === 0 && await review.evaluate((el) => el.classList.contains("is-picked")), "Escape closes it, and it stays picked");
await page.keyboard.press("Enter");
await page.waitForTimeout(200);
ok(await pop.isVisible(), "Return opens the picked event");
await page.keyboard.press("Escape");

// Ctrl+D: a copy at the same time, in the same calendar.
await page.keyboard.press("Control+d");
await page.waitForTimeout(300);
let s = await saves();
ok(s.length === 1 && s[0].guid === null && s[0].title === "Design review" && s[0].startsAt === ten && s[0].calendarGuid === "work",
  `Ctrl+D duplicates it  ${JSON.stringify(s[0])}`);

// Ctrl+C, then Ctrl+V on another day: the copy at its own time of day.
await page.keyboard.press("Control+c");
await page.keyboard.press("Control+3");
await page.waitForTimeout(300);
await page.getByRole("gridcell").filter({ has: page.getByRole("button", { name: new RegExp(`^${await fullDate(2)}`) }) }).locator(".cx-day-hit").click();
await page.keyboard.press("Control+v");
await page.waitForTimeout(300);
s = await saves();
const two = await local(2, 10);
ok(s.length === 2 && s[1].guid === null && s[1].startsAt === two && s[1].endsAt === two + 3600, `Ctrl+V pastes it on the selected day  ${JSON.stringify(s[1])}`);

// Month: dragged from tomorrow to three days on, the clock time kept. Back to this month first:
// the paste above selected a day two on, and when that day opens the next month (on 30 October
// 2026, 1 November begins a month that starts on a Sunday) tomorrow is not on its page.
await page.getByRole("button", { name: "Today" }).click();
await page.waitForTimeout(300);
const sprint = page.locator(".cx-item", { hasText: "Sprint planning" });
const target = page.getByRole("gridcell").filter({ has: page.getByRole("button", { name: new RegExp(`^${await fullDate(3)}`) }) });
const from = await sprint.boundingBox();
const to = await target.boundingBox();
await page.mouse.move(from.x + 20, from.y + from.height / 2);
await page.mouse.down();
await page.mouse.move(from.x + 40, from.y + from.height / 2 + 4, { steps: 3 });
await page.mouse.move(to.x + to.width / 2, to.y + to.height - 12, { steps: 8 });
ok(await target.evaluate((el) => el.classList.contains("is-drop")), "the day under the pointer is marked");
await page.mouse.up();
await page.waitForTimeout(300);
s = await saves();
const sprintAt = await local(1, 11);
ok(s.length === 3 && s[2].guid === "e3" && s[2].startsAt === sprintAt + 2 * 86400 && s[2].endsAt === sprintAt + 2 * 86400 + 3600,
  `dragged two days on  ${JSON.stringify(s[2] && { g: s[2].guid, d: (s[2].startsAt - sprintAt) / 3600 })}`);
ok(await page.getByRole("dialog").count() === 0, "a drag does not open the event");

// Its menu moves it to another calendar.
await page.locator(".cx-item", { hasText: "Design review" }).click({ button: "right" });
await page.getByRole("menuitem", { name: "Calendar" }).hover();
await page.getByRole("menuitemcheckbox", { name: "Personal" }).click();
await page.waitForTimeout(300);
s = await saves();
ok(s.at(-1)?.guid === "e1" && s.at(-1)?.calendarGuid === "personal", "Calendar ▸ Personal moves it");

// Delete, from the keyboard, asks first. In Week, where every event of the day is drawn: a
// Month day shows as many lines as fit, and today has grown by the copies made above.
await page.keyboard.press("Control+2");
await page.waitForTimeout(300);
// Back to today's week: the paste above picked a day two on, which late in the week is
// next week, where today's Dentist isn't drawn.
await page.getByRole("button", { name: "Today", exact: true }).click();
await page.waitForTimeout(300);
await page.locator(".cx-block", { hasText: "Dentist" }).first().click();
await page.waitForTimeout(200);
await page.keyboard.press("Escape");
await page.keyboard.press("Delete");
const ask = page.getByRole("alertdialog");
ok(/Delete .Dentist/.test(await ask.innerText()), "Delete asks first");
await ask.getByRole("button", { name: "Cancel" }).click();
ok((await page.evaluate(() => window.__eventDeletes ?? [])).length === 0, "Cancel keeps it");
await page.keyboard.press("Delete");
await page.getByRole("alertdialog").getByRole("button", { name: "Delete" }).click();
await page.waitForTimeout(300);
ok((await page.evaluate(() => window.__eventDeletes ?? [])).some((d) => d.guid === "e2"), "confirmed, it is deleted");

// Week: the top edge moves the start and leaves the end.
await page.keyboard.press("Control+2");
await page.waitForTimeout(300);
await review.evaluate((el) => el.scrollIntoView({ block: "center" }));
const block = await review.boundingBox();
await page.mouse.move(block.x + block.width / 2, block.y + 2);
await page.mouse.down();
await page.mouse.move(block.x + block.width / 2, block.y + 2 - 48, { steps: 6 });
await page.mouse.up();
await page.waitForTimeout(300);
s = await saves();
ok(s.at(-1)?.guid === "e1" && s.at(-1)?.startsAt === ten - 3600 && s.at(-1)?.endsAt === ten + 3600,
  `the top edge starts it an hour earlier  ${JSON.stringify(s.at(-1) && [(s.at(-1).startsAt - ten) / 60, (s.at(-1).endsAt - ten) / 60])}`);

// A default alert for Home goes on each new event made there.
await page.getByRole("complementary", { name: "Calendars" }).getByText("Home").click({ button: "right" });
await page.getByRole("menuitem", { name: "Default Alert", exact: true }).hover();
await page.getByRole("menuitemcheckbox", { name: "15 minutes before" }).click();
await page.getByRole("button", { name: "New event" }).click();
await page.keyboard.type("Call the bank tomorrow 3pm");
await page.keyboard.press("Enter");
await page.waitForTimeout(300);
s = await saves();
ok(JSON.stringify(s.at(-1)?.alerts) === "[15]", `a new event takes Home's default alert  ${JSON.stringify(s.at(-1)?.alerts)}`);
await b.close();
process.exitCode = failures ? 1 : 0;
