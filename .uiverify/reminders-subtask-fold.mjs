// A reminder's subtasks fold away under it, as on the Mac: its arrow hides them and says how
// many wait there, and shows them again; Ctrl+Shift+E hides every reminder's, Ctrl+E shows
// them all. The fold is kept on this computer across a reload.
//   BASE=http://localhost:4173 ENGINE=webkit node .uiverify/reminders-subtask-fold.mjs
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const until = (page, fn, arg) => page.waitForFunction(fn, arg, { timeout: 8000, polling: "raf" }).then(() => true, () => false);

const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(() => {
  const r = (guid, title, parentGuid = null) => ({ guid, listGuid: "l1", title, notes: null, dueAt: null, allDay: false,
    completedAt: null, priority: 0, flagged: false, parentGuid, etag: null });
  window.__extraReminders = [r("par", "Plan the move"), r("k1", "Book the van", "par"), r("k2", "Pack the kitchen", "par")];
  window.__filterLists = true;
});
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.locator(".rx-list", { hasText: "Daily" }).click();

const shown = (title) => page.evaluate((t) => [...document.querySelectorAll(".rx-row .rx-title")].some((e) => e.value === t), title);
ok(await until(page, () => [...document.querySelectorAll(".rx-row .rx-title")].some((e) => e.value === "Book the van")), "a reminder's subtasks show under it");

const arrow = page.locator('.rx-row[data-guid="par"] .rx-kids');
ok(await arrow.count() === 1, "a reminder with subtasks has an arrow for them");
ok(await page.locator('.rx-row[data-guid="k1"] .rx-kids').count() === 0, "a subtask has none");
ok(await page.locator('.rx-row[data-guid="r1"] .rx-kids').count() === 0, "nor does a reminder without subtasks");
ok(await arrow.getAttribute("aria-expanded") === "true", "shown, the arrow says so");

await arrow.click();
ok(await until(page, () => ![...document.querySelectorAll(".rx-row .rx-title")].some((e) => e.value === "Book the van" || e.value === "Pack the kitchen")),
  "its arrow folds them away");
ok(await shown("Plan the move"), "the reminder itself stays");
ok((await arrow.innerText()).trim() === "2", "and says how many wait under it");
ok(await arrow.getAttribute("aria-expanded") === "false", "folded, the arrow says so");

// Kept across a reload, as the Mac keeps it.
await page.reload({ waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.locator(".rx-list", { hasText: "Daily" }).click();
await until(page, () => [...document.querySelectorAll(".rx-row .rx-title")].some((e) => e.value === "Plan the move"));
ok(!(await shown("Book the van")), "the fold is kept after a reload");

// Ctrl+E shows every subtask; Ctrl+Shift+E hides them all.
await page.mouse.click(900, 760);
await page.keyboard.press("Control+e");
ok(await until(page, () => [...document.querySelectorAll(".rx-row .rx-title")].some((e) => e.value === "Book the van")), "Ctrl+E shows every subtask");
await page.keyboard.press("Control+Shift+e");
ok(await until(page, () => ![...document.querySelectorAll(".rx-row .rx-title")].some((e) => e.value === "Pack the kitchen")), "Ctrl+Shift+E hides them all");
await page.locator('.rx-row[data-guid="par"] .rx-kids').click();
ok(await until(page, () => [...document.querySelectorAll(".rx-row .rx-title")].some((e) => e.value === "Pack the kitchen")), "and its arrow shows them again");

await ctx.close();
await b.close();
console.log(failures ? `\n${failures} FAILED` : "\nall passed");
process.exit(failures ? 1 : 0);
