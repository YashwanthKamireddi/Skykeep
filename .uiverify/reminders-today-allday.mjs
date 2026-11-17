// Today, as Reminders groups it: a reminder for the day with no time sits above Morning,
// Afternoon and Tonight with the overdue ones — it has no time of day to be filed under. Taken
// as its midnight, it went under Morning, and so did every one added in Today.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(() => {
  const d = new Date();
  // Today, as Apple keeps a day: its midnight in UTC.
  const day = Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()) / 1000;
  const at = (h) => Math.floor(new Date(d.getFullYear(), d.getMonth(), d.getDate(), h).getTime() / 1000);
  window.__smartExtra = [{ guid: "allday1", listGuid: "l1", title: "Renew the parking permit", notes: null, dueAt: day, allDay: true,
    completedAt: null, priority: 0, flagged: false, parentGuid: null, etag: null },
    // A morning reminder with a subtask due in the afternoon: the subtask stays under it.
    { guid: "par", listGuid: "l1", title: "Get ready for the trip", notes: null, dueAt: at(10), allDay: false,
      completedAt: null, priority: 0, flagged: false, parentGuid: null, etag: null },
    { guid: "kid", listGuid: "l1", title: "Charge the camera", notes: null, dueAt: at(15), allDay: false,
      completedAt: null, priority: 0, flagged: false, parentGuid: "par", etag: null }];
});
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); failures++; });
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.waitForTimeout(600);
// Each row with the heading it sits under (none for the rows above the first heading).
const layout = await page.evaluate(() => {
  const out = [];
  let heading = null;
  for (const el of document.querySelectorAll(".rx-section, .rx-row")) {
    if (el.classList.contains("rx-section")) heading = el.textContent.trim();
    else out.push([heading, el.querySelector("input, textarea")?.value ?? el.textContent.trim()]);
  }
  return out;
});
const at = layout.find(([, t]) => t.startsWith("Renew the parking permit"));
ok(at && at[0] === null, `the day's reminder sits above the times of day  ${JSON.stringify(at)}  ${JSON.stringify(layout.map(([h]) => h))}`);

const parent = layout.findIndex(([, t]) => t.startsWith("Get ready for the trip"));
const kid = layout.findIndex(([, t]) => t.startsWith("Charge the camera"));
ok(parent >= 0 && kid === parent + 1 && layout[kid][0] === layout[parent][0], `a subtask stays under its reminder, in its part of the day  ${JSON.stringify([layout[parent], layout[kid]])}`);

// One added in Today is the same: due today, no time.
const field = page.locator(".rx-add input").first();
if (await field.count()) {
  await field.fill("Call the plumber");
  await field.press("Enter");
  await page.waitForTimeout(800);
  const after = await page.evaluate(() => {
    const out = [];
    let heading = null;
    for (const el of document.querySelectorAll(".rx-section, .rx-row")) {
      if (el.classList.contains("rx-section")) heading = el.textContent.trim();
      else out.push([heading, el.querySelector("input, textarea")?.value ?? el.textContent.trim()]);
    }
    return out;
  });
  const added = after.find(([, t]) => t.startsWith("Call the plumber"));
  ok(added && added[0] === null, `one added in Today sits there too  ${JSON.stringify(added)}`);
} else ok(false, "found the add field");
await b.close();
process.exitCode = failures ? 1 : 0;
