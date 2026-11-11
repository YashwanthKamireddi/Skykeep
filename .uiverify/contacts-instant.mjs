// Contacts' writes are instant, as every write here is: a card saved shows as saved the moment
// Save is pressed — the form away, the name in the list — and a card deleted is gone at once,
// however long the library takes behind. Turned down, the card goes back to what it was and
// the form returns as it was being written. And the list names people as the Mac does, the
// part they are sorted by in bold; Ctrl+L edits the card and Ctrl+Shift+N makes a list.
import { engine, engineName } from "./engine.mjs";
import { SERVICES, autoConfirm, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, contactsFixture: true });
await ctx.addInitScript(autoConfirm);
await ctx.addInitScript(() => {
  // The library slow to take a write (a pass storing ahead of it), or turning it down.
  const internals = window.__TAURI_INTERNALS__;
  const invoke = internals.invoke;
  internals.invoke = async (cmd, args) => {
    if (cmd === "contact_save" || cmd === "contact_delete") {
      await new Promise((r) => setTimeout(r, window.__slow ?? 0));
      if (window.__refuse) throw "The library couldn’t be saved.";
      let out = await invoke(cmd, args);
      if (cmd === "contact_save" && !args.contact.guid && window.__newGuid) out = window.__newGuid;
      const book = window.__contactList;
      if (cmd === "contact_delete") book.splice(book.findIndex((c) => c.guid === args.guid), 1);
      else {
        const card = { ...args.contact, guid: args.contact.guid || out };
        const at = book.findIndex((c) => c.guid === card.guid);
        if (at >= 0) book[at] = card;
        else book.push(card);
      }
      return out;
    }
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
const bold = (name) => page.locator(".contact-row .contact-row-name", { hasText: name }).evaluate((e) => e.querySelector("b")?.textContent ?? null);

// The Mac's list: the sorted-by part in bold.
ok((await bold("Zoya Ahuja")) === "Ahuja", "a person's last name is in bold");
ok((await bold("Northfield University")) === "Northfield University", "a company's name is in bold");
ok((await bold("Amma")) === "Amma", "a first name alone is in bold");
ok((await bold("+91 90000 11111")) === null, "a card with only a number is plain");

// Ctrl+L edits the card open.
await page.locator(".contact-row", { hasText: "Zoya Ahuja" }).click();
await page.keyboard.press("Control+l");
ok(await page.locator(".contact-form h2", { hasText: "Edit contact" }).isVisible(), "Ctrl+L edits the card");

// Saved with the library slow: shown as saved at once.
await page.evaluate(() => { window.__slow = 1500; });
await page.getByLabel("Last", { exact: true }).fill("Rivera");
const t0 = Date.now();
await page.getByRole("button", { name: "Save" }).click();
await page.locator(".contact-row", { hasText: "Zoya Rivera" }).waitFor({ timeout: 1000 });
const took = Date.now() - t0;
ok(took < 1000 && (await page.locator(".contact-form").count()) === 0, `an edit shows as saved at once, the form away  (${took} ms)`);
ok((await page.locator(".contact-detail h2").innerText()) === "Zoya Rivera", "the card shows it");
await page.waitForTimeout(1700);
ok((await names()).includes("Zoya Rivera"), "and it stays once the library has it");

// A new card: in the list and open at once.
await page.keyboard.press("Control+n");
await page.getByLabel("First", { exact: true }).fill("Maya");
await page.getByLabel("Last", { exact: true }).fill("Silva");
await page.getByRole("button", { name: "Add contact" }).click();
await page.locator(".contact-row", { hasText: "Maya Silva" }).waitFor({ timeout: 1000 });
ok((await page.locator(".contact-detail h2").innerText()) === "Maya Silva", "a new card is in the list and open at once");
await page.waitForTimeout(1700);
ok((await page.locator('.contact-row[aria-selected="true"]').getAttribute("data-guid")) === "C-new", "and keeps its place under the id it is given");

// A pass reading the book while the library is still taking a write: the library's older copy
// must not undo the edit on screen, and a new card must stay, and be the one open once saved.
// The read came back without the stand-in, the selection jumped to the first person, and the
// card, saved, was never opened.
const passRead = () => page.evaluate(() => window.__emit("quince://synced", { services: [], asOf: null, tookMs: 0 }));
await page.evaluate(() => { window.__slow = 1500; window.__newGuid = "C-new2"; });
await page.locator(".contact-row", { hasText: "Zoya Rivera" }).click();
await page.keyboard.press("Control+l");
await page.getByLabel("Last", { exact: true }).fill("Rao");
await page.getByRole("button", { name: "Save" }).click();
await page.locator(".contact-row", { hasText: "Zoya Rao" }).waitFor({ timeout: 1000 });
await passRead();
await page.waitForTimeout(400);
ok((await names()).includes("Zoya Rao") && !(await names()).includes("Zoya Rivera"), `an edit stays over a read landing mid-save  ${JSON.stringify(await names())}`);
await page.waitForTimeout(1500);
ok((await names()).includes("Zoya Rao"), "and once the library has it");

await page.keyboard.press("Control+n");
await page.getByLabel("First", { exact: true }).fill("Ines");
await page.getByLabel("Last", { exact: true }).fill("Duarte");
await page.getByRole("button", { name: "Add contact" }).click();
await page.locator(".contact-row", { hasText: "Ines Duarte" }).waitFor({ timeout: 1000 });
await passRead();
await page.waitForTimeout(400);
ok((await names()).includes("Ines Duarte"), `a new card stays over a read landing mid-save  ${JSON.stringify(await names())}`);
ok((await page.locator(".contact-detail h2").innerText()) === "Ines Duarte", "and stays open");
await page.waitForTimeout(1700);
ok((await page.locator('.contact-row[aria-selected="true"]').getAttribute("data-guid")) === "C-new2"
  && (await page.locator(".contact-detail h2").innerText()) === "Ines Duarte", "and is the card open once saved, under its id");

await page.evaluate(() => { window.__slow = 1500; });
await page.locator(".contact-row", { hasText: "Ines Duarte" }).click();
await page.keyboard.press("Delete");
await page.waitForFunction(() => ![...document.querySelectorAll(".contact-row")].some((r) => r.textContent.includes("Ines Duarte")), null, { timeout: 1000 });
await passRead();
await page.waitForTimeout(400);
ok(!(await names()).includes("Ines Duarte"), `a card deleted stays gone over a read landing mid-delete  ${JSON.stringify(await names())}`);
await page.waitForTimeout(1500);
await page.evaluate(() => { window.__slow = 0; });

// Deleted: gone at once.
await page.locator(".contact-row", { hasText: "Amma" }).click();
await page.keyboard.press("Delete");
await page.waitForFunction(() => ![...document.querySelectorAll(".contact-row")].some((r) => r.textContent.includes("Amma")), null, { timeout: 1000 })
  .then(() => ok(true, "a card deleted is gone at once"), () => ok(false, "a card deleted is gone at once"));
await page.waitForTimeout(1700);

// Turned down: back as it was, the form back as it was being written.
await page.evaluate(() => { window.__slow = 300; window.__refuse = true; });
await page.locator(".contact-row", { hasText: "Northfield University" }).click();
await page.keyboard.press("Enter");
await page.getByLabel("Company", { exact: true }).fill("Northfield College");
await page.getByRole("button", { name: "Save" }).click();
await page.locator(".contact-row", { hasText: "Northfield College" }).waitFor({ timeout: 1000 });
await page.locator(".contact-form").waitFor({ timeout: 3000 });
ok((await page.getByLabel("Company", { exact: true }).inputValue()) === "Northfield College", "turned down, the form comes back as it was being written");
ok((await names()).includes("Northfield University") && !(await names()).includes("Northfield College"), "and the card in the list is as it was");
ok(await page.locator(".toast", { hasText: "couldn’t be saved" }).isVisible(), "with why, said");
await page.keyboard.press("Escape");
await page.locator(".contact-row", { hasText: "+91 90000 11111" }).click();
await page.keyboard.press("Delete");
await page.waitForTimeout(800);
ok((await names()).includes("+91 90000 11111"), "a delete turned down puts the card back");
await page.evaluate(() => { window.__refuse = false; window.__slow = 0; });

// Ctrl+Shift+N names a new list in place.
await page.keyboard.press("Control+Shift+N");
ok(await page.locator(".contacts-lists input.notes-folder-rename").isVisible(), "Ctrl+Shift+N starts a new list, its name being typed");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
