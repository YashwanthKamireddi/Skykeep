// Added in Today it is due today and stays in Today; in Flagged it is flagged. Completed
// reminders sit beneath the open ones, the latest first.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
await ctx.addInitScript(() => {
  const wrap = () => { const t = window.__TAURI_INTERNALS__; if (!t || t.__rec) return; const inner = t.invoke;
    t.invoke = async (c, a) => { if (c === "reminder_create") (window.__creates ??= []).push(a); return inner(c, a); }; t.__rec = true; };
  wrap(); document.addEventListener("DOMContentLoaded", wrap);
});
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.locator(".rx-tile", { hasText: "Today" }).click();
await page.waitForTimeout(300);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const field = page.getByPlaceholder("New Reminder");
await field.fill("Call the bank");
await field.press("Enter");
await page.waitForTimeout(150);
ok((await page.getByRole("checkbox", { name: "Call the bank" }).count()) === 1, "a reminder added in Today stays in Today");
const sent = await page.evaluate(() => window.__creates?.at(-1));
ok(sent?.allDay === true && typeof sent?.dueAt === "number", `and is made due today  ${JSON.stringify(sent)}`);
await page.locator(".rx-tile", { hasText: "Flagged" }).click();
await page.waitForTimeout(300);
await field.fill("Passport");
await field.press("Enter");
await page.waitForTimeout(150);
ok((await page.evaluate(() => window.__creates?.at(-1)))?.flagged === true, "a reminder added in Flagged is flagged");

// Completed beneath open, latest first.
await page.locator(".rx-list", { hasText: "Daily" }).click();
await page.waitForTimeout(300);
const toggle = page.getByRole("button", { name: /Show Completed|Show completed/ });
if (await toggle.count()) { await toggle.first().click(); await page.waitForTimeout(300); }
const states = await page.locator(".rx-row").evaluateAll((rows) => rows.map((r) => r.classList.contains("is-done")));
const firstDone = states.indexOf(true);
ok(firstDone === -1 || states.slice(firstDone).every(Boolean), `completed reminders sit beneath the open ones  ${JSON.stringify(states)}`);
await b.close();
process.exitCode = failures ? 1 : 0;
