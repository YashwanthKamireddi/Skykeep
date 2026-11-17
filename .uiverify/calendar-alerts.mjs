// An event's alerts: read from Apple when it opens, changed from icloud.com's presets, and
// written only when changed — a save that did not touch them sends none, so a slow read can
// never clear them.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1300, height: 850 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Calendar");
await page.keyboard.press("Control+2");
await page.waitForTimeout(300);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

await page.locator(".cx-block", { hasText: "Dentist" }).first().click();
await page.waitForTimeout(300);
const shown = await page.getByRole("button", { name: "Alert", exact: true }).innerText();
ok(/15 minutes before/.test(shown), `an event opens with the alert Apple holds  [${shown.trim()}]`);
ok((await page.getByRole("button", { name: "Second alert" }).count()) === 1, "and a second alert can be added");
await page.getByRole("button", { name: "Alert", exact: true }).click();
await page.getByRole("option", { name: "1 hour before" }).click();
await page.locator('input[placeholder="New Event"]').first().press("Enter");
await page.waitForTimeout(300);
let sent = await page.evaluate(() => window.__eventSaves?.at(-1));
ok(JSON.stringify(sent?.alerts) === "[60]", `a changed alert is written  ${JSON.stringify(sent?.alerts)}`);

await page.locator(".cx-block", { hasText: "Design review" }).first().click();
await page.waitForTimeout(300);
await page.locator('input[placeholder="New Event"]').first().fill("Design review, again");
await page.locator('input[placeholder="New Event"]').first().press("Enter");
await page.waitForTimeout(300);
sent = await page.evaluate(() => window.__eventSaves?.at(-1));
ok(sent && !("alerts" in sent), `an edit that leaves the alerts alone sends none  ${JSON.stringify(Object.keys(sent ?? {}))}`);

// An all-day event's alerts are Apple's: on the day or days before, each at 9 AM.
await page.keyboard.press("Escape");
await page.keyboard.press("Control+3");
await page.waitForTimeout(300);
await page.getByText("Flight to Hyderabad").first().click();
await page.waitForTimeout(300);
await page.getByRole("button", { name: "Alert", exact: true }).click();
const offered = await page.getByRole("option").allInnerTexts();
ok(offered.includes("On day of event (9 AM)") && offered.includes("1 week before (9 AM)"), `an all-day event offers Apple's 9 AM alerts  ${JSON.stringify(offered)}`);
await page.getByRole("option", { name: "1 day before (9 AM)" }).click();
await page.locator('input[placeholder="New Event"]').first().press("Enter");
await page.waitForTimeout(300);
sent = await page.evaluate(() => window.__eventSaves?.at(-1));
ok(JSON.stringify(sent?.alerts) === "[900]", `and rings at 9 AM the day before  ${JSON.stringify(sent?.alerts)}`);
await b.close();
process.exitCode = failures ? 1 : 0;
