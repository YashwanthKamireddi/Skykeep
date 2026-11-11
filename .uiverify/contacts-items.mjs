// A pass that names the cards it changed has Contacts read those alone: the card renamed on
// the phone is redrawn, the new one appears, the deleted one goes — and the whole book is not
// read again, nor any other row redrawn. One that cannot name them is read whole, as before;
// and out of sight, the named cards wait for the screen to come back.
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, contactsFixture: true });
await ctx.addInitScript(() => {
  const internals = window.__TAURI_INTERNALS__;
  const invoke = internals.invoke;
  internals.invoke = async (cmd, args) => {
    if (cmd === "contacts_list") window.__reads = (window.__reads ?? 0) + 1;
    if (cmd === "contacts_get") (window.__gets ??= []).push(args.guids);
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
const reads = () => page.evaluate(() => window.__reads ?? 0);
const gets = () => page.evaluate(() => window.__gets ?? []);
const names = () => page.locator(".contact-row .contact-row-name").allInnerTexts();
const pass = (items) => page.evaluate((items) => window.__emit("quince://synced", {
  services: [], asOf: null, tookMs: 0, changed: ["contacts"], ...(items ? { items: { contacts: items } } : {}),
}), items);

// Mark a row that the pass should leave alone, to see that it is not drawn again.
await page.locator(".contact-row", { hasText: "Amma" }).evaluate((e) => { e.__untouched = true; });

// On the phone: Zoya renamed, a card added, the unnamed number deleted.
await page.evaluate(() => {
  const book = window.__contactList;
  // Replaced, not changed in place: the stub hands its own objects to the page.
  const at = book.findIndex((c) => c.guid === "c1");
  book[at] = { ...book[at], lastName: "Rivera" };
  book.push({ guid: "c9", firstName: "Maya", lastName: "Silva", company: null, isCompany: false, birthday: null, notes: null, etag: null, methods: [] });
  book.splice(book.findIndex((c) => c.guid === "c4"), 1);
});
let before = await reads();
await pass({ added: ["c9"], changed: ["c1"], removed: ["c4"] });
await page.locator(".contact-row", { hasText: "Maya Silva" }).waitFor({ timeout: 3000 });
await page.locator(".contact-row", { hasText: "Zoya Rivera" }).waitFor({ timeout: 3000 });
const now = await names();
ok(now.includes("Zoya Rivera") && !now.includes("Zoya Ahuja"), `the card renamed on the phone is redrawn  ${JSON.stringify(now)}`);
ok(now.includes("Maya Silva"), "the card added appears");
ok(!now.includes("+91 90000 11111"), "the card deleted goes");
ok((await reads()) === before, "and the whole book is not read again");
ok(JSON.stringify((await gets()).at(-1)?.slice().sort()) === JSON.stringify(["c1", "c9"]), `only the cards named are read  ${JSON.stringify(await gets())}`);
ok(await page.locator(".contact-row", { hasText: "Amma" }).evaluate((e) => e.__untouched === true), "every other row is left as it was");
ok((await page.locator(".contact-detail h2").innerText()) === "Zoya Rivera", "the open card shows the change");

// A pass that can't name its cards: read whole.
before = await reads();
await pass(null);
await page.waitForFunction((n) => (window.__reads ?? 0) > n, before, { timeout: 3000 }).catch(() => {});
ok((await reads()) === before + 1, "one that cannot name them is read whole, once");

// A pass that changed nothing it shows is not read at all.
before = await reads();
const g = (await gets()).length;
await pass({ added: [], changed: [], removed: [] });
await page.waitForTimeout(250);
ok((await reads()) === before && (await gets()).length === g, "one that names no card reads nothing");

// Out of sight: named cards wait, and are read once on coming back.
await openScreen(page, "Notes");
await page.evaluate(() => {
  const book = window.__contactList;
  const at = book.findIndex((c) => c.guid === "c2");
  book[at] = { ...book[at], firstName: "Amma Jamie" };
});
const g2 = (await gets()).length;
before = await reads();
await pass({ added: [], changed: ["c2"], removed: [] });
await pass({ added: [], changed: ["c3"], removed: [] });
await page.waitForTimeout(250);
ok((await gets()).length === g2, "out of sight, named cards are not read at once");
await openScreen(page, "Contacts");
await page.locator(".contact-row", { hasText: "Amma Jamie" }).waitFor({ timeout: 3000 });
const after = await gets();
ok(after.length === g2 + 1 && after.at(-1).slice().sort().join() === "c2,c3", `coming back reads them, together, once  ${JSON.stringify(after.slice(g2))}`);
ok((await reads()) === before, "still without the whole book");

// With a search typed, the named card is still read alone: the search filters the book in the
// window, so it decides who shows without the library being asked for the whole book.
await page.locator(".contacts-search input").fill("Maya");
await page.waitForFunction(() => document.querySelectorAll(".contact-row").length === 1, null, { timeout: 3000 }).catch(() => {});
await page.evaluate(() => {
  const book = window.__contactList;
  const at = book.findIndex((c) => c.guid === "c9");
  book[at] = { ...book[at], lastName: "Silvana" };
});
before = await reads();
await pass({ added: [], changed: ["c9"], removed: [] });
await page.locator(".contact-row", { hasText: "Maya Silvana" }).waitFor({ timeout: 3000 }).catch(() => {});
ok((await reads()) === before && JSON.stringify(await names()) === JSON.stringify(["Maya Silvana"]),
  `with a search typed, the card is read alone and the search still decides who shows  ${JSON.stringify(await names())}`);
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
