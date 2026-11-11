// What Reminders shows stays the newest truth (desktop audit H3, M3, M4, H2):
// - a tick's read after it lands reads the list on screen now, not the one it was made in;
// - a title being typed survives a sync that renames the reminder underneath it;
// - "Tomorrow" becomes "Today" at midnight, and a reminder turns red the minute it falls due;
// - Return confirming an input method's candidate is the IME's, not the row's.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
// 23:58 on a Wednesday, two minutes before midnight, on a clock the suite moves.
const start = new Date(2026, 10, 11, 23, 58, 0);
await page.clock.install({ time: start });
await page.addInitScript((at) => {
  const s = (d) => Math.floor(d.getTime() / 1000);
  const d = new Date(at);
  const minute = new Date(d); minute.setMinutes(59);
  const tomorrow = new Date(d); tomorrow.setDate(d.getDate() + 1); tomorrow.setHours(9, 0, 0, 0);
  window.__filterLists = true;
  window.__extraReminders = [
    { guid: "x1", listGuid: "l3", title: "Last call", notes: null, dueAt: s(minute), completedAt: null, priority: 0, flagged: false, parentGuid: null, etag: null },
    { guid: "x2", listGuid: "l3", title: "Morning walk", notes: null, dueAt: s(tomorrow), completedAt: null, priority: 0, flagged: false, parentGuid: null, etag: null },
    { guid: "x3", listGuid: "l2", title: "Stretch", notes: null, dueAt: null, completedAt: null, priority: 0, flagged: false, parentGuid: null, etag: null },
  ];
}, start.getTime());
await page.goto(process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const settle = (ms) => page.clock.runFor(ms);

// 1. Labels move with the clock.
await page.locator(".rx-list", { hasText: "Life Goals" }).first().click();
await settle(400);
const due = (title) => page.locator(".rx-row", { has: page.locator(`input[value="${title}"]`) }).locator(".rx-due").first();
const before = { last: await due("Last call").innerText(), lastRed: await due("Last call").evaluate((el) => el.classList.contains("overdue")), walk: await due("Morning walk").innerText() };
ok(/Today/.test(before.last) && !before.lastRed && /Tomorrow/.test(before.walk), `before: ${JSON.stringify(before)}`);
await page.clock.fastForward("01:30");
await settle(200);
const lastRed = await due("Last call").evaluate((el) => el.classList.contains("overdue"));
ok(lastRed, "a reminder turns red the minute it falls due");
await page.clock.fastForward("01:00");
await settle(200);
const walk = await due("Morning walk").innerText();
ok(/Today/.test(walk) && !/Tomorrow/.test(walk), `after midnight tomorrow's reminder says Today  [${walk}]`);

// 2. A tick whose read lands after another list was picked reads that list.
await page.locator(".rx-list", { hasText: "Daily" }).first().click();
await settle(400);
await page.evaluate(() => { window.__doneDelay = 800; });
await page.getByRole("checkbox", { name: "Plan tomorrow" }).click();
await settle(50);
await page.locator(".rx-list", { hasText: "GYM" }).first().click();
await settle(2500);
const shown = await page.locator(".rx-rows .rx-title").evaluateAll((els) => els.map((e) => e.value));
ok(shown.includes("Evening run") && !shown.includes("Pick up the parcel"), `GYM shows GYM's reminders after the tick lands  ${JSON.stringify(shown)}`);
await page.evaluate(() => { window.__doneDelay = 0; });

// 3. A title being typed survives a sync renaming it. (GYM picked again, so this part stands
// on its own whatever the part above found.)
await page.locator(".rx-list", { hasText: "Daily" }).first().click();
await settle(300);
await page.locator(".rx-list", { hasText: "GYM" }).first().click();
await settle(400);
const title = page.locator('.rx-row input.rx-title[value="Stretch"]').first();
await title.click();
await page.keyboard.press("End");
await page.keyboard.type(" well");
// The phone renames it meanwhile: the library's copy is a new record, as a pass stores one.
await page.evaluate(() => {
  window.__extraReminders = window.__extraReminders.map((r) => (r.guid === "x3" ? { ...r, title: "Stretch first" } : r));
  window.__emit("quince://reminders-synced", null);
});
await settle(400);
const typing = await page.evaluate(() => ({ value: document.activeElement?.value, isTitle: document.activeElement?.classList.contains("rx-title") }));
ok(typing.isTitle && typing.value === "Stretch well", `the words typed stay, and so does the cursor  ${JSON.stringify(typing)}`);

// 4. Return confirming an IME candidate leaves the field as it is.
const edits = await page.evaluate(() => (window.__edits ?? []).length);
await page.evaluate(() => {
  const el = document.activeElement;
  el.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", code: "Enter", keyCode: 229, isComposing: true, bubbles: true, cancelable: true }));
});
await settle(300);
const after = await page.evaluate(() => ({ isTitle: document.activeElement?.classList.contains("rx-title"), next: document.querySelectorAll(".rx-next").length }));
ok(after.isTitle && after.next === 0 && (await page.evaluate(() => (window.__edits ?? []).length)) === edits,
  `composing, Return neither saves nor starts the next  ${JSON.stringify(after)}`);
// Leaving it renames it to what was typed, over the sync's name.
await page.keyboard.press("Enter");
await settle(400);
const renamed = await page.evaluate(() => (window.__edits ?? []).map((e) => e.title).filter(Boolean).at(-1));
ok(renamed === "Stretch well", `and Return then saves what was typed  [${renamed}]`);

await b.close();
console.log(failures ? `\n${failures} failure(s)` : "\nall passed");
process.exit(failures ? 1 : 0);
