// A company's card goes by the company, as Contacts shows it: in the list, at the top of the
// card, and in the search across everything — with the person named on it underneath. It used
// to read as the person, filed under the company's letter, with the company nowhere.
//
//   URL=http://localhost:4203 ENGINE=webkit node .uiverify/contacts-company.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const URL = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(() => {
  window.__contactsExtra = [{ guid: "c9", firstName: "Ravi", lastName: "Menon", company: "Bluepine Logistics", isCompany: true,
    birthday: null, notes: null, etag: null, methods: [{ kind: "phone", label: "work", value: "+1 555 010 0142" }] }];
});
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, contactsFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(URL, { waitUntil: "networkidle" });
await openScreen(page, "Contacts");
await page.waitForTimeout(400);
const rows = (await page.locator(".contact-rows > li").allInnerTexts()).map((t) => t.replace(/\s+/g, " ").trim());
ok(rows.some((t) => t.endsWith("Bluepine Logistics")) && !rows.some((t) => /Ravi Menon$/.test(t)), `the list names the company  ${JSON.stringify(rows)}`);
await page.locator(".contact-rows > li", { hasText: "Bluepine" }).first().click();
await page.waitForTimeout(250);
const head = await page.locator(".contact-head-names h2").innerText();
ok(head === "Bluepine Logistics", `the card is headed by the company  [${head}]`);
const under = await page.locator(".contact-head-names .contact-company").allInnerTexts();
ok(under.includes("Ravi Menon"), `with the person named underneath  ${JSON.stringify(under)}`);
await b.close();
console.log(failures ? `\n${failures} FAILED` : "\nall passed");
process.exit(failures ? 1 : 0);
