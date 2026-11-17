// A scheduled reminder in Calendar is never a dead end: its whole title shows on hover, and a
// click — or Return — opens its card with the full title, when it is due, its list and its
// notes, wholly on screen; Escape or a click elsewhere puts it away, and "Show in Reminders"
// takes you to it. A click on it never starts a new event. In Month, Week and the day pane.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const LONG = "Sort out everything for the trip to the coast before Friday, including the tickets and the bags";
for (const theme of ["light", "dark"]) {
  const ctx = await b.newContext({ viewport: { width: 1100, height: 1400 }, colorScheme: theme });
  await ctx.addInitScript(() => { window.__longReminder = true; });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true, remindersFixture: true });
  const page = await ctx.newPage();
  await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await openScreen(page, "Calendar");
  await page.keyboard.press("Control+3");
  await page.waitForTimeout(500);
  const item = page.locator(".cx-item.is-reminder", { hasText: "Sort out everything" }).first();
  const cut = await item.locator(".cx-item-title").evaluate((el) => el.scrollWidth > el.clientWidth);
  ok(cut, `${theme}: Month cuts the long title short`);
  ok((await item.getAttribute("title")) === LONG, `${theme}: and names it whole on hover`);
  const creates = () => page.evaluate(() => (window.__eventCreates ?? window.__creates ?? []).length);
  const before = await creates();
  await item.click();
  const card = page.locator(".cx-popover.is-reminder");
  await card.waitFor({ timeout: 2000 }).catch(() => {});
  ok((await card.count()) === 1, `${theme}: a click opens its card`);
  ok((await card.locator(".cx-info-title").textContent()) === LONG, `${theme}: with the whole title`);
  const text = await card.innerText();
  ok(/Ask Alex Rivera about the car/.test(text), `${theme}: its notes`);
  ok(/\d{1,2}:\d{2}|\d{1,2}\s?(AM|PM)/i.test(text), `${theme}: when it is due  ${JSON.stringify(text.split("\n").slice(0, 4))}`);
  const box = await card.boundingBox();
  ok(box && box.x >= 0 && box.y >= 0 && box.x + box.width <= 1100 && box.y + box.height <= 1400, `${theme}: wholly on screen  ${JSON.stringify(box)}`);
  ok((await page.locator(".cx-popover:not(.is-reminder)").count()) === 0 && (await creates()) === before, `${theme}: and no new event was begun`);
  await page.keyboard.press("Escape");
  await page.waitForTimeout(150);
  ok((await card.count()) === 0, `${theme}: Escape puts it away`);
  // From the keyboard: Return on the reminder opens it too.
  await item.focus();
  await page.keyboard.press("Enter");
  await page.waitForTimeout(150);
  ok((await card.count()) === 1, `${theme}: Return opens it`);
  await page.mouse.click(5, 1390);
  await page.waitForTimeout(150);
  ok((await card.count()) === 0, `${theme}: a click elsewhere puts it away`);
  // Week: the same card from the timeline.
  await page.keyboard.press("Control+2");
  await page.waitForTimeout(500);
  const mark = page.locator(".cx-reminder", { hasText: "Sort out everything" }).first().locator(".cx-reminder-title");
  await mark.scrollIntoViewIfNeeded();
  await mark.click();
  await page.waitForTimeout(150);
  ok((await card.count()) === 1 && (await page.locator(".cx-block.is-draft").count()) === 0, `${theme}: Week opens it, with no draft event under it`);
  // Show in Reminders.
  await page.evaluate(() => { window.__windows = []; });
  await card.getByRole("button", { name: "Show in Reminders" }).click();
  await page.waitForTimeout(600);
  // Each app is its own window: Calendar's hands the reminder to Reminders' window
  // (apps.rs app_reveal), which opens on it; it never turns into Reminders itself.
  const asked = await page.evaluate(() => (window.__windows ?? []).find((w) => w.cmd === "app_reveal"));
  ok(asked?.key === "reminders" && asked?.reveal?.id && asked.reveal.screen === "reminders",
    `${theme}: Show in Reminders opens it in Reminders' own window  ${JSON.stringify(asked)}`);
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
