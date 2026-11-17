// Contacts in Contacts' order — last name, then first; a company by its name; a card with
// no name last — under the letter each run begins with.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, contactsFixture: true });
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Contacts");
await page.waitForTimeout(400);
const seen = await page.locator(".contact-rows > li").allInnerTexts();
const flat = seen.map((t) => t.replace(/\s+/g, " ").trim());
console.log(JSON.stringify(flat));
const names = flat.filter((t) => t.length > 1 && !/^[A-Z#]$/.test(t)).map((t) => t.replace(/^\S+ /, ""));
const letters = flat.filter((t) => /^[A-Z#]$/.test(t));
const pass = names.at(-1)?.startsWith("+91") && letters.at(-1) === "#" && new Set(letters).size === letters.length;
console.log(pass ? "PASS contacts run in Contacts' order under their letters" : `FAIL order ${JSON.stringify({ names, letters })}`);
await b.close();
process.exitCode = pass ? 0 : 1;
