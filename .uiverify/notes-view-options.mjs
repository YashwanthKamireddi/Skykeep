// Notes' View Options: sort by Date Edited, Date Created or Title, and Group By Date — the
// headings follow, every note stays listed once, and the choice is still there after a reload.
//
//   BASE=http://localhost:4173 ENGINE=webkit node .uiverify/notes-view-options.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };

const ctx = await b.newContext({ viewport: { width: 1300, height: 850 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.waitForTimeout(400);

const headings = () => page.locator(".note-rows .notes-section").allInnerTexts();
const rows = () => page.locator(".note-rows .note-row").count();
const choose = async (label) => {
  await page.getByRole("button", { name: "View Options" }).click();
  await page.getByRole("menuitemcheckbox", { name: label }).click();
  await page.waitForTimeout(200);
};

const before = await headings();
const count = await rows();
ok(before[0] === "Pinned" && before.length >= 2, `by date edited, Pinned leads and the rest are grouped by date  ${JSON.stringify(before)}`);

await page.getByRole("button", { name: "View Options" }).click();
const items = await page.getByRole("menuitemcheckbox").allInnerTexts();
ok(["Date Edited", "Date Created", "Title", "Group By Date"].every((l) => items.some((i) => i.includes(l))), `the menu offers every order  ${JSON.stringify(items)}`);
ok((await page.getByRole("menuitemcheckbox", { name: "Date Edited" }).getAttribute("aria-checked")) === "true", "Date Edited is the one checked");
await page.keyboard.press("Escape");

await choose("Title");
ok(JSON.stringify(await headings()) === JSON.stringify(["Pinned", "Notes"]), `by title, one run under Pinned  ${JSON.stringify(await headings())}`);
ok((await rows()) === count, "every note is still listed");
await page.getByRole("button", { name: "View Options" }).click();
ok((await page.getByRole("menuitemcheckbox", { name: "Group By Date" }).count()) === 0, "Group By Date is not offered for Title, as in Notes");
await page.keyboard.press("Escape");

await page.reload({ waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.waitForTimeout(400);
ok(JSON.stringify(await headings()) === JSON.stringify(["Pinned", "Notes"]), "the order is kept after a reload");

await choose("Date Edited");
await choose("Group By Date");
ok(JSON.stringify(await headings()) === JSON.stringify(["Pinned", "Notes"]), `not grouped, the dated notes run as one  ${JSON.stringify(await headings())}`);
ok((await rows()) === count, "and every note is still listed");
await choose("Group By Date");
ok(JSON.stringify(await headings()) === JSON.stringify(before), "grouping back on restores the date headings");

await b.close();
process.exitCode = failures ? 1 : 0;
