// Calendars made, renamed, recoloured and deleted from the sidebar, as on icloud.com: + makes
// one; a name in use is refused with Apple's words; right-click renames, recolours, deletes;
// the default calendar offers no Delete.
import { engine } from "./engine.mjs";
import { SERVICES, autoConfirm, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
const page = await ctx.newPage();
await ctx.addInitScript(autoConfirm);
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Calendar");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const names = () => page.locator(".cx-cal-name").allInnerTexts();

await page.getByRole("button", { name: "Add Calendar" }).click();
await page.getByRole("menuitem", { name: "New Calendar" }).click();
await page.getByLabel("New calendar name").fill("Trips");
await page.keyboard.press("Enter");
await page.waitForTimeout(400);
ok((await names()).includes("Trips"), "+ makes a calendar, named as typed");

await page.getByRole("button", { name: "Add Calendar" }).click();
await page.getByRole("menuitem", { name: "New Calendar" }).click();
await page.getByLabel("New calendar name").fill("Work");
await page.keyboard.press("Enter");
await page.waitForTimeout(400);
const notice = await page.locator(".notice, [role=alert]").allInnerTexts();
ok(notice.some((t) => /already exists/.test(t)) && (await names()).filter((n) => n === "Work").length === 1, `a name already in use is refused, in Apple's words  ${JSON.stringify(notice)}`);

const work = page.locator(".cx-cal", { hasText: "Work" });
await work.click({ button: "right" });
await page.getByRole("menuitem", { name: "Rename" }).click();
await page.getByLabel("Rename Work").fill("Office");
await page.keyboard.press("Enter");
await page.waitForTimeout(400);
ok((await names()).includes("Office") && !(await names()).includes("Work"), "Rename renames it in place");

await page.locator(".cx-cal", { hasText: "Office" }).click({ button: "right" });
await page.getByRole("menuitem", { name: "Colour" }).hover();
await page.getByRole("menuitemcheckbox", { name: "Brown" }).click();
await page.waitForTimeout(300);
const writes = await page.evaluate(() => window.__calWrites);
ok(writes.some((w) => w.update?.colour === "#a2845e"), `Colour sets Apple's brown  ${JSON.stringify(writes.at(-1))}`);

await page.locator(".cx-cal", { hasText: "Home" }).click({ button: "right" });
const homeMenu = await page.getByRole("menuitem").allInnerTexts();
ok(!homeMenu.some((t) => /Delete/.test(t)), `the default calendar offers no Delete  ${JSON.stringify(homeMenu)}`);
await page.keyboard.press("Escape");

await page.locator(".cx-cal", { hasText: "Trips" }).click({ button: "right" });
await page.getByRole("menuitem", { name: /Delete Calendar/ }).click();
await page.waitForTimeout(400);
ok(!(await names()).includes("Trips"), "Delete Calendar, once confirmed, removes it");
await b.close();
process.exitCode = failures ? 1 : 0;
