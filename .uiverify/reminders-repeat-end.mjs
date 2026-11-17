// End Repeat: ticking a repeating reminder for the last time before its end date completes it,
// as the iPhone does — it leaves the list like any finished reminder and never comes back with
// a date past its end. One with time left still rolls on. node .uiverify/reminders-repeat-end.mjs
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
await ctx.addInitScript(() => {
  const now = Math.floor(Date.now() / 1000);
  const base = { listGuid: "l1", notes: null, allDay: false, completedAt: null, priority: 0, flagged: false, parentGuid: null, etag: null, recurFrequency: 0, recurInterval: 1 };
  window.__extraReminders = [
    { ...base, guid: "last", title: "Last antibiotic", dueAt: now + 3600 },
    { ...base, guid: "more", title: "Take vitamins", dueAt: now + 3600 },
  ];
  // The backend's answer: "last" ends tonight, so its next would be past End Repeat.
  window.__ending = ["last"];
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.locator(".rx-list", { hasText: "Daily" }).click();
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const row = (t) => page.locator(".rx-row", { has: page.getByLabel(`Rename ${t}`) });

await row("Last antibiotic").locator(".rx-tick").click();
await page.waitForTimeout(80);
ok((await row("Last antibiotic").locator(".rx-tick").getAttribute("aria-checked")) === "true", "the last tick fills the circle at once");
await page.waitForTimeout(1800);
ok((await row("Last antibiotic").count()) === 0, "and it leaves the list, completed, like any finished reminder");
const sent = await page.evaluate(() => window.__done ?? []);
ok(sent.filter((d) => d.guid === "last").length === 1 && sent.find((d) => d.guid === "last").done === true, "one tick sent");
await page.waitForTimeout(2000);
ok((await row("Last antibiotic").count()) === 0, "and it never comes back with a date past its end");

const before = await row("Take vitamins").innerText();
await row("Take vitamins").locator(".rx-tick").click();
await page.waitForTimeout(2600);
ok((await row("Take vitamins").count()) === 1 && (await row("Take vitamins").locator(".rx-tick").getAttribute("aria-checked")) === "false", "one with time left still rolls on, open");
ok((await row("Take vitamins").innerText()) !== before, "to its next date");
console.log(failures ? `${failures} failed` : "all passed");
process.exitCode = failures ? 1 : process.exitCode;
await b.close();
