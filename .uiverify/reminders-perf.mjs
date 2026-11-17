// What Reminders and Calendar ask the backend for, and how long a key takes, measured against
// the stub: a search keystroke, another service's outbox tick, a reminders tick's tag sweep,
// and a key typed into New Reminder with a thousand rows on screen. Prints the numbers; fails
// where the screen asks for more than it needs.
//
//   BASE=http://localhost:4173 ENGINE=webkit node .uiverify/reminders-perf.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true, calendarFixture: true });
await ctx.addInitScript(() => {
  // Every command the window sends, counted by name.
  const wrap = () => {
    const t = window.__TAURI_INTERNALS__;
    if (!t || t.__counted) return;
    const inner = t.invoke;
    window.__calls = {};
    t.invoke = (cmd, args) => {
      window.__calls[cmd] = (window.__calls[cmd] ?? 0) + 1;
      return inner(cmd, args);
    };
    t.__counted = true;
  };
  wrap(); document.addEventListener("DOMContentLoaded", wrap);
  // A thousand rows, as a busy list holds them: one invented reminder each.
  const now = Math.floor(Date.now() / 1000);
  window.__extraReminders = Array.from({ length: 1000 }, (_, i) => ({
    guid: `bulk-${i}`, listGuid: "l1", title: `Errand number ${i + 1}`, notes: null,
    dueAt: now + (i + 1) * 3600, completedAt: null, priority: 0, flagged: false, parentGuid: null, etag: null,
  }));
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); failures++; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
// Calendar first, so it stays mounted behind Reminders as it does once visited.
await openScreen(page, "Calendar");
await page.waitForTimeout(500);
await openScreen(page, "Reminders");
await page.locator(".rx-list", { hasText: "Daily" }).first().click();
await page.waitForFunction(() => document.querySelectorAll(".rx-rows .rx-row").length >= 1000, null, { timeout: 15000 });
await page.waitForTimeout(500);

const reset = () => page.evaluate(() => { window.__calls = {}; });
const calls = () => page.evaluate(() => ({ ...window.__calls }));
const sum = (c, names) => names.reduce((n, k) => n + (c[k] ?? 0), 0);

// 1. Another service's change landing: neither Reminders nor Calendar reads anything again.
await reset();
await page.evaluate(() => window.__emit("quince://outbox-changed", { services: ["notes"] }));
await page.waitForTimeout(600);
let c = await calls();
const notesTick = sum(c, ["reminders_in", "reminder_marks", "reminder_tags", "reminders_tagged", "reminder_extras", "calendar_events", "calendar_list", "reminders_due_between"]);
console.log(`${engineName}: a Notes outbox tick → ${notesTick} Reminders/Calendar reads ${JSON.stringify(c)}`);
ok(notesTick === 0, "a Notes change landing reads nothing in Reminders or Calendar");

// 2. A reminders change landing: the tag and smart-list sweep is one call, not one per tag.
await reset();
await page.evaluate(() => window.__emit("quince://outbox-changed", { services: ["reminders"] }));
await page.waitForTimeout(600);
c = await calls();
const sweep = sum(c, ["reminders_tagged", "reminders_in_smart_list", "reminder_extras"]);
console.log(`${engineName}: a Reminders outbox tick → tag/smart sweep ${sweep} calls, reminders_in ${c.reminders_in ?? 0}, marks ${c.reminder_marks ?? 0}`);
ok(sweep <= 1, "the tags and smart-list counts are read in one call");
ok((c.reminders_in ?? 0) >= 1, "a reminders change landing still reads the list again");

// 3. Searching: one search per pause, not one per key, and no listeners torn down per key.
await reset();
const search = page.getByLabel("Search reminders").first();
await search.click();
for (const ch of "errand") {
  await page.keyboard.type(ch);
  await page.waitForTimeout(30);
}
await page.waitForTimeout(500);
c = await calls();
console.log(`${engineName}: typing "errand" → reminders_search ${c.reminders_search ?? 0}, marks ${c.reminder_marks ?? 0}, listen ${c["plugin:event|listen"] ?? 0}, unlisten ${c["plugin:event|unlisten"] ?? 0}`);
ok((c.reminders_search ?? 0) <= 2, "a word typed quickly is searched once or twice, not per key");
ok((c["plugin:event|listen"] ?? 0) === 0, "typing a search sets up no event listeners again");
ok(await page.locator(".rx-rows .rx-row").count() >= 1000, "the results are shown");
await search.fill("");
await page.waitForTimeout(400);

// 4. A key typed into New Reminder with 1,000 rows: time from the key to the screen settled.
const field = page.getByPlaceholder(/New Reminder/i).first();
await field.click();
const times = await page.evaluate(async () => {
  const input = document.querySelector(".rx-add input");
  const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
  const out = [];
  let text = "";
  for (const ch of "Buy oat milk") {
    text += ch;
    const t0 = performance.now();
    set.call(input, text);
    input.dispatchEvent(new Event("input", { bubbles: true }));
    await new Promise((r) => setTimeout(r, 0));
    out.push(performance.now() - t0);
    await new Promise((r) => setTimeout(r, 30));
  }
  return out;
});
const mean = times.reduce((a, x) => a + x, 0) / times.length;
console.log(`${engineName}: a key in New Reminder with 1,000 rows → mean ${mean.toFixed(1)} ms, worst ${Math.max(...times).toFixed(1)} ms`);
ok(mean < 16, "a key typed with 1,000 rows on screen settles within a frame");
await field.fill("");

// 5. Dragging over rows marks where it lands without drawing the list again.
const marked = await page.evaluate(async ([a, z]) => {
  const rowsNow = [...document.querySelectorAll(".rx-rows .rx-row")];
  const from = rowsNow[a], to = rowsNow[z];
  const dt = new DataTransfer();
  from.dispatchEvent(new DragEvent("dragstart", { bubbles: true, dataTransfer: dt }));
  await new Promise((r) => setTimeout(r, 50));
  to.dispatchEvent(new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer: dt }));
  await new Promise((r) => setTimeout(r, 50));
  const on = to.classList.contains("is-drop-before");
  const only = document.querySelectorAll(".rx-row.is-drop-before").length;
  rowsNow[z + 1].dispatchEvent(new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer: dt }));
  await new Promise((r) => setTimeout(r, 50));
  const moved = rowsNow[z + 1].classList.contains("is-drop-before") && !to.classList.contains("is-drop-before");
  from.dispatchEvent(new DragEvent("dragend", { bubbles: true, dataTransfer: dt }));
  await new Promise((r) => setTimeout(r, 50));
  const cleared = document.querySelectorAll(".rx-row.is-drop-before").length === 0;
  return { on, only, moved, cleared };
}, [2, 5]);
ok(marked.on && marked.only === 1, "dragging over a row marks it as where the reminder lands");
ok(marked.moved, "the mark follows the pointer to the next row");
ok(marked.cleared, "the mark goes when the drag ends");

// 6. Urgent read from Apple after the marks came back: merged into its row as it arrives.
const urgentRow = page.locator('.rx-row[data-guid="bulk-0"]');
await urgentRow.scrollIntoViewIfNeeded();
ok((await urgentRow.locator(".rx-urgent").count()) === 0, "a row shows no Urgent before it is known");
await page.evaluate(() => window.__emit("quince://reminder-urgent", [{ guid: "bulk-0", urgent: true }, { guid: "bulk-1", urgent: false }]));
await page.waitForTimeout(200);
ok((await urgentRow.locator(".rx-urgent").count()) === 1, "Urgent read later shows on its row");
ok((await page.locator('.rx-row[data-guid="bulk-1"] .rx-urgent').count()) === 0, "a row read as not Urgent stays without it");

await b.close();
process.exitCode = failures ? 1 : 0;
