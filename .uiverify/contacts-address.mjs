// A card's postal addresses, part by part: shown in the order the country writes them, with
// a way to the map; edited as Contacts on the Mac lays an address out; written at once, and
// put back with a toast when Apple turns the change down.
//
//   URL=http://localhost:4204 SHOTS=/some/dir ENGINE=webkit node .uiverify/contacts-address.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const URL = process.env.URL ?? process.env.URL ?? "http://localhost:4173";
const SHOTS = process.env.SHOTS ?? null;
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

for (const scheme of ["light", "dark"]) {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 860 }, colorScheme: scheme });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
    calendarFixture: false, notesFixture: false, contactsFixture: true, driveFixture: false, remindersFixture: false });
  const page = await ctx.newPage();
  await page.goto(URL, { waitUntil: "networkidle" });
  await openScreen(page, "Contacts");
  await page.locator(".contact-row", { hasText: "Zoya" }).click();
  const card = page.locator(".contact-detail");
  await card.locator(".contact-postal").first().waitFor();
  const shot = async (name) => SHOTS && card.screenshot({ path: `${SHOTS}/${engineName}-${scheme}-${name}.png` });

  if (scheme === "dark") {
    // The look, in the dark: the card and the editor.
    await shot("card");
    await page.getByRole("button", { name: "Edit", exact: true }).click();
    await page.getByLabel("Street, address 1").waitFor();
    await shot("edit");
    await ctx.close();
    continue;
  }

  // ── Shown in the country's own order ──
  const texts = await card.locator(".contact-postal .contact-address").allInnerTexts();
  ok(texts[0] === "Hauptstraße 5\nHinterhaus\n10115 Berlin\nGermany", `a German address puts the postcode before the town  ${JSON.stringify(texts[0])}`);
  ok(texts[1] === "12 MG Road\nBengaluru 560001\nKarnataka\nIndia", `an Indian one puts the state under the town  ${JSON.stringify(texts[1])}`);
  const labels = await card.locator(".contact-postal dt").allInnerTexts();
  ok(/home/.test(labels[0]) && /work/.test(labels[1]), `labels read as Apple's own  ${JSON.stringify(labels)}`);
  const maps = await card.locator(".contact-postal .contact-maps").first().getAttribute("href");
  ok(maps?.startsWith("geo:0,0?q=") && decodeURIComponent(maps).includes("10115 Berlin"), `Show in Maps searches the address  ${maps}`);
  await shot("card");

  // ── Edit one part; Return saves, at once ──
  await page.getByRole("button", { name: "Edit", exact: true }).click();
  const street = page.getByLabel("Street, address 1");
  await street.waitFor();
  ok((await street.inputValue()) === "Hauptstraße 5\nHinterhaus", "the street is one multi-line field");
  // Tab walks the parts in the order they are laid out.
  await street.focus();
  const order = [];
  for (let i = 0; i < 4; i++) {
    await page.keyboard.press("Tab");
    order.push(await page.evaluate(() => document.activeElement?.getAttribute("aria-label")));
  }
  ok(JSON.stringify(order) === JSON.stringify(["City, address 1", "State, address 1", "Postcode, address 1", "Country, address 1"]), `Tab goes street, city, state, postcode, country  ${JSON.stringify(order)}`);
  const [sTop, cTop, stTop, pTop, coTop] = await Promise.all(["Street", "City", "State", "Postcode", "Country"].map(async (n) => (await page.getByLabel(`${n}, address 1`).boundingBox()).y));
  ok(cTop === stTop && stTop === pTop && sTop < cTop && cTop < coTop, "city, state and postcode share a line, between street and country");
  await shot("edit");
  const city = page.getByLabel("City, address 1");
  await city.fill("Potsdam");
  await page.getByLabel("Postcode, address 1").fill("14467");
  await page.keyboard.press("Enter");
  await page.locator(".contact-form").waitFor({ state: "detached" });
  // Drawn before Apple answers (the stub takes 400 ms).
  const drawn = await card.locator(".contact-postal .contact-address").first().innerText();
  const sentYet = await page.evaluate(() => (window.__addressEdits ?? []).length);
  ok(drawn.includes("14467 Potsdam"), `the change shows at once  ${JSON.stringify(drawn)}`);
  await page.waitForFunction(() => (window.__addressEdits ?? []).length === 1);
  const first = await page.evaluate(() => window.__addressEdits[0]);
  ok(JSON.stringify(first.edits) === JSON.stringify([
    { op: "setField", index: 0, field: "city", value: "Potsdam" },
    { op: "setField", index: 0, field: "postalCode", value: "14467" },
  ]) && first.guid === "c1" && first.base?.length === 2, `only the parts changed are sent, against the list the form opened  ${JSON.stringify(first.edits)}  (queued ${sentYet})`);
  const saves = await page.evaluate(() => (window.__contactSaves ?? []).length);
  ok(saves === 0, "a change to an address alone does not rewrite the rest of the card");

  // ── Esc leaves an edit unsaved ──
  await page.getByRole("button", { name: "Edit", exact: true }).click();
  await page.getByLabel("City, address 1").fill("Hamburg");
  await page.keyboard.press("Escape");
  await page.waitForTimeout(500);
  ok((await page.evaluate(() => (window.__addressEdits ?? []).length)) === 1 && !(await card.innerText()).includes("Hamburg"), "Escape leaves the address as it was");

  // ── Add one, with its label ──
  await page.getByRole("button", { name: "Edit", exact: true }).click();
  await page.getByRole("button", { name: "Add Field" }).click();
  await page.getByRole("menuitem", { name: "Address" }).click();
  const newStreet = page.getByLabel("Street, address 3");
  ok(await newStreet.evaluate((el) => el === document.activeElement), "a new address starts in its street");
  await newStreet.fill("1 Main St");
  await page.getByLabel("City, address 3").fill("Springfield");
  await page.getByLabel("State, address 3").fill("IL");
  await page.getByLabel("Postcode, address 3").fill("62701");
  await page.getByLabel("Country, address 3").fill("United States");
  await page.getByLabel("Label for address 3").click();
  await page.getByRole("option", { name: "Custom" }).click();
  await page.getByLabel("Custom label for address 3").fill("cabin");
  await page.getByRole("button", { name: "Save" }).click();
  await page.waitForFunction(() => (window.__addressEdits ?? []).length === 2);
  const add = await page.evaluate(() => window.__addressEdits[1].edits);
  ok(add.length === 1 && add[0].op === "add" && add[0].address.label === "cabin" && add[0].address.countryCode === "us", `added with its label and country code  ${JSON.stringify(add)}`);
  await page.waitForTimeout(500);
  const third = await card.locator(".contact-postal").nth(2).innerText();
  ok(third.includes("Springfield, IL 62701") && third.includes("cabin"), `an American address is written the American way  ${JSON.stringify(third)}`);

  // ── Remove one ──
  await page.getByRole("button", { name: "Edit", exact: true }).click();
  await page.getByRole("button", { name: "Remove address 2" }).click();
  await page.getByRole("button", { name: "Save" }).click();
  ok((await card.locator(".contact-postal").count()) === 2 && !(await card.innerText()).includes("Bengaluru"), "a removed address leaves the card at once");
  await page.waitForFunction(() => (window.__addressEdits ?? []).length === 3);
  ok(JSON.stringify(await page.evaluate(() => window.__addressEdits[2].edits)) === JSON.stringify([{ op: "remove", index: 1 }]), "sent as one removal");
  await page.waitForTimeout(500);

  // ── Turned down: put back, with a toast ──
  await page.evaluate(() => { window.__addressFail = true; window.__addressDelay = 600; });
  await page.getByRole("button", { name: "Edit", exact: true }).click();
  await page.getByLabel("Label for address 1").click();
  await page.getByRole("option", { name: "work", exact: true }).click();
  await page.getByLabel("Street, address 1").fill("Unter den Linden 1");
  await page.getByLabel("Street, address 1").press("Enter");
  const during = await card.locator(".contact-postal").first().innerText();
  ok(during.includes("Unter den Linden 1") && /work/.test(during), "shown at once while Apple decides");
  const toast = page.locator(".toast", { hasText: "Address not saved" });
  await toast.waitFor({ timeout: 4000 });
  const after = await card.locator(".contact-postal").first().innerText();
  ok(after.includes("Hauptstraße 5") && !after.includes("Unter den Linden") && /home/.test(after), `turned down, the address goes back  ${JSON.stringify(after)}`);
  ok(await toast.getByRole("button", { name: "Try Again" }).isVisible(), "the toast offers to try again");
  await toast.getByRole("button", { name: "Try Again" }).click();
  await page.waitForFunction(() => (window.__addressEdits ?? []).length === 5);
  await page.waitForTimeout(700);
  ok((await card.locator(".contact-postal").first().innerText()).includes("Unter den Linden 1"), "trying again writes it");

  // ── A brand-new card takes an address too: the card is saved, then its address queued
  //    behind it under the id the card was given (outbox.rs keeps the order) ──
  await page.evaluate(() => { window.__addressDelay = 50; });
  const editsBefore = await page.evaluate(() => (window.__addressEdits ?? []).length);
  await page.getByRole("button", { name: "New contact" }).click();
  await page.locator(".contact-form").waitFor();
  await page.getByLabel("First", { exact: true }).fill("Meera");
  await page.getByRole("button", { name: "Add Field" }).click();
  await page.getByRole("menuitem", { name: "Address" }).click();
  await page.getByLabel("Street, address 1").fill("4 Lake View");
  await page.getByLabel("City, address 1").fill("Chennai");
  await page.getByLabel("Country, address 1").fill("India");
  await page.getByRole("button", { name: "Add contact" }).click();
  await page.waitForFunction((n) => (window.__addressEdits ?? []).length === n + 1, editsBefore);
  const fresh = await page.evaluate(() => ({ save: window.__contactSaves.at(-1), edit: window.__addressEdits.at(-1) }));
  ok(fresh.save.contact.firstName === "Meera" && !fresh.save.contact.guid, `the new card is saved first  ${JSON.stringify(fresh.save.contact.firstName)}`);
  ok(fresh.edit.guid === "C-new" && fresh.edit.base.length === 0 && fresh.edit.edits.length === 1 && fresh.edit.edits[0].op === "add"
    && fresh.edit.edits[0].address.city === "Chennai" && fresh.edit.edits[0].address.countryCode === "in",
    `its address goes as one addition, to the card's new id  ${JSON.stringify(fresh.edit)}`);
  await ctx.close();
}
await b.close();
process.exitCode = failures === 0 ? 0 : 1;
