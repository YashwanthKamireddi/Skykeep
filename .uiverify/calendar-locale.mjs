// The week as this computer's region keeps it: Monday first and a 24-hour clock in Britain,
// Sunday first and AM/PM in the US — in Month, Week, the mini month and Year alike.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
for (const [locale, first, initial, weekendCols, twelve] of [["en-GB", "Mon", "M", [5, 6], false], ["en-US", "Sun", "S", [0, 6], true]]) {
  const ctx = await b.newContext({ viewport: { width: 1300, height: 850 }, locale });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
  const page = await ctx.newPage();
  await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await openScreen(page, "Calendar");
  await page.getByRole("button", { name: "Month", exact: true }).click();
  await page.waitForTimeout(300);
  const month = await page.evaluate(() => ({
    head: document.querySelector(".cx-weekdays span")?.textContent,
    firstCell: document.querySelector(".cx-day-hit")?.getAttribute("aria-label"),
    weekend: [...document.querySelectorAll(".cx-week:first-child .cx-day")].map((d, i) => (d.classList.contains("is-weekend") ? i : -1)).filter((i) => i >= 0),
    mini: document.querySelector(".cx-side .cx-minimonth-wd")?.textContent,
  }));
  ok(month.head === first, `${locale}: Month's first column is ${first}  [${month.head}]`);
  ok(month.firstCell?.startsWith(first === "Mon" ? "Monday" : "Sunday"), `${locale}: Month's first day is a ${first}  [${month.firstCell}]`);
  ok(JSON.stringify(month.weekend) === JSON.stringify(weekendCols), `${locale}: the weekend is shaded where it falls  [${month.weekend}]`);
  ok(month.mini === initial, `${locale}: the mini month starts on ${initial}  [${month.mini}]`);
  await page.getByRole("button", { name: "Week", exact: true }).click();
  await page.waitForTimeout(300);
  const week = await page.evaluate(() => ({
    head: document.querySelector(".cal-col-head")?.getAttribute("aria-label"),
    weekend: [...document.querySelectorAll(".cx-column")].map((d, i) => (d.classList.contains("is-weekend") ? i : -1)).filter((i) => i >= 0),
    hours: [...document.querySelectorAll(".cx-hour-labels span:not(.cx-now-label)")].map((s) => s.textContent),
  }));
  ok(week.head?.startsWith(first === "Mon" ? "Monday" : "Sunday"), `${locale}: Week starts on ${first}  [${week.head}]`);
  ok(JSON.stringify(week.weekend) === JSON.stringify(weekendCols), `${locale}: Week shades its weekend  [${week.weekend}]`);
  const one = week.hours.find((h) => /13|1\s?PM/i.test(h));
  ok(twelve ? week.hours.includes("Noon") && /PM/i.test(one ?? "") : !week.hours.includes("Noon") && /^13:00$/.test(one ?? ""),
    `${locale}: hours on a ${twelve ? "12" : "24"}-hour clock  [${week.hours.slice(10, 14).join(" | ")}]`);
  const times = await page.locator(".cx-block-time").allTextContents();
  ok(twelve ? times.some((t) => /AM|PM/i.test(t)) : times.every((t) => /^\d{2}:\d{2}$/.test(t)), `${locale}: event times read the same clock  [${times.slice(0, 3).join(" | ")}]`);
  await page.getByRole("button", { name: "Year", exact: true }).click();
  await page.waitForTimeout(300);
  const year = await page.evaluate(() => document.querySelector(".cal-mini .cx-minimonth-wd")?.textContent);
  ok(year === initial, `${locale}: Year's months start on ${initial}  [${year}]`);
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
