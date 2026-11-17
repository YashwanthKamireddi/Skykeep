// Reminders' Completed list, as on the Mac: a tile with its count, the reminders ticked off
// grouped by list, nothing to add there, and one unticked leaves it. And the Mac's keys for the
// selected reminder: Ctrl+Shift+F flags it, Ctrl+Shift+C completes it, Ctrl+T makes it due
// today; Ctrl+Shift+H shows or hides a list's completed ones.
//   BASE=http://localhost:4173 ENGINE=webkit node .uiverify/reminders-completed.mjs
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const until = (page, fn, arg) => page.waitForFunction(fn, arg, { timeout: 8000, polling: "raf" }).then(() => true, () => false);

const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Reminders");

// The tile, with its count, after Apple's four.
const tile = page.locator(".rx-tile", { hasText: "Completed" });
ok(await tile.count() === 1, "a Completed tile is in the sidebar");
ok((await tile.locator(".rx-tile-count").innerText()).trim() === "3", "with how many are completed");
const order = await page.locator(".rx-tile .rx-tile-name").allInnerTexts();
ok(order.slice(0, 5).join(",") === "Today,Scheduled,All,Flagged,Completed", `in Apple's order (${order.slice(0, 5)})`);

await tile.click();
ok(await until(page, () => [...document.querySelectorAll(".rx-row .rx-title")].some((t) => t.value === "Water the plants")),
  "it shows the reminders ticked off");
ok(await page.locator(".rx-row .rx-title").evaluateAll((els) => !els.some((e) => e.value === "Plan tomorrow")), "and none still to do");
ok(await page.locator(".rx-section", { hasText: "Daily" }).count() === 1, "grouped under the list each is in");
ok(await page.locator(".rx-add").count() === 0, "with nothing to add there");
ok(/1 completed/.test(await page.locator(".rx-head").innerText()), "its heading counts what is completed");

// Unticked, it is open again and leaves Completed.
await page.locator(".rx-row", { has: page.locator('.rx-title[value="Water the plants"]') }).locator(".rx-tick").click();
ok(await until(page, () => (window.__done ?? []).some((d) => d.guid === "r4" && d.done === false)), "unticking it sends it as open");
ok(await until(page, () => ![...document.querySelectorAll(".rx-row .rx-title")].some((t) => t.value === "Water the plants")),
  "and it leaves Completed");

// The Mac's keys, on the selected reminder in a list.
await page.locator(".rx-list", { hasText: "Daily" }).click();
await until(page, () => [...document.querySelectorAll(".rx-row .rx-title")].some((t) => t.value === "Plan tomorrow"));
const row = page.locator(".rx-row", { has: page.locator('.rx-title[value="Plan tomorrow"]') });
await row.focus();
await page.keyboard.press("Control+Shift+f");
ok(await until(page, () => (window.__edits ?? []).some((e) => e.flagged === true)), "Ctrl+Shift+F flags the selected reminder");
ok(await until(page, () => document.querySelector('.rx-row[data-guid="r1"] .rx-flag')?.getAttribute("aria-pressed") === "true"),
  "and it shows flagged at once");

// While its title is being typed in too.
await row.locator(".rx-title").focus();
await page.keyboard.press("Control+t");
ok(await until(page, () => {
  const d = new Date();
  const from = new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime() / 1000;
  return (window.__edits ?? []).some((e) => e.allDay === false && e.dueAt >= from && e.dueAt < from + 86400);
}),
  "Ctrl+T makes it due today, keeping its time");
ok(await until(page, () => /Today/.test(document.querySelector('.rx-row[data-guid="r1"] .rx-due')?.textContent ?? "")),
  "and its row says Today");

await row.focus();
await page.keyboard.press("Control+Shift+c");
ok(await until(page, () => (window.__done ?? []).some((d) => d.guid === "r1" && d.done === true)), "Ctrl+Shift+C completes it");

// Show and hide the list's completed reminders.
const link = page.locator(".rx-head .link");
const before = await link.innerText();
await page.mouse.click(900, 760);
await page.keyboard.press("Control+Shift+h");
ok(await until(page, (b4) => document.querySelector(".rx-head .link")?.textContent !== b4, before), "Ctrl+Shift+H shows or hides completed");
await page.keyboard.press("Control+Shift+h");
ok(await until(page, (b4) => document.querySelector(".rx-head .link")?.textContent === b4, before), "and again puts it back");

await ctx.close();
await b.close();
console.log(failures ? `\n${failures} FAILED` : "\nall passed");
process.exit(failures ? 1 : 0);
