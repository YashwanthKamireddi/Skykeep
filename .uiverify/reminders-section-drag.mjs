// A reminder dragged onto a reminder in another section of its list goes into that section,
// where it was dropped — as on the phone — and Apple is told both: its place and its section.
// It had been put in order and left in its old section, so it sprang back under its old heading.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
await ctx.addInitScript(() => {
  window.__filterLists = true;
  window.__sections = [
    { guid: "s1", listGuid: "l1", title: "Errands", createdAt: 1 },
    { guid: "s2", listGuid: "l1", title: "At home", createdAt: 2 },
  ];
  const r = (guid, title, sectionGuid) => ({ guid, listGuid: "l1", title, notes: null, dueAt: null, completedAt: null,
    priority: 0, flagged: false, parentGuid: null, sectionGuid, recurFrequency: null, recurInterval: null, etag: null });
  window.__extraReminders = [r("x1", "Return the library books", "s1"), r("x2", "Buy oat milk", "s1"), r("x3", "Fix the balcony light", "s2")];
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); failures++; });
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.locator(".rx-list", { hasText: "Daily" }).click();
await page.waitForTimeout(400);
const layout = () => page.evaluate(() => {
  const out = [];
  let heading = null;
  for (const el of document.querySelectorAll(".rx-main .rx-section, .rx-main .rx-row")) {
    if (el.classList.contains("rx-section")) heading = el.textContent.trim();
    else out.push([heading, el.querySelector(".rx-title")?.value]);
  }
  return out;
});
// Dragged as the browser drags a row: start, over the row it is dropped on, drop, end.
await page.evaluate(() => {
  const row = (t) => [...document.querySelectorAll(".rx-row")].find((r) => r.querySelector(".rx-title")?.value === t);
  const from = row("Fix the balcony light"), to = row("Buy oat milk");
  const dt = new DataTransfer();
  from.dispatchEvent(new DragEvent("dragstart", { bubbles: true, dataTransfer: dt }));
  to.dispatchEvent(new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer: dt }));
  to.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: dt }));
  from.dispatchEvent(new DragEvent("dragend", { bubbles: true, dataTransfer: dt }));
});
await page.waitForTimeout(80);
const now = await layout();
const at = now.find(([, t]) => t === "Fix the balcony light");
const i = now.findIndex(([, t]) => t === "Fix the balcony light"), j = now.findIndex(([, t]) => t === "Buy oat milk");
ok(at?.[0] === "Errands" && i === j - 1, `it is under Errands at once, just before the one dropped on  ${JSON.stringify(now)}`);
await page.waitForTimeout(900);
const settled = await layout();
ok(settled.find(([, t]) => t === "Fix the balcony light")?.[0] === "Errands", `and stays there  ${JSON.stringify(settled)}`);
const told = await page.evaluate(() => ({ reorder: window.__reorders?.at(-1), section: (window.__detailWrites ?? []).filter((w) => w.cmd === "reminder_section_move").at(-1) }));
ok(told.reorder?.before === "x2" && told.section?.section === "s1", `Apple is told its place and its section  ${JSON.stringify(told)}`);
await b.close();
process.exitCode = failures ? 1 : 0;
