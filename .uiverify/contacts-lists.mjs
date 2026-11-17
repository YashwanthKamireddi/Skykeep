// Contacts' lists, as on the Mac and icloud.com: All Contacts and each list in a column,
// a list shows only its people, and every change — a new list, a rename, someone added by
// menu or by dragging, someone taken out, a list deleted — shows the moment it is made,
// though Apple takes most of a second to answer. A change Apple refuses goes back.
import { engine } from "./engine.mjs";
import { SERVICES, autoConfirm, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, contactsFixture: true });
const page = await ctx.newPage();
await ctx.addInitScript(autoConfirm);
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Contacts");
await page.waitForTimeout(400);

const failures = [];
const check = (ok, what) => {
  console.log(`${ok ? "ok  " : "FAIL"} ${what}`);
  if (!ok) failures.push(what);
};
const side = page.locator(".contacts-lists");
const row = (name) => side.locator(".notes-folder-row", { has: page.locator(".notes-folder-name", { hasText: new RegExp(`^${name}$`) }) });
// textContent, not innerText: a hovered row hides its count to make room for its ⋯.
const count = async (name) => Number((await row(name).locator(".notes-folder-count").textContent()).trim() || 0);
const people = async () =>
  (await page.locator(".contact-row-name").allInnerTexts()).map((t) => t.trim());
const quick = 200; // well inside the stub's 700 ms answer

check((await side.locator(".notes-folder-name").allInnerTexts()).join("|") === "All Contacts|Family", "the column holds All Contacts, then the lists");
await row("Family").locator(".notes-folder").click();
check((await page.locator(".contacts-list h1").innerText()) === "Family", "a list's name heads the people");
check(JSON.stringify(await people()) === JSON.stringify(["Amma"]), "a list shows only its people");

// Add by the right-click menu, a step into its lists.
await row("All Contacts").locator(".notes-folder").click();
await page.locator(".contact-row", { hasText: "Zoya Ahuja" }).click({ button: "right" });
// "Add to List" opens its lists beside it, as a Mac submenu does.
await page.getByRole("menuitem", { name: "Add to List" }).hover();
await page.getByRole("menuitem", { name: "Family" }).click();
await page.waitForTimeout(quick);
check((await count("Family")) === 2, "added by menu, the count moves at once");

// A new list, named in place.
await side.getByRole("button", { name: "New List" }).click();
const naming = side.getByRole("textbox", { name: "New list name" });
check(await naming.isVisible(), "New List opens its name for typing straight away");
await naming.fill("Work");
await naming.press("Enter");
await page.waitForTimeout(quick);
check(await row("Work").isVisible(), "the new list is in the column at once");
await page.waitForTimeout(900);

// Dragged onto a list, as on the Mac.
await page.locator(".contact-rows > li", { hasText: "Northfield University" }).dragTo(row("Work"));
await page.waitForTimeout(quick);
check((await count("Work")) === 1, "a contact dragged onto a list joins it at once");
await page.waitForTimeout(900);

// Rename from the row's ⋯.
await row("Work").hover();
await row("Work").getByRole("button", { name: "Actions for Work" }).click();
await page.getByRole("menuitem", { name: "Rename List" }).click();
const rename = side.getByRole("textbox", { name: "Rename Work" });
await rename.fill("Office");
await rename.press("Enter");
await page.waitForTimeout(quick);
check(await row("Office").isVisible(), "a rename shows at once");
await page.waitForTimeout(900);

// Taken out from the card.
await row("Office").locator(".notes-folder").click();
await page.locator(".contact-row", { hasText: "Northfield University" }).click();
await page.getByRole("button", { name: "Remove from List" }).click();
await page.waitForTimeout(quick);
check((await people()).length === 0 && (await count("Office")) === 0, "taken out of a list, they leave it at once");
await page.waitForTimeout(900);

// Refused by Apple: drawn, then put back, with the reason.
await page.evaluate(() => { window.__listFail = true; });
await row("All Contacts").locator(".notes-folder").click();
await page.locator(".contact-row", { hasText: "Zoya Ahuja" }).click();
await page.getByRole("button", { name: "Add to List" }).click();
await page.getByRole("menuitem", { name: "Office" }).click();
await page.waitForTimeout(quick);
const drawn = await count("Office");
await page.waitForTimeout(1100);
check(drawn === 1 && (await count("Office")) === 0, "a refused add shows, then goes back");
check(await page.locator(".toast.is-error").first().isVisible().catch(() => false), "and says why");
await page.evaluate(() => { window.__listFail = false; });

// Deleted.
await row("Office").hover();
await row("Office").getByRole("button", { name: "Actions for Office" }).click();
await page.getByRole("menuitem", { name: "Delete List" }).click();
await page.waitForTimeout(quick);
check((await row("Office").count()) === 0, "a deleted list leaves the column at once");
await page.waitForTimeout(900);

// Delete, inside a list, asks: out of the list, or the card everywhere.
await row("Family").locator(".notes-folder").click();
await page.locator(".contact-row", { hasText: "Amma" }).click();
await page.locator(".contact-detail").getByRole("button", { name: "Delete", exact: true }).click();
const asked = page.getByRole("alertdialog", { name: "Delete contact" });
check(await asked.isVisible(), "deleting inside a list asks which is meant");
await asked.getByRole("button", { name: "Remove from List" }).click();
await page.waitForTimeout(quick);
check((await count("Family")) === 1 && !(await people()).includes("Amma"), "Remove from List takes them out at once");
check(!(await page.evaluate(() => window.__contactDeletes?.length)), "and the card itself is left alone");
await page.waitForTimeout(900);

const writes = await page.evaluate(() => window.__listWrites);
check(
  JSON.stringify(writes.map((w) => Object.keys(w).sort().join(","))) ===
    JSON.stringify(["add,guid", "add,name", "add,guid", "guid,name", "guid,remove", "add,guid", "delete,guid", "guid,remove"]),
  "each change goes to Apple as what changed, not a guessed whole list"
);

await b.close();
console.log(failures.length ? `FAIL ${failures.length} contacts-lists check(s)` : "PASS contacts lists, instant and as Apple keeps them");
process.exitCode = failures.length ? 1 : 0;
