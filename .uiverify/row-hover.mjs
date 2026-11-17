// A sidebar row's ⋯: moving from the row onto it keeps the row lit the whole way, and the
// name beside it does not move — in Reminders' lists, Notes' folders and Contacts' lists.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true, notesFixture: true, contactsFixture: true });
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
for (const [screen, rowSel, hitSel, nameSel, moreSel] of [
  ["Reminders", ".rx-list-row", ".rx-list", ".rx-list-name", ".rx-list-more"],
  ["Notes", ".notes-folder-row", ".notes-folder", ".notes-folder-name", ".notes-folder-more"],
  ["Contacts", ".contacts-lists .notes-folder-row", ".notes-folder", ".notes-folder-name", ".notes-folder-more"],
]) {
  await openScreen(page, screen);
  await page.waitForTimeout(400);
  const row = page.locator(rowSel).filter({ has: page.locator(moreSel) }).first();
  if (!(await row.count())) { ok(false, `${screen}: a row with a ⋯`); continue; }
  const lit = () => row.locator(hitSel).evaluate((el) => getComputedStyle(el).backgroundColor);
  const nameBox = () => row.locator(nameSel).evaluate((el) => { const r = el.getBoundingClientRect(); return `${Math.round(r.width)}`; });
  await row.locator(hitSel).hover({ position: { x: 30, y: 10 } });
  await page.waitForTimeout(200);
  const onRow = await lit();
  const nameOnRow = await nameBox();
  await row.locator(moreSel).hover();
  await page.waitForTimeout(200);
  ok((await lit()) === onRow && onRow !== "rgba(0, 0, 0, 0)", `${screen}: the row stays lit with the pointer on its ⋯  [${onRow} → ${await lit()}]`);
  ok((await nameBox()) === nameOnRow, `${screen}: the name does not move when the ⋯ is pointed at`);
}
await b.close();
process.exitCode = failures ? 1 : 0;
