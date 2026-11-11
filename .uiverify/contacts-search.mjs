// Contacts' search filters the book the window already holds. It used to ask the library on
// every key: each read built every card in Rust and sent the whole book back (about 5 MB at
// 10,000 cards), and the list sorted and redrew it, key by key. Now typing asks the library
// nothing, the list follows once typing pauses, and Return goes to the first of what was typed
// even when pressed before the list caught up. Who is found is the library's own rule
// (contacts-search.test.mjs): a nickname, a number however it is written.
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(() => {
  window.__contactsExtra = [
    { guid: "c5", firstName: "Ravi", lastName: "Menon", nickname: "Bunny", company: null, isCompany: false,
      birthday: null, notes: null, etag: null, methods: [{ kind: "phone", label: "mobile", value: "98765 12345" }] },
  ];
});
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, contactsFixture: true });
await ctx.addInitScript(() => {
  const internals = window.__TAURI_INTERNALS__;
  const invoke = internals.invoke;
  internals.invoke = async (cmd, args) => {
    if (cmd === "contacts_list") (window.__asked ??= []).push(args?.search ?? null);
    return invoke(cmd, args);
  };
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Contacts");
await page.locator(".contact-row", { hasText: "Zoya Ahuja" }).waitFor();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName}: ${m}`); if (!p) failures++; };
const names = () => page.locator(".contact-row .contact-row-name").allInnerTexts();
const field = page.getByLabel("Search contacts");

await page.evaluate(() => { window.__asked = []; });
await field.click();
await page.keyboard.type("bunny", { delay: 30 });
await page.waitForTimeout(400);
const asked = await page.evaluate(() => window.__asked);
ok(asked.length === 0, `typing asks the library nothing  ${JSON.stringify(asked)}`);
ok(JSON.stringify(await names()) === JSON.stringify(["Ravi Menon"]), `a nickname finds the person  ${JSON.stringify(await names())}`);

await field.fill("");
await page.waitForTimeout(50);
ok((await names()).length === 5, `cleared, everyone is back at once  ${JSON.stringify(await names())}`);

await field.fill("+91 98765 12345");
await page.waitForTimeout(400);
ok(JSON.stringify(await names()) === JSON.stringify(["Ravi Menon"]), `a whole number with its country code finds the card without it  ${JSON.stringify(await names())}`);
await field.fill("zz-nobody");
await page.waitForTimeout(400);
ok(await page.locator(".contact-empty", { hasText: "No matches." }).isVisible(), `no one found says so  ${JSON.stringify(await names())}`);

// Return straight after the last key, before the list has followed it.
await field.fill("");
await page.waitForTimeout(50);
await page.locator(".contact-row", { hasText: "Northfield University" }).click();
await field.click();
await page.keyboard.type("amma");
await page.keyboard.press("Enter");
await page.waitForTimeout(300);
ok((await page.locator(".contact-detail h2").innerText()) === "Amma", `Return at once opens the first of what was typed  "${await page.locator(".contact-detail h2").innerText()}"`);

// Return confirming an input method's candidate is the input method's, not the search's.
await field.fill("");
await page.waitForTimeout(50);
await page.locator(".contact-row", { hasText: "Northfield University" }).click();
await field.click();
await field.fill("zoya");
await page.waitForTimeout(300);
await field.evaluate((el) => el.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", keyCode: 229, isComposing: true, bubbles: true, cancelable: true })));
await page.waitForTimeout(200);
ok(await field.evaluate((el) => el === document.activeElement), "Return while composing leaves the focus in the search, not on the first match");

await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
