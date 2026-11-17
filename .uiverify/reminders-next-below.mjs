// Return on a reminder in a list begins the next one right below it, as on the Mac: the new
// row is there at once in that place, goes to Apple in that place (put before the reminder
// that followed), and the field goes on below it for another. Return on an empty one ends it;
// a subtask's next is a subtask of the same reminder; leaving the field with words adds them.
//   BASE=http://localhost:4173 ENGINE=webkit node .uiverify/reminders-next-below.mjs
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const until = (page, fn, arg) => page.waitForFunction(fn, arg, { timeout: 8000, polling: "raf" }).then(() => true, () => false);

const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(() => {
  const r = (guid, title, parentGuid = null) => ({ guid, listGuid: "l3", title, notes: null, dueAt: null, allDay: false,
    completedAt: null, priority: 0, flagged: false, parentGuid, etag: null });
  window.__extraReminders = [r("a", "Learn to swim"), r("a1", "Find a pool", "a"), r("b", "Run a marathon"), r("c", "See the northern lights")];
  window.__filterLists = true;
});
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.locator(".rx-list", { hasText: "Life Goals" }).click();
await until(page, () => document.querySelectorAll(".rx-row .rx-title").length >= 4);

const titles = () => page.locator(".rx-rows > li").evaluateAll((els) =>
  els.map((e) => (e.classList.contains("rx-next") ? "[next]" : e.querySelector(".rx-title")?.value)));

// Return on "Learn to swim": the field is below it and its subtask, not at the foot.
await page.getByLabel("Rename Learn to swim").focus();
await page.keyboard.press("Enter");
ok(await until(page, () => document.activeElement?.closest(".rx-next") !== null), "Return opens a new reminder, its field focused");
let shown = await titles();
ok(shown.join("|") === "Learn to swim|Find a pool|[next]|Run a marathon|See the northern lights", `right below it and its subtask (${shown.join(" | ")})`);

// Typed and Return: there at once, in that place, the field going on below it.
await page.keyboard.type("Learn to dive");
await page.keyboard.press("Enter");
ok(await until(page, () => [...document.querySelectorAll(".rx-rows > li")].map((e) => e.classList.contains("rx-next") ? "[next]" : e.querySelector(".rx-title")?.value).join("|")
  === "Learn to swim|Find a pool|Learn to dive|[next]|Run a marathon|See the northern lights"), "the new one is there at once, the field below it");
ok(await page.evaluate(() => document.activeElement?.closest(".rx-next") !== null && document.activeElement.value === ""), "and the field is empty and still focused");
ok(await until(page, () => (window.__creates ?? []).some((c) => c.title === "Learn to dive" && c.listGuid === "l3")), "it is made in the list");
ok(await until(page, () => (window.__reorders ?? []).some((r) => r.before === "b")), "and put before the reminder that followed");

// Another, typed straight on: it goes below the last.
await page.keyboard.type("Swim a mile");
await page.keyboard.press("Enter");
ok(await until(page, () => {
  const t = [...document.querySelectorAll(".rx-rows > li:not(.rx-next) .rx-title")].map((e) => e.value);
  return t.indexOf("Swim a mile") === t.indexOf("Learn to dive") + 1 && t.indexOf("Run a marathon") === t.indexOf("Swim a mile") + 1;
}), "a second goes below the first");

// Return on the empty field ends it there.
await page.keyboard.press("Enter");
ok(await until(page, () => !document.querySelector(".rx-next")), "Return on an empty one ends it");
ok(await page.evaluate(() => (window.__creates ?? []).length) === 2, "and makes nothing");

// A subtask's next is a subtask of the same reminder.
await until(page, () => document.querySelector('.rx-row[data-guid="a1"] .rx-title'));
await page.locator('.rx-row[data-guid="a1"] .rx-title').focus();
await page.keyboard.press("Enter");
ok(await until(page, () => document.activeElement?.closest(".rx-next.is-subtask") !== null), "below a subtask, the next is a subtask too");
await page.keyboard.type("Buy goggles");
await page.keyboard.press("Enter");
ok(await until(page, () => (window.__edits ?? []).some((e) => e.parentGuid === "a")), "made under the same reminder");
ok(await until(page, () => [...document.querySelectorAll(".rx-row.is-subtask .rx-title")].some((e) => e.value === "Buy goggles")), "and shown as its subtask");
await page.keyboard.press("Escape");
ok(await until(page, () => !document.querySelector(".rx-next")), "Escape ends it");

// Leaving the field with words in it adds them, and the field goes.
await page.getByLabel("Rename See the northern lights").focus();
await page.keyboard.press("Enter");
await until(page, () => document.activeElement?.closest(".rx-next") !== null);
await page.keyboard.type("Visit Iceland");
await page.mouse.click(900, 760);
ok(await until(page, () => (window.__creates ?? []).some((c) => c.title === "Visit Iceland")), "leaving the field adds what was typed");
ok(await until(page, () => !document.querySelector(".rx-next")), "and the field goes");

// In a smart list, Return goes to New Reminder at the foot, as before.
await page.locator(".rx-tile", { hasText: "Today" }).click();
await until(page, () => document.querySelector(".rx-row .rx-title"));
await page.locator(".rx-row .rx-title").first().focus();
await page.keyboard.press("Enter");
ok(await until(page, () => document.activeElement?.closest(".rx-add") !== null), "in Today, Return goes to New Reminder");

await ctx.close();
await b.close();
console.log(failures ? `\n${failures} FAILED` : "\nall passed");
process.exit(failures ? 1 : 0);
