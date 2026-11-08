// Birthdays and holidays, as a phone shows them: their own read-only calendars under Other,
// each day opening to say what it is, with nothing to edit or delete, and each calendar able
// to be switched off.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1400, height: 900 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
  calendarFixture: true, notesFixture: true, contactsFixture: true, driveFixture: true, remindersFixture: true });
const page = await ctx.newPage();
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Calendar");
await page.getByRole("button", { name: "Month", exact: true }).click();
await page.waitForTimeout(500);

let failures = 0;
const ok = (pass, yes, no) => { console.log(pass ? `PASS ${yes}` : `FAIL ${no}`); if (!pass) failures++; };

const side = await page.locator(".cx-side").innerText();
ok(/Other[\s\S]*Birthdays[\s\S]*India Holidays/.test(side), "Birthdays and the holidays are listed under Other",
  `the sidebar reads: ${side.replace(/\s+/g, " ").slice(0, 200)}`);
const shown = async (title) => (await page.locator(".cx-bar-event, .cx-item", { hasText: title }).count()) > 0;
ok((await shown("Zoya Ahuja’s Birthday")) && (await shown("Diwali")), "a birthday and a holiday are on their days",
  "the birthday or the holiday is missing from the month");

await page.locator(".cx-bar-event, .cx-item", { hasText: "Zoya Ahuja’s Birthday" }).first().click();
await page.waitForTimeout(300);
const pop = await page.evaluate(() => {
  const p = document.querySelector(".cx-popover");
  return p && {
    del: Boolean(p.querySelector(".cx-pop-delete")),
    buttons: [...p.querySelectorAll("button")].map((x) => x.textContent.trim()),
    fields: p.querySelectorAll("input, textarea, select").length,
    title: p.querySelector(".cx-info-title")?.textContent,
  };
});
// As Calendar shows one: what it is and when, not a form of fields that cannot be used.
ok(pop && !pop.del && pop.fields === 0 && pop.title === "Zoya Ahuja’s Birthday",
  "a birthday opens as an info card, with no fields and nothing to delete", `its popover: ${JSON.stringify(pop)}`);
await page.keyboard.press("Escape");

await page.locator(".cx-cal", { hasText: "Birthdays" }).click().catch(async () => {
  await page.locator(".cx-side label, .cx-side button", { hasText: "Birthdays" }).first().click();
});
await page.waitForTimeout(300);
ok(!(await shown("Zoya Ahuja’s Birthday")) && (await shown("Diwali")), "switching Birthdays off hides them, and only them",
  "switching Birthdays off did not hide them, or hid the holidays too");

// Search, as Calendar's toolbar field: Ctrl+F to it, type, and a result opens its day.
await page.keyboard.press("Control+f");
const focused = await page.evaluate(() => document.activeElement?.getAttribute("aria-label"));
ok(focused === "Search events", "Ctrl+F goes to the search field", `focus went to ${focused}`);
await page.keyboard.type("diwali");
await page.waitForTimeout(400);
const rows = await page.locator(".cx-found-row").allInnerTexts();
ok(rows.length === 1 && rows[0].includes("Diwali"), "searching finds the event", `found: ${JSON.stringify(rows)}`);
if (rows.length) {
  await page.locator(".cx-found-row").first().click();
  await page.waitForTimeout(400);
  const after = await page.evaluate(() => ({
    day: document.querySelector('.segment[aria-pressed="true"]')?.textContent,
    shows: document.body.innerText.includes("Diwali"),
    open: Boolean(document.querySelector(".cx-found")),
  }));
  ok(after.day === "Day" && after.shows && !after.open, "and opening it shows its day", `after: ${JSON.stringify(after)}`);
  const pane = await page.locator(".cx-pane").innerText().catch(() => "");
  ok(pane.includes("Diwali") && pane.includes("all-day"), "Day keeps the day at a glance beside its timeline",
    `the day pane reads: ${JSON.stringify(pane.slice(0, 120))}`);
}

// A day holding only all-day things — a holiday — still shows as busy in the mini month.
const diwali = new Date(); diwali.setDate(diwali.getDate() + 5);
const busyDays = await page.evaluate(() => [...document.querySelectorAll(".cx-side .cx-minimonth-day.is-busy")].map((d) => d.getAttribute("aria-label")));
const label = diwali.toLocaleDateString(undefined, { dateStyle: "full" });
ok(busyDays.includes(label), `a holiday's day carries a dot in the mini month  [${label}]`, `no dot on ${label}: ${JSON.stringify(busyDays)}`);
await b.close();
process.exitCode = failures === 0 ? 0 : 1;
