// Several contacts at once, as Contacts on the Mac: Ctrl-click adds one, Shift-click a run,
// Ctrl-A everyone; the card shows how many, and what is done is done to them all.
import { engine } from "./engine.mjs";
import { SERVICES, autoConfirm, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, contactsFixture: true });
const page = await ctx.newPage();
await ctx.addInitScript(autoConfirm);
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Contacts");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const chosen = () => page.locator('.contact-row[aria-selected="true"] .contact-row-name').allInnerTexts();
const heading = () => page.locator(".contact-many h2").innerText().catch(() => "");

await page.locator(".contact-row", { hasText: "Zoya Ahuja" }).click();
await page.locator(".contact-row", { hasText: "Northfield" }).click({ modifiers: ["Control"] });
ok(JSON.stringify(await chosen()) === JSON.stringify(["Zoya Ahuja", "Northfield University"]), `Ctrl-click adds one  ${JSON.stringify(await chosen())}`);
ok((await heading()) === "2 Contacts Selected", "and the card says how many");

await page.locator(".contact-row", { hasText: "Zoya Ahuja" }).click();
await page.locator(".contact-row", { hasText: "Northfield" }).click({ modifiers: ["Shift"] });
ok((await chosen()).length === 3, `Shift-click takes the run between  ${JSON.stringify(await chosen())}`);

await page.keyboard.press("Control+a");
ok((await chosen()).length === 4, "Ctrl-A takes everyone");

await page.getByRole("button", { name: "Export vCards" }).click();
await page.waitForTimeout(200);
const sent = await page.evaluate(() => window.__exports?.at(-1));
ok(sent?.guids.length === 4 && sent.fileName === "iCloud vCards.vcf", "all of them go out together");

await page.locator(".contact-many").getByRole("button", { name: "Add to List" }).click();
await page.getByRole("menuitem", { name: "Family" }).click();
await page.waitForTimeout(150);
const famCount = await page.locator(".contacts-lists .notes-folder-row", { hasText: "Family" }).locator(".notes-folder-count").textContent();
ok(famCount.trim() === "4", `all of them join a list at once  ${famCount}`);
await page.waitForTimeout(900);
const write = await page.evaluate(() => window.__listWrites?.at(-1));
ok(write?.add?.length === 3, `only those not in it yet are sent  ${JSON.stringify(write)}`);

await page.locator(".contact-row", { hasText: "Amma" }).click();
ok((await chosen()).length === 1 && !(await page.locator(".contact-many").count()), "a plain click goes back to one");
await b.close();
process.exitCode = failures ? 1 : 0;
