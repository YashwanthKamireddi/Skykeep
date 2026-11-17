// Reminders' keys, as on a Mac: Tab tucks a reminder under the one above, at once;
// Shift-Tab brings it back out; Return goes on to a new reminder below it.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.locator(".rx-list", { hasText: "Daily" }).click();
await page.waitForTimeout(300);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const row = () => page.locator(".rx-row", { has: page.getByLabel("Rename Pick up the parcel") });

await page.getByLabel("Rename Pick up the parcel").focus();
await page.keyboard.press("Tab");
await page.waitForTimeout(150);
ok(await row().evaluate((el) => el.classList.contains("is-subtask")), "Tab tucks a reminder under the one above, at once");
let sent = await page.evaluate(() => window.__edits?.at(-1));
ok(typeof sent?.parentGuid === "string", `and asks Apple to  ${JSON.stringify(sent)}`);

await page.getByLabel("Rename Pick up the parcel").focus();
await page.keyboard.press("Shift+Tab");
await page.waitForTimeout(150);
ok(!(await row().evaluate((el) => el.classList.contains("is-subtask"))), "Shift-Tab brings it back out");
sent = await page.evaluate(() => window.__edits?.at(-1));
ok(sent && sent.parentGuid === null, `and asks Apple to  ${JSON.stringify(sent)}`);

// Command-] and Command-[ do the same, as the Mac's Format menu has them.
await page.getByLabel("Rename Pick up the parcel").focus();
await page.keyboard.press("Control+]");
await page.waitForTimeout(150);
ok(await row().evaluate((el) => el.classList.contains("is-subtask")), "Ctrl-] indents it");
await page.getByLabel("Rename Pick up the parcel").focus();
await page.keyboard.press("Control+[");
await page.waitForTimeout(150);
ok(!(await row().evaluate((el) => el.classList.contains("is-subtask"))), "Ctrl-[ outdents it");
// Up and Down walk from reminder to reminder.
const order = await page.locator(".rx-row .rx-title").evaluateAll((els) => els.map((e) => e.getAttribute("aria-label")));
const at = order.indexOf("Rename Pick up the parcel");
await page.getByLabel("Rename Pick up the parcel").focus();
await page.keyboard.press("ArrowDown");
ok(await page.evaluate((want) => document.activeElement?.getAttribute("aria-label") === want, order[at + 1]), "Down goes to the next reminder");
await page.keyboard.press("ArrowUp");
ok(await page.evaluate(() => document.activeElement?.getAttribute("aria-label") === "Rename Pick up the parcel"), "Up comes back");

await page.getByLabel("Rename Pick up the parcel").focus();
await page.keyboard.press("Enter");
await page.waitForTimeout(150);
// Right below it, as on the Mac (reminders-next-below.mjs has the rest); Escape ends it there.
ok(await page.evaluate(() => {
  const next = document.activeElement?.closest(".rx-next");
  return next?.previousElementSibling?.querySelector(".rx-title")?.value === "Pick up the parcel";
}), "Return goes on to a new reminder, right below it");
await page.keyboard.press("Escape");
// Drag to reorder: the row lands where it is dropped, at once, and Apple is asked.
const titles = () => page.locator(".rx-row:not(.is-done) .rx-title").evaluateAll((els) => els.map((e) => e.value));
const before = await titles();
const last = before.filter(Boolean).at(-1);
const src = page.locator(".rx-row", { has: page.getByLabel(`Rename ${last}`) });
const dst = page.locator(".rx-row").first();
await src.dragTo(dst);
await page.waitForTimeout(200);
const after = await titles();
ok(after[0] === last, `a dragged reminder lands where it is dropped  ${JSON.stringify(after.slice(0, 3))}`);
const asked = await page.evaluate(() => window.__reorders?.at(-1));
ok(asked && asked.before, `and Apple is asked to put it there  ${JSON.stringify(asked)}`);
// Typing in a title still types, in a row that can be dragged.
await page.getByLabel(`Rename ${last}`).click();
await page.keyboard.press("End");
await page.keyboard.type("!");
ok((await page.getByLabel(`Rename ${last}`).inputValue()).endsWith("!"), "a reminder's title still takes typing");

// The row takes the keyboard: Escape leaves a title for its row, arrows go row to row,
// Space ticks the row, Return goes back into its title.
await page.getByLabel("Rename Pick up the parcel").focus();
await page.keyboard.press("Escape");
const focusedRow = () => page.evaluate(() => document.activeElement?.classList.contains("rx-row") ? document.activeElement.querySelector(".rx-title")?.value : null);
ok((await focusedRow()) === "Pick up the parcel", "Escape leaves the title for its row");
const startRow = await focusedRow();
await page.keyboard.press("ArrowDown");
const below = await focusedRow();
ok(below && below !== startRow, `Down goes to the next row  ${startRow} → ${below}`);
await page.keyboard.press("ArrowUp");
ok((await focusedRow()) === startRow, "and Up comes back");
await page.keyboard.press(" ");
await page.waitForTimeout(200);
const done = await page.evaluate(() => window.__done?.at(-1));
ok(done && done.done === true && done.guid === "r2", `Space ticks that row, and only it  ${JSON.stringify(done)}`);
await page.keyboard.press("Enter");
ok(await page.evaluate(() => document.activeElement?.classList.contains("rx-title")), "Return goes into its title");
await b.close();
process.exitCode = failures ? 1 : 0;
