// A reminder dragged onto another list in the sidebar moves to it, as on the Mac: it leaves the
// list on screen at once and the move goes to Apple. Dropped on its own list, nothing happens.
//   BASE=http://localhost:4173 ENGINE=webkit node .uiverify/reminders-drag-list.mjs
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const until = (page, fn, arg) => page.waitForFunction(fn, arg, { timeout: 8000, polling: "raf" }).then(() => true, () => false);

const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(() => { window.__filterLists = true; window.__keepMoves = true; });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.locator(".rx-list", { hasText: "Daily" }).click();
await until(page, () => document.querySelector('.rx-row[data-guid="r1"]'));

// Onto its own list: nothing.
await page.locator('.rx-row[data-guid="r1"]').dragTo(page.locator(".rx-list", { hasText: "Daily" }));
await page.waitForTimeout(300);
ok(!(await page.evaluate(() => (window.__edits ?? []).some((e) => "listGuid" in e))), "dropped on its own list, nothing moves");

// Onto GYM: it moves there.
await page.locator('.rx-row[data-guid="r1"]').dragTo(page.locator(".rx-list", { hasText: "GYM" }));
ok(await until(page, () => (window.__edits ?? []).some((e) => e.listGuid === "l2")), "dropped on another list, it is moved there");
ok(await until(page, () => !document.querySelector('.rx-row[data-guid="r1"]')), "and leaves the list on screen");
ok(await page.locator(".rx-list.is-drop-target").count() === 0, "with no list left marked");

await page.locator(".rx-list", { hasText: "GYM" }).click();
ok(await until(page, () => [...document.querySelectorAll(".rx-row .rx-title")].some((t) => t.value === "Plan tomorrow")), "and is in the list it was dropped on");

await ctx.close();
await b.close();
console.log(failures ? `\n${failures} FAILED` : "\nall passed");
process.exit(failures ? 1 : 0);
