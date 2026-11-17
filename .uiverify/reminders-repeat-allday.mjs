// A daily reminder for the day, with no time, ticked: it rolls on to tomorrow and says so —
// "Moved to Tomorrow", as a day — never with a time it does not have ("Tomorrow 12:00 AM").
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
await ctx.addInitScript(() => {
  const d = new Date();
  // Today, as Apple keeps a day: its midnight in UTC.
  const day = Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()) / 1000;
  window.__extraReminders = [{ guid: "rd", listGuid: "l1", title: "Water the herbs", notes: null, dueAt: day, allDay: true,
    completedAt: null, priority: 0, flagged: false, parentGuid: null, etag: null, recurFrequency: 0, recurInterval: 1 }];
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); failures++; });
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.locator(".rx-list", { hasText: "Daily" }).click();
await page.waitForTimeout(300);
const row = page.locator(".rx-row", { has: page.getByLabel("Rename Water the herbs") });
await row.locator(".rx-tick").click();
const moved = row.locator(".rx-moved");
await moved.waitFor({ timeout: 4000 }).catch(() => {});
const said = (await moved.count()) ? (await moved.innerText()).trim() : "(nothing)";
ok(said === "Moved to Tomorrow", `it says where it went, as a day  ${JSON.stringify(said)}`);
await b.close();
process.exitCode = failures ? 1 : 0;
