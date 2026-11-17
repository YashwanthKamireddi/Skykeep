// One occurrence of a repeating event is changed or deleted only once Calendar's question is
// answered — "only this event" — and is then sent as that occurrence alone.
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

await page.locator(".cx-block", { hasText: "Weekly standup" }).first().click();
await page.waitForTimeout(250);
const title = page.locator('input[placeholder="New Event"]').first();
await title.fill("Weekly standup, moved room");
await title.press("Enter");
await page.waitForTimeout(250);
ok((await page.getByRole("alertdialog").count()) === 1, "changing one occurrence asks first");
await page.keyboard.press("Escape");
await page.waitForTimeout(200);
ok((await page.evaluate(() => (window.__eventSaves ?? []).length)) === 0, "Cancel sends nothing");

await page.locator(".cx-block", { hasText: "Weekly standup" }).first().click();
await page.waitForTimeout(250);
await page.locator('input[placeholder="New Event"]').first().fill("Weekly standup, moved room");
await page.locator('input[placeholder="New Event"]').first().press("Enter");
await page.waitForTimeout(250);
await page.getByRole("button", { name: "Only This Event", exact: true }).click();
await page.waitForTimeout(300);
const saved = await page.evaluate(() => window.__eventSaves?.at(-1));
ok(saved?.scope === "this" && saved?.guid === "SERIES__20261020T040000Z", `and "only this event" sends that occurrence alone  ${JSON.stringify({ g: saved?.guid, s: saved?.scope })}`);

await page.locator(".cx-block", { hasText: "Weekly standup" }).first().click();
await page.waitForTimeout(250);
await page.evaluate(() => document.activeElement?.blur());
await page.keyboard.press("Delete");
await page.waitForTimeout(250);
await page.getByRole("button", { name: "Delete Only This Event" }).click();
await page.waitForTimeout(300);
const del = await page.evaluate(() => window.__eventDeletes?.at(-1));
ok(del?.scope === "this", `deleting one occurrence deletes it alone  ${JSON.stringify(del)}`);
// All events: the change goes to the whole series.
await page.locator(".cx-block", { hasText: "Weekly standup" }).first().click();
await page.waitForTimeout(250);
await page.locator('input[placeholder="New Event"]').first().fill("Weekly sync");
await page.locator('input[placeholder="New Event"]').first().press("Enter");
await page.waitForTimeout(250);
await page.getByRole("button", { name: "All Events", exact: true }).click();
await page.waitForTimeout(300);
const all = await page.evaluate(() => window.__eventSaves?.at(-1));
ok(all?.scope === "all", `and "all events" sends the change for the series  ${JSON.stringify({ s: all?.scope })}`);
// All future events: from this day on, the days before kept.
await page.locator(".cx-block", { hasText: "Weekly" }).first().click();
await page.waitForTimeout(250);
await page.locator('input[placeholder="New Event"]').first().fill("Weekly standup, new room");
await page.locator('input[placeholder="New Event"]').first().press("Enter");
await page.waitForTimeout(250);
const buttons = await page.getByRole("alertdialog").getByRole("button").allInnerTexts();
ok(buttons.join("|") === "Only This Event|All Future Events|All Events|Cancel", `the question offers this, future, all, as macOS stacks them  ${JSON.stringify(buttons)}`);
await page.getByRole("button", { name: "All Future Events", exact: true }).click();
await page.waitForTimeout(300);
const future = await page.evaluate(() => window.__eventSaves?.at(-1));
ok(future?.scope === "future" && future?.guid === "SERIES__20261020T040000Z", `and "all future events" sends that, from this occurrence  ${JSON.stringify({ s: future?.scope })}`);
// Repeat: an occurrence shows its series' repeat; a one-off can be made to repeat.
await page.locator(".cx-block", { hasText: "Weekly" }).first().click();
await page.waitForTimeout(300);
ok(/Every Week/.test(await page.getByRole("button", { name: "Repeat", exact: true }).innerText()), "an occurrence shows how its series repeats");
await page.keyboard.press("Escape");
await page.locator(".cx-block", { hasText: "Dentist" }).first().click();
await page.waitForTimeout(300);
await page.getByRole("button", { name: "Repeat", exact: true }).click();
await page.getByRole("option", { name: "Every Day" }).click();
await page.locator('input[placeholder="New Event"]').first().press("Enter");
await page.waitForTimeout(300);
const daily = await page.evaluate(() => window.__eventSaves?.at(-1));
ok(daily?.repeatChanged === true && daily?.repeat?.freq === "daily" && !daily?.scope, `a one-off made to repeat every day  ${JSON.stringify({ r: daily?.repeat, c: daily?.repeatChanged })}`);
// Stopping a series from repeating is a change to all of it — no question needed.
await page.locator(".cx-block", { hasText: "Weekly" }).first().click();
await page.waitForTimeout(300);
await page.getByRole("button", { name: "Repeat", exact: true }).click();
await page.getByRole("option", { name: "Never" }).click();
await page.locator('input[placeholder="New Event"]').first().press("Enter");
await page.waitForTimeout(300);
const never = await page.evaluate(() => window.__eventSaves?.at(-1));
ok(never?.scope === "all" && never?.repeat === null && (await page.getByRole("alertdialog").count()) === 0, `a series set to Never, for all of it  ${JSON.stringify({ s: never?.scope, r: never?.repeat })}`);
// Deleting from this day on.
await page.locator(".cx-block", { hasText: "Weekly" }).first().click();
await page.waitForTimeout(250);
await page.evaluate(() => document.activeElement?.blur());
await page.keyboard.press("Delete");
await page.waitForTimeout(250);
await page.getByRole("button", { name: "Delete All Future Events" }).click();
await page.waitForTimeout(300);
const gone = await page.evaluate(() => window.__eventDeletes?.at(-1));
ok(gone?.scope === "future", `"delete all future events" sends that  ${JSON.stringify(gone)}`);

// Repeats Apple holds beyond "every week" are shown in words, as Apple's Calendar says them,
// never as a plain "Every Week".
for (const [repeat, words] of [
  [{ freq: "monthly", interval: 1, count: null, position: "2nd", weekDays: ["TU"] }, "Every month on the 2nd Tuesday"],
  [{ freq: "weekly", interval: 2, count: null, weekDays: ["TU", "TH"], until: "2026-12-15" }, "Every 2 weeks on Tue and Thu, until"],
  [{ freq: "monthly", interval: 1, count: 6, monthDays: [1, 15] }, "Every month on the 1st and 15th, 6 times"],
  [{ freq: "yearly", interval: 1, count: null, position: "4th", weekDays: ["TH"], months: [11] }, "Every year on the 4th Thursday of November"],
]) {
  await page.keyboard.press("Escape");
  await page.evaluate((r) => { window.__repeat = r; }, repeat);
  await page.locator(".cx-block", { hasText: "Weekly" }).first().click();
  await page.waitForTimeout(350);
  const shown = await page.getByText(words).count();
  ok(shown > 0, `a ${repeat.freq} repeat reads "${words}…"`);
}
await b.close();
process.exitCode = failures ? 1 : 0;
