// Ticking a repeating reminder answers at once like any other: the circle fills the moment it
// is clicked, while Apple is still making the next one; then the row rolls on, open again with
// its next date, as on the phone. (It used to show nothing until Apple answered.)
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
await ctx.addInitScript(() => {
  const now = Math.floor(Date.now() / 1000);
  window.__extraReminders = [{ guid: "rr", listGuid: "l1", title: "Take vitamins", notes: null, dueAt: now + 3600, allDay: false,
    completedAt: null, priority: 0, flagged: false, parentGuid: null, etag: null, recurFrequency: 0, recurInterval: 1 }];
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.locator(".rx-list", { hasText: "Daily" }).click();
await page.waitForTimeout(300);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const row = page.locator(".rx-row", { has: page.getByLabel("Rename Take vitamins") });
const due = () => row.innerText();
const before = await due();
await page.evaluate(() => { window.__doneDelay = 1500; });
await row.locator(".rx-tick").click();
await page.waitForTimeout(80);
ok((await row.locator(".rx-tick").getAttribute("aria-checked")) === "true", "the circle fills at once, while Apple is still making the next one");
await page.waitForTimeout(700);
ok((await row.count()) === 1, "and the row stays — a repeating one never leaves");
await page.waitForTimeout(1500);
ok((await row.locator(".rx-tick").getAttribute("aria-checked")) === "false", "once Apple has it, it is open again");
ok((await due()) !== before, `with its next date  (${(await due()).replace(/\n/g, " ")})`);

// Clicked twice in a row (a double-click, or a second click because nothing seemed to happen):
// one tick, never two — a daily reminder must not skip a day. And when Apple answers at once,
// the circle still stays filled a moment before it rolls on.
await page.evaluate(() => { window.__doneDelay = 0; window.__done = []; });
const sent = () => page.evaluate(() => (window.__done ?? []).filter((d) => d.guid === "rr" && d.done).length);
await row.locator(".rx-tick").dblclick();
await page.waitForTimeout(300);
ok((await sent()) === 1, `a double-click ticks it once  (${await sent()} sent)`);
ok((await row.locator(".rx-tick").getAttribute("aria-checked")) === "true", "and it stays ticked a moment, even with Apple's answer back");
await row.locator(".rx-tick").click();
await page.waitForTimeout(200);
ok((await sent()) === 1, "a second click while it settles is not a second tick");
await page.waitForTimeout(2600);
ok((await row.locator(".rx-tick").getAttribute("aria-checked")) === "false", "then it rolls on, open for its next day");
await row.locator(".rx-tick").click();
await page.waitForTimeout(200);
ok((await sent()) === 2, "and a deliberate tick after that counts again");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
