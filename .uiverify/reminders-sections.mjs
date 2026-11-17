// A list's sections, as Apple shows them: the reminders outside any section first with no
// heading, then each section under its own heading in the order it was made — and a reminder
// placed in a section since deleted stays in view with the loose ones.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
await ctx.addInitScript(() => {
  window.__filterLists = true;
  window.__sections = [
    { guid: "s1", listGuid: "l1", title: "Errands", createdAt: 1 },
    { guid: "s2", listGuid: "l1", title: "At home", createdAt: 2 },
    { guid: "s3", listGuid: "l2", title: "Elsewhere", createdAt: 3 },
  ];
  const r = (guid, title, sectionGuid) => ({ guid, listGuid: "l1", title, notes: null, dueAt: null, completedAt: null,
    priority: 0, flagged: false, parentGuid: null, sectionGuid, recurFrequency: null, recurInterval: null, etag: null });
  window.__extraReminders = [
    r("x1", "Return the library books", "s1"),
    r("x2", "Buy oat milk", "s1"),
    r("x3", "Fix the balcony light", "s2"),
    r("x4", "Call the plumber", "gone-section"),
  ];
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

// What the pane reads top to bottom: headings and titles, in order.
const order = await page.evaluate(() =>
  [...document.querySelectorAll(".rx-main .rx-section, .rx-main .rx-row .rx-title")].map((el) =>
    el.classList.contains("rx-section") ? `## ${el.textContent}` : el.value));
const at = (x) => order.indexOf(x);
ok(at("## Errands") > -1 && at("## At home") > at("## Errands"), `sections in the order they were made  ${JSON.stringify(order)}`);
ok(!order.includes("## Elsewhere"), "another list's sections are not shown");
ok(at("Plan tomorrow") < at("## Errands") && at("Call the plumber") < at("## Errands"), "loose reminders first, with no heading — a lost section's too");
ok(at("Return the library books") > at("## Errands") && at("Buy oat milk") < at("## At home"), "each reminder under its own section");
ok(at("Fix the balcony light") > at("## At home"), "the last section holds its own");
ok(await page.locator(".rx-section.is-list-section").count() === 2, "two section headings");

// The info panel names the section; section writes are on (proven on the iPhone).
await page.getByRole("button", { name: "Details for Buy oat milk" }).click();
await page.waitForTimeout(300);
const field = await page.locator(".rx-detail .rx-field", { hasText: "Section" }).innerText();
ok(field.includes("Errands"), `the info names its section  ${JSON.stringify(field)}`);
ok(await page.getByRole("button", { name: "Add Section" }).count() > 0, "Add Section is offered now section writes are on");
// The row menu offers the move to a section.
await page.locator(".rx-row", { has: page.getByLabel("Rename Buy oat milk") }).click({ button: "right", position: { x: 300, y: 10 } });
await page.waitForTimeout(200);
const menu = await page.locator('[role="menu"] [role="menuitem"]').allInnerTexts();
ok(menu.some((m) => m.includes("Move to Section")), `Move to Section is offered  ${JSON.stringify(menu)}`);
await page.keyboard.press("Escape");

// Smart views gather from every list and do not use a list's sections.
await page.locator(".rx-tile", { hasText: "All" }).click();
await page.waitForTimeout(300);
ok(await page.locator(".rx-section.is-list-section").count() === 0, "a smart view is grouped by list, not section");
ok(errors.length === 0, `no page errors  ${errors.join(" | ")}`);
await b.close();
process.exitCode = failures ? 1 : 0;
