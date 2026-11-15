// Month's all-day and multi-day bars stay inside their week. In a short window a week has room
// under its dates for a lane or two; the bars past that ran on over the weeks below, across their
// dates and their own bars. Now only the lanes that fit are drawn, and each day counts the bars
// it lost in its "N more" (which opens the day, every bar in its all-day row). With room, every
// bar is drawn across exactly the days it covers.
//
//   URL=http://localhost:4173 ENGINE=webkit node .uiverify/calendar-month-bars.mjs
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

for (const [width, height] of [[1300, 1200], [1300, 850], [1300, 520], [760, 600]]) {
  const ctx = await b.newContext({ viewport: { width, height } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true,
    extraEvents: [
      // From today, nine days: across into next week.
      { guid: "m1", title: "Conference", hour: -9, length: 24 * 9, allDay: true },
      { guid: "m2", title: "Trip", day: 1, hour: -9, length: 24 * 2, allDay: true },
      { guid: "m3", title: "Two days", day: 2, hour: 1, length: 30 },
      { guid: "m4", title: "Workshop", day: 1, hour: -9, length: 24 * 3, allDay: true },
    ] });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
  await page.goto(process.env.URL ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await openScreen(page, "Calendar");
  await page.keyboard.press("Control+3");
  await page.locator(".cx-week").first().waitFor();
  await page.waitForTimeout(500);
  const r = await page.evaluate(() => {
    const out = { spill: [], wrongDays: [], drawn: 0, more: 0, titles: [] };
    for (const week of document.querySelectorAll(".cx-week")) {
      const wr = week.getBoundingClientRect();
      const days = [...week.querySelectorAll(".cx-day")].map((d) => d.getBoundingClientRect());
      for (const bar of week.querySelectorAll(".cx-bar-event")) {
        out.drawn++;
        out.titles.push(bar.textContent);
        const br = bar.getBoundingClientRect();
        if (br.top < wr.top - 0.5 || br.bottom > wr.bottom + 0.5) out.spill.push(bar.textContent);
        // Its ends inside the first and last day it covers, never past a day's edge.
        const first = days.findIndex((d) => br.left >= d.left - 0.5 && br.left < d.right);
        const last = days.findIndex((d) => br.right > d.left && br.right <= d.right + 0.5);
        if (first < 0 || last < first) out.wrongDays.push(bar.textContent);
      }
      for (const m of week.querySelectorAll(".cx-more")) out.more += Number(/\d+/.exec(m.textContent)?.[0] ?? 0);
    }
    return out;
  });
  const label = `${width}×${height}`;
  ok(r.spill.length === 0, `${label}: no bar runs out of its week  ${JSON.stringify(r.spill)}`);
  ok(r.wrongDays.length === 0, `${label}: each bar starts and ends inside a day it covers  ${JSON.stringify(r.wrongDays)}`);
  // How many pieces the bars break into depends on the weekday today is (a bar is cut at each
  // week's end), so what's checked is the promise: with room, every one is drawn, none hidden.
  const every = ["Conference", "Trip", "Two days", "Workshop"].every((t) => r.titles.some((x) => x.includes(t)));
  if (height === 1200) ok(every && r.more === 0, `${label}: with room, every bar is drawn  (${r.drawn} bars, ${r.more} more)`);
  else ok(r.more > 0, `${label}: the bars that do not fit are counted in "N more"  (${r.drawn} bars, ${r.more} more)`);
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
