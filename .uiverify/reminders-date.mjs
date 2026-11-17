// A reminder's date, as the phone sets one: a day, and a Time switch. Off is all day; on
// keeps a time. Each is sent as such.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.locator(".rx-list", { hasText: "GYM" }).click();
await page.waitForTimeout(300);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
await page.getByRole("button", { name: "Details for Evening run" }).click();
await page.waitForTimeout(200);
const sw = page.getByRole("switch", { name: "Time for Evening run" });
ok((await sw.isChecked()) && (await page.getByLabel("Time of day for Evening run").count()) === 1, "a timed reminder shows its date, the Time switch on, and its time");
await page.locator(".rx-switch", { has: sw }).click();
await page.waitForTimeout(200);
const off = await page.evaluate(() => window.__edits?.at(-1));
const utcMidnight = off && off.dueAt % 86400 === 0;
ok(off?.allDay === true && utcMidnight, `Time off makes it all day, sent as that day's UTC midnight  ${JSON.stringify(off)}`);
await b.close();
process.exitCode = failures ? 1 : 0;
