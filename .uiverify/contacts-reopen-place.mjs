// Contacts reopens where it was left, as Apple's does: the list and the card open in it, kept on
// this computer (lib/place); a list deleted since falls back to All Contacts.
import { engine } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, contactsFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
const url = `${process.env.URL ?? process.env.BASE ?? "http://localhost:4173"}/?app=contacts`;
const kept = () => page.evaluate(() => JSON.parse(localStorage.getItem("quince.contacts.place") ?? "null"));
const settle = () => page.waitForTimeout(600);

await page.goto(url, { waitUntil: "networkidle" });
await settle();
await page.locator('[aria-current]', { hasText: /^Family/ }).first().click();
await settle();
ok(JSON.stringify(await kept()) === '{"container":"L1","item":"c2"}', `the list and its card are kept  ${JSON.stringify(await kept())}`);
await page.reload({ waitUntil: "networkidle" });
await settle();
const family = await page.locator('[aria-current="true"]', { hasText: /^Family/ }).count();
ok(family > 0, "reopened, Family is the list open");
ok(JSON.stringify(await kept()) === '{"container":"L1","item":"c2"}', `and its card is open  ${JSON.stringify(await kept())}`);

await page.evaluate(() => localStorage.setItem("quince.contacts.place", JSON.stringify({ container: "", item: "c3" })));
await page.reload({ waitUntil: "networkidle" });
await settle();
ok((await kept())?.item === "c3", `the card left open in All is open again  ${JSON.stringify(await kept())}`);

await page.evaluate(() => localStorage.setItem("quince.contacts.place", JSON.stringify({ container: "gone", item: "c3" })));
await page.reload({ waitUntil: "networkidle" });
await settle();
ok(JSON.stringify(await kept()) === '{"container":"","item":"c3"}', `a deleted list falls back to All Contacts, the card still open  ${JSON.stringify(await kept())}`);
await b.close();
process.exitCode = failures ? 1 : 0;
