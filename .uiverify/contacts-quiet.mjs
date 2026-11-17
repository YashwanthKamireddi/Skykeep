// Contacts reads its book again only when there is something new in it. A pass that landed
// with nothing new, or with something new only for another app, leaves it alone; one that
// changed contacts — or one that cannot say what it changed — is read. A card sent while
// another app is in front is read once on coming back, not once per send out of sight.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, contactsFixture: true });
await ctx.addInitScript(() => {
  const internals = window.__TAURI_INTERNALS__;
  const invoke = internals.invoke;
  internals.invoke = async (cmd, args) => {
    if (cmd === "contacts_list") window.__reads = (window.__reads ?? 0) + 1;
    return invoke(cmd, args);
  };
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Contacts");
await page.waitForTimeout(500);

let failures = 0;
const ok = (p, m) => { console.log(`${p ? "ok  " : "FAIL"} ${m}`); if (!p) failures++; };
const reads = () => page.evaluate(() => window.__reads ?? 0);
const status = { services: [], asOf: null, tookMs: 0 };
const synced = (changed) => page.evaluate(([s, c]) => window.__emit("quince://synced", c === undefined ? s : { ...s, changed: c }), [status, changed]);
const sent = (services) => page.evaluate((s) => window.__emit("quince://outbox-changed", { services: s }), services);

ok((await page.locator(".contact-row").count()) > 0, "the book is on screen");

let before = await reads();
await synced([]);
await page.waitForTimeout(300);
ok((await reads()) === before, "a pass that stored nothing new is not read again");

await synced(["notes", "photos"]);
await page.waitForTimeout(300);
ok((await reads()) === before, "nor one that changed only other apps");

await synced(["contacts"]);
await page.waitForTimeout(300);
ok((await reads()) === before + 1, "one that changed contacts is read, once");

before = await reads();
await synced(undefined);
await page.waitForTimeout(300);
ok((await reads()) === before + 1, "one that cannot say what it changed is read, as always");

// Another app in front: a contact sent meanwhile waits for the screen to come back.
await openScreen(page, "Notes");
await page.waitForTimeout(400);
before = await reads();
await sent(["contacts"]);
await sent(["contacts"]);
await page.waitForTimeout(300);
ok((await reads()) === before, "out of sight, a card sent is not read at once");
await openScreen(page, "Contacts");
await page.waitForTimeout(400);
ok((await reads()) === before + 1, "and is read once on coming back");

// In front, a card sent is read at once, as before.
before = await reads();
await sent(["contacts"]);
await page.waitForTimeout(300);
ok((await reads()) === before + 1, "in front, a card sent is read at once");

await b.close();
if (failures) process.exitCode = 1;
