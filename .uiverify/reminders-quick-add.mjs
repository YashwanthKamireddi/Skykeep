// A reminder typed in one line: what was understood shows before Return, and Return saves the
// title with its day, time, priority and repeat — or, after Add as Typed, the line exactly as
// it was typed. A plain line shows nothing extra.
//
//   BASE=http://localhost:4173 ENGINE=webkit node .uiverify/reminders-quick-add.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };

const ctx = await b.newContext({ viewport: { width: 1300, height: 850 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.locator(".rx-side .rx-list").first().click().catch(() => {});
await page.waitForTimeout(400);
const field = page.locator(".rx-add input");
await field.click();

await field.fill("Buy milk");
ok((await page.locator(".rx-add-understood").count()) === 0, "a plain line shows nothing extra");

await field.fill("!! Call mom tomorrow 6pm daily");
const reading = page.locator(".rx-add-understood .rx-add-reading");
await reading.waitFor();
const said = await reading.innerText();
ok(/Tomorrow/.test(said) && /6:00/.test(said) && /Daily/.test(said) && /Medium priority/.test(said), `what was understood shows before Return  "${said}"`);
await field.press("Enter");
await page.waitForTimeout(600);
const created = await page.evaluate(() => window.__creates?.at(-1));
const tomorrow6 = (() => { const d = new Date(); d.setDate(d.getDate() + 1); d.setHours(18, 0, 0, 0); return Math.floor(d.getTime() / 1000); })();
ok(created?.title === "Call mom" && created?.dueAt === tomorrow6 && created?.allDay === false, `saved as "Call mom", due tomorrow 6 PM  ${JSON.stringify(created)}`);
const edits = await page.evaluate(() => window.__edits ?? []);
ok(edits.some((e) => e.priority === 5), `and given medium priority  ${JSON.stringify(edits)}`);
const repeats = await page.evaluate(() => window.__repeats ?? []);
ok(repeats.some((r) => r.repeat?.frequency === 0 && r.repeat?.interval === 1), `and set to repeat daily  ${JSON.stringify(repeats)}`);
ok((await field.inputValue()) === "", "the field is ready for the next one");

await field.fill("Read chapter friday");
await page.locator(".rx-add-understood").waitFor();
await page.getByRole("button", { name: "Add as Typed" }).click();
ok((await page.locator(".rx-add-understood").count()) === 0, "Add as Typed puts the reading away");
await field.press("Enter");
await page.waitForTimeout(600);
const typed = await page.evaluate(() => window.__creates?.at(-1));
ok(typed?.title === "Read chapter friday" && (typed?.dueAt ?? null) === null, `and the line is saved exactly as typed  ${JSON.stringify(typed)}`);

await b.close();
process.exitCode = failures ? 1 : 0;
