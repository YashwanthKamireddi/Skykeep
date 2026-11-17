// Reminders' keys, as on the Mac with Ctrl for Command: Ctrl+N a new reminder, Ctrl+Shift+N a
// new list, Ctrl+F the search, Ctrl+I the selected reminder's info; on a row Up/Down move,
// Space ticks, Return edits, Delete asks first and then deletes.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
await ctx.addInitScript(() => {
  const real = window.__TAURI_INTERNALS__;
  // Deletes are recorded, as Apple would be asked.
  const watch = () => {
    const inv = window.__TAURI_INTERNALS__?.invoke;
    if (!inv || inv.__wrapped) return;
    const wrapped = async (cmd, args) => {
      if (cmd === "reminder_delete") {
        (window.__deletes ??= []).push(args.guid);
        // Apple takes a moment, so what shows first is the app's own answer.
        await new Promise((r) => setTimeout(r, 1500));
      }
      return inv(cmd, args);
    };
    wrapped.__wrapped = true;
    window.__TAURI_INTERNALS__.invoke = wrapped;
  };
  document.addEventListener("DOMContentLoaded", watch);
  void real;
});
const page = await ctx.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.locator(".rx-list", { hasText: "Daily" }).click();
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const active = () => page.evaluate(() => {
  const a = document.activeElement;
  return { cls: a?.className ?? "", label: a?.getAttribute("aria-label") ?? "", row: a?.classList.contains("rx-row") ? a.querySelector(".rx-title")?.value : null };
});
const clickEmpty = () => page.mouse.click(900, 760);

// Ctrl+N: the new reminder field, from anywhere.
await clickEmpty();
await page.keyboard.press("Control+n");
await page.waitForTimeout(150);
ok((await page.evaluate(() => !!document.activeElement?.closest(".rx-add"))), "Ctrl+N goes to a new reminder");
await page.keyboard.type("Book the dentist");
await page.keyboard.press("Enter");
await page.waitForTimeout(300);
ok(await page.locator(".rx-row .rx-title").evaluateAll((els) => els.some((e) => e.value === "Book the dentist")), "and Return adds it");
// From a tag, which has nowhere to add, Ctrl+N opens a list to add to.
await page.locator(".rx-tag-chip", { hasText: "#home" }).click();
await page.waitForTimeout(300);
await clickEmpty();
await page.keyboard.press("Control+n");
await page.waitForTimeout(500);
ok(await page.evaluate(() => !!document.activeElement?.closest(".rx-add")), "Ctrl+N from a tag still lands in a new reminder");
await page.keyboard.press("Escape");

// Ctrl+Shift+N: a new list.
await clickEmpty();
await page.keyboard.press("Control+Shift+N");
await page.waitForTimeout(200);
ok((await active()).label === "New list name", `Ctrl+Shift+N names a new list  ${JSON.stringify(await active())}`);
await page.keyboard.press("Escape");
await page.locator(".rx-list", { hasText: "Daily" }).click();
await page.waitForTimeout(300);

// Ctrl+F: the search.
await clickEmpty();
await page.keyboard.press("Control+f");
ok((await active()).label === "Search reminders", "Ctrl+F goes to the search");
await page.keyboard.press("Escape");

// Down from nowhere picks the first reminder; Up and Down move.
await clickEmpty();
await page.keyboard.press("ArrowDown");
const first = (await active()).row;
ok(first === "Plan tomorrow", `Down starts at the first reminder  ${first}`);
await page.keyboard.press("ArrowDown");
ok((await active()).row === "Pick up the parcel", "Down moves to the next");
await page.keyboard.press("ArrowUp");
ok((await active()).row === "Plan tomorrow", "Up comes back");

// Ctrl+I: the selected reminder's info, and again to close it.
await page.keyboard.press("ArrowDown");
await page.keyboard.press("Control+i");
await page.waitForTimeout(250);
ok(await page.locator(".rx-row.is-open", { has: page.getByLabel("Rename Pick up the parcel") }).count() === 1, "Ctrl+I opens its info");
await page.keyboard.press("Control+i");
await page.waitForTimeout(250);
ok(await page.locator(".rx-detail").count() === 0, "Ctrl+I again closes it");
ok((await active()).row === "Pick up the parcel", "and the row keeps the keys");

// Space ticks; Return edits.
// Apple is slow here, so what shows is the tick itself, before any answer.
await page.evaluate(() => { window.__doneDelay = 800; });
await page.keyboard.press(" ");
await page.waitForTimeout(60);
ok(await page.locator(".rx-row", { has: page.getByLabel("Rename Pick up the parcel") }).locator('.rx-tick[aria-checked="true"]').count() === 1, "Space ticks the selected reminder, at once");
await page.waitForTimeout(900);
await page.evaluate(() => { window.__doneDelay = 0; });
const done = await page.evaluate(() => window.__done?.at(-1));
ok(done?.guid === "r2" && done.done === true, `and asks Apple for that one  ${JSON.stringify(done)}`);
// The ticked row goes, and the keys carry on from the one that took its place.
const heir = (await active()).row;
ok(heir && heir !== "Pick up the parcel", `a ticked row hands the keys to the next  ${heir}`);
await page.keyboard.press("Enter");
ok((await active()).cls.includes("rx-title"), "Return edits its title");
await page.keyboard.press("Escape");
ok((await active()).row === heir, "Escape goes back to the row");

// Delete asks first; Cancel keeps it; Delete deletes and moves on.
await page.keyboard.press("Delete");
await page.waitForTimeout(200);
const sheet = page.locator('[role="alertdialog"]');
ok(await sheet.count() === 1, "Delete asks first");
ok((await sheet.innerText()).includes(heir), "naming the reminder");
await page.keyboard.press("Escape");
await page.waitForTimeout(200);
ok(await page.locator(".rx-row .rx-title").evaluateAll((els, h) => els.some((e) => e.value === h), heir), "Cancel keeps it");
ok(!(await page.evaluate(() => window.__deletes?.length)), "and nothing is sent");
const heirGuid = await page.locator(".rx-row", { has: page.getByLabel(`Rename ${heir}`) }).getAttribute("data-guid");
await page.locator(".rx-row", { has: page.getByLabel(`Rename ${heir}`) }).focus();
await page.keyboard.press("Delete");
await page.waitForTimeout(200);
await page.locator('[role="alertdialog"] .confirm-answer').click();
await page.waitForTimeout(300);
ok(!(await page.locator(".rx-row .rx-title").evaluateAll((els, h) => els.some((e) => e.value === h), heir)), "Delete, confirmed, removes it at once");
ok(JSON.stringify(await page.evaluate(() => window.__deletes)) === JSON.stringify([heirGuid]), `and asks Apple  ${JSON.stringify(await page.evaluate(() => window.__deletes))}`);
ok((await active()).row !== null, `the focus moves on to a neighbouring row  ${JSON.stringify(await active())}`);
// Backspace in a title still deletes text, not the reminder.
await page.getByLabel("Rename Plan tomorrow").click();
await page.keyboard.press("End");
await page.keyboard.press("Backspace");
ok(await page.locator('[role="alertdialog"]').count() === 0, "Backspace in a title edits the text");
ok(errors.length === 0, `no page errors  ${errors.join(" | ")}`);
await b.close();
process.exitCode = failures ? 1 : 0;
