// Scheduled, by day: a subtask due another day than its reminder stays directly under it,
// rather than being drawn nested under whatever ends that other day.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(() => {
  const d = new Date();
  const at = (days, h) => Math.floor(new Date(d.getFullYear(), d.getMonth(), d.getDate() + days, h).getTime() / 1000);
  const r = (guid, title, dueAt, parentGuid = null) => ({ guid, listGuid: "l1", title, notes: null, dueAt, allDay: false,
    completedAt: null, priority: 0, flagged: false, parentGuid, etag: null });
  window.__smartExtra = [r("par", "Plan the move", at(3, 10)), r("kid", "Book the van", at(5, 10), "par"), r("other", "Dentist", at(5, 9))];
});
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); failures++; });
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.locator(".rx-smart, .rx-tile, button", { hasText: "Scheduled" }).first().click();
await page.waitForTimeout(600);
const layout = await page.evaluate(() => {
  const out = [];
  let heading = null;
  for (const el of document.querySelectorAll(".rx-section, .rx-row")) {
    if (el.classList.contains("rx-section")) heading = el.textContent.trim();
    else out.push([heading, el.querySelector(".rx-title")?.value]);
  }
  return out;
});
const p = layout.findIndex(([, t]) => t === "Plan the move"), k = layout.findIndex(([, t]) => t === "Book the van");
ok(p >= 0 && k === p + 1 && layout[p][0] === layout[k][0], `the subtask is under its reminder  ${JSON.stringify(layout)}`);
await b.close();
process.exitCode = failures ? 1 : 0;
