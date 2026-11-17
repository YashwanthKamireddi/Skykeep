// Week numbers on request (More ▸ Show Week Numbers, remembered), sidebar groups that fold
// and show or hide together, and search: coming up first, arrows to choose, Return to go to
// the day with the event open.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1300, height: 850 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
const page = await ctx.newPage();
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Calendar");
await page.keyboard.press("Control+3");
await page.waitForTimeout(300);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

ok(await page.locator(".cx-weeknum").count() === 0, "no week numbers by default");
await page.getByRole("button", { name: "More", exact: true }).click();
await page.getByRole("menuitemcheckbox", { name: "Show Week Numbers" }).click();
await page.waitForTimeout(200);
const nums = await page.locator(".cx-weeknum").allInnerTexts();
const expected = await page.evaluate(() => {
  // The week holding today, counted as the view counts it.
  const cell = [...document.querySelectorAll(".cx-day")].find((c) => c.querySelector(".cx-daynum.is-today"));
  return cell?.parentElement?.querySelector(".cx-weeknum")?.textContent;
});
ok(nums.length === 6 && nums.every((n) => /^\d{1,2}$/.test(n)) && Number(nums[1]) === (Number(nums[0]) % 53) + 1 || Number(nums[1]) === 1,
  `each week of the month is numbered  [${nums.join(" ")}]`);
ok(Boolean(expected) && await page.locator(".cx-minimonth-wk").count() >= 6, "the mini month numbers its weeks too");
await page.keyboard.press("Control+2");
await page.waitForTimeout(200);
ok(/^W\d+$/.test(await page.locator(".cx-head-week").innerText()), "Week shows its number");
await page.reload({ waitUntil: "networkidle" });
await openScreen(page, "Calendar");
await page.waitForTimeout(300);
ok(await page.locator(".cx-head-week").innerText() !== "", "remembered after a restart");

// Groups fold, and show or hide all their calendars.
const side = page.getByRole("complementary", { name: "Calendars" });
const other = side.getByRole("button", { name: "Other" });
await other.click();
ok(await other.getAttribute("aria-expanded") === "false" && await side.getByText("Birthdays").count() === 0, "a group folds away");
await other.click();
ok(await side.getByText("Birthdays").isVisible(), "and opens again");
await side.getByRole("button", { name: "iCloud" }).click({ button: "right" });
await page.getByRole("menuitem", { name: "Hide All" }).click();
await page.waitForTimeout(200);
ok(await page.locator(".cx-block").count() === 0, "Hide All hides the group's calendars");
const ticks = await side.locator("input[type=checkbox]").evaluateAll((els) => els.slice(0, 3).map((e) => e.checked));
ok(ticks.every((t) => !t), "their ticks say so");
await side.getByRole("button", { name: "iCloud" }).click({ button: "right" });
await page.getByRole("menuitem", { name: "Show All" }).click();
await page.waitForTimeout(200);
ok(await page.locator(".cx-block").count() > 0, "Show All brings them back");

// Search: the arrows choose, Return goes there and opens it.
await page.keyboard.press("Control+f");
await page.keyboard.type("planning");
await page.waitForTimeout(400);
const rows = page.getByRole("option");
ok(await rows.count() === 1 && /Sprint planning/.test(await rows.first().innerText()), "the event is found");
ok(await page.locator(".cx-mark").first().innerText() === "planning", "the words found are marked");
await page.keyboard.press("Control+a");
await page.keyboard.type("e");
await page.waitForTimeout(400);
const n = await rows.count();
await page.keyboard.press("ArrowDown");
ok(n > 1 && await rows.nth(1).getAttribute("aria-selected") === "true", "Down lights the next");
const chosen = (await rows.nth(1).locator(".cx-found-title").innerText()).trim();
await page.keyboard.press("Enter");
await page.waitForTimeout(500);
ok(await page.getByRole("button", { name: "Day", exact: true }).getAttribute("aria-pressed") === "true", "Return goes to its day");
ok(await page.getByRole("dialog", { name: chosen }).isVisible(), `and opens it  [${chosen}]`);
await b.close();
process.exitCode = failures ? 1 : 0;
