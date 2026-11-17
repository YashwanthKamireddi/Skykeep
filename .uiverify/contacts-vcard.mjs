// vCards and My Card, as icloud.com offers them: a card or a whole list out to Downloads
// under Apple's file names, a file brought in, and "me" beside this person's own card —
// moved at once when another is made My Card.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, contactsFixture: true });
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Contacts");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const meRow = () => page.locator(".contact-row", { has: page.locator(".contact-me") }).locator(".contact-row-name").allInnerTexts();

ok(JSON.stringify(await meRow()) === JSON.stringify(["Zoya Ahuja"]), "\"me\" marks this person's own card");
await page.locator(".contact-row", { hasText: "Amma" }).click({ button: "right" });
await page.getByRole("menuitem", { name: "Make This My Card" }).click();
await page.waitForTimeout(150);
ok(JSON.stringify(await meRow()) === JSON.stringify(["Amma"]), "Make This My Card moves it at once");

await page.locator(".contact-row", { hasText: "Amma" }).click({ button: "right" });
await page.getByRole("menuitem", { name: "Export vCard" }).click();
await page.waitForTimeout(200);
let sent = await page.evaluate(() => window.__exports?.at(-1));
ok(sent?.fileName === "Amma.vcf" && sent.guids.join() === "c2", `one card goes out under its name  ${JSON.stringify(sent)}`);
ok(/Saved to Downloads/.test(await page.locator(".toast").last().innerText()) && (await page.locator(".toast").last().getByRole("button", { name: "Show in Folder" }).count()) === 1, "and says where it went, with a way to see it");

await page.getByRole("button", { name: "More" }).click();
await page.getByRole("menuitem", { name: "Export All Contacts" }).click();
await page.waitForTimeout(200);
sent = await page.evaluate(() => window.__exports?.at(-1));
ok(sent?.fileName === "iCloud vCards.vcf" && sent.guids.length === 4, `everyone goes out as iCloud vCards  ${JSON.stringify(sent?.fileName)}`);

// Mass import: several files at once, a Google Contacts CSV among them.
await page.evaluate(() => { window.__pick = ["/home/test/people.vcf", "/home/test/google.csv"]; });
await page.getByRole("button", { name: "More" }).click();
await page.getByRole("menuitem", { name: "Import Contacts" }).click();
await page.waitForTimeout(300);
const paths = await page.evaluate(() => window.__imports?.at(-1)?.paths);
ok(JSON.stringify(paths) === JSON.stringify(["/home/test/people.vcf", "/home/test/google.csv"]), `every chosen file, vCard and CSV, is imported at once  ${JSON.stringify(paths)}`);
ok(/2 contacts imported/.test(await page.locator(".toast").last().innerText()), "and the count is said");
// Importing an export of the same book: everyone already there is left out, and said so.
await page.evaluate(() => { window.__importAnswer = [0, 0, 123]; });
await page.getByRole("button", { name: "More" }).click();
await page.getByRole("menuitem", { name: "Import Contacts" }).click();
await page.waitForTimeout(300);
const said = await page.locator(".toast").last().innerText();
ok(/0 contacts imported, 123 contacts already in Contacts were left out/.test(said), `people already in Contacts are left out, and that is said  "${said}"`);
await b.close();
process.exitCode = failures ? 1 : 0;
