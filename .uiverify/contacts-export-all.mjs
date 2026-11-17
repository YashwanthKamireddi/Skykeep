// Export All Contacts exports everyone, whatever is typed in the search: with a search open it
// exported only the people that search had found, under the name of the whole book.
//
//   URL=http://localhost:4203 ENGINE=webkit node .uiverify/contacts-export-all.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(() => { window.__contactsSearch = true; });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, contactsFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Contacts");
await page.waitForTimeout(400);
const everyone = await page.locator(".contact-rows > li .contact-row, .contact-rows > li button").count();
await page.getByRole("searchbox", { name: "Search contacts" }).or(page.locator(".contacts-search")).first().fill("zoya");
await page.waitForTimeout(400);
const rows = (await page.locator(".contact-rows > li").allInnerTexts()).filter((t) => t.trim().length > 1);
ok(rows.length === 1, `the search shows its one match  ${JSON.stringify(rows)}`);
await page.getByRole("button", { name: /^More/ }).first().click();
await page.getByRole("menuitem", { name: "Export All Contacts" }).click();
await page.waitForTimeout(300);
const sent = await page.evaluate(() => window.__exports ?? []);
ok(sent.length === 1 && sent[0].guids.length === 4, `every contact is exported, not only the match  ${JSON.stringify(sent.map((e) => e.guids))} (${everyone} rows)`);
await b.close();
console.log(failures ? `\n${failures} FAILED` : "\nall passed");
process.exit(failures ? 1 : 0);
