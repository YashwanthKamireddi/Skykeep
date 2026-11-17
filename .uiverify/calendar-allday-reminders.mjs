// A reminder for a day, not a time, in Calendar: on its own day wherever the viewer is, its card
// naming that day — not the one before, east of Greenwich — with no "12 AM" beside it in Month
// or the day's list, and found on the first day a view reads, west of Greenwich too.
//
// Apple keeps such a reminder's date as that day's midnight in UTC.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

/** A day's UTC midnight, as Apple stores an all-day date. */
const utcDay = (y, m, d) => Date.UTC(y, m, d) / 1000;
const allDay = (guid, title, dueAt) => ({ guid, listGuid: "l1", title, notes: null, dueAt, allDay: true, completedAt: null, priority: 0, etag: null });

async function open(timezoneId, due) {
  const ctx = await b.newContext({ viewport: { width: 1300, height: 900 }, timezoneId, locale: "en-GB" });
  await ctx.addInitScript((d) => { window.__dueExtra = d; }, due);
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true, remindersFixture: true });
  const page = await ctx.newPage();
  await page.goto(BASE, { waitUntil: "networkidle" });
  await openScreen(page, "Calendar");
  await page.waitForTimeout(400);
  return { ctx, page };
}

// East of Greenwich: the card names the reminder's own day.
{
  // Tomorrow, on the clock of a zone east of UTC, as Apple stores it.
  const now = new Date(new Date().toLocaleString("en-US", { timeZone: "Asia/Kolkata" }));
  const day = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1);
  const { ctx, page } = await open("Asia/Kolkata", [allDay("ad1", "Renew the passport", utcDay(day.getFullYear(), day.getMonth(), day.getDate()))]);
  await page.keyboard.press("Control+3");
  await page.waitForTimeout(400);
  const item = page.locator(".cx-item.is-reminder", { hasText: "Renew the passport" }).first();
  ok((await item.count()) === 1, "Month: the day's reminder is in its cell");
  const time = (await item.locator(".cx-item-time").count()) ? (await item.locator(".cx-item-time").innerText()).trim() : "";
  ok(!/\d/.test(time), `Month: with no time beside it  ${JSON.stringify(time)}`);
  await item.click();
  const card = page.locator(".cx-popover.is-reminder");
  await card.waitFor({ timeout: 2000 }).catch(() => {});
  const text = (await card.count()) ? await card.innerText() : "";
  // Its weekday and date, however the engine punctuates them.
  const wanted = `${day.toLocaleDateString("en-GB", { weekday: "long" })} ${day.toLocaleDateString("en-GB", { day: "numeric", month: "long" })}`;
  ok(text.replace(/,/g, "").includes(wanted), `its card names its day, ${wanted}  ${JSON.stringify(text.split("\n").slice(0, 3))}`);
  await page.keyboard.press("Escape");
  // The day's list, in Day.
  await page.keyboard.press("Control+1");
  await page.keyboard.press("Shift+Control+T");
  const pad = (n) => String(n).padStart(2, "0");
  await page.getByRole("dialog", { name: "Go to Date" }).getByLabel("Date").fill(`${day.getFullYear()}-${pad(day.getMonth() + 1)}-${pad(day.getDate())}`);
  await page.keyboard.press("Enter");
  await page.waitForTimeout(400);
  // In the all-day row, not on the hour grid's midnight line out of sight.
  const inRow = await page.locator(".cx-allday .cx-reminder", { hasText: "Renew the passport" }).count();
  const onGrid = await page.locator(".cx-columns .cx-reminder", { hasText: "Renew the passport" }).count();
  ok(inRow === 1 && onGrid === 0, `Day: in the all-day row, not on the hour grid  (row ${inRow}, grid ${onGrid})`);
  const row = page.locator(".cx-pane-reminder", { hasText: "Renew the passport" });
  const when = (await row.count()) ? (await row.locator(".cx-pane-when").innerText()).trim() : "(none)";
  ok((await row.count()) === 1 && !/\d/.test(when), `Day: in the day's list, with no time  ${JSON.stringify(when)}`);
  await ctx.close();
}

// West of Greenwich: one on the first day a view reads is there too — 1 January, in Year.
{
  const year = new Date().getFullYear();
  const { ctx, page } = await open("America/New_York", [allDay("ad2", "New year plans", utcDay(year, 0, 1))]);
  await page.keyboard.press("Control+4");
  await page.waitForTimeout(600);
  const jan1 = page.locator(".cal-year button.cx-minimonth-day").first();
  const cls = (await jan1.getAttribute("class")) ?? "";
  ok(/is-busy/.test(cls), `Year: 1 January shows it has something  ${JSON.stringify(cls)}`);
  await ctx.close();
}

await b.close();
process.exitCode = failures ? 1 : 0;
