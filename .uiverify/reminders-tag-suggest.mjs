// The tag field offers the tags already in use as one types, as Reminders does on a Mac and an
// iPhone (#63): those starting with what is typed first, then those containing it, never one
// the reminder already carries; picked with the arrows and Return or a click; Escape closes the
// list alone; and a new tag is still typed and added as before.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.locator(".rx-list", { hasText: "Daily" }).click();
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const writes = () => page.evaluate(() => window.__detailWrites ?? []);
const offered = () => page.locator(".rx-detail .rx-tag-suggest [role=option]").allInnerTexts();
const field = page.locator(".rx-detail .rx-tag-input");

// Pick up the parcel carries #errands and #home; the account also has #fitness.
await page.getByRole("button", { name: "Details for Pick up the parcel" }).click();
await page.waitForTimeout(300);
await field.click();
ok(JSON.stringify(await offered()) === "[]", "nothing is offered before anything is typed");
await field.pressSequentially("fi");
ok(JSON.stringify(await offered()) === JSON.stringify(["#fitness"]), `typing offers the tags in use that start with it  ${await offered()}`);
await field.fill("");
await field.pressSequentially("e");
const e = await offered();
ok(!e.includes("#errands") && !e.includes("#home"), `never one the reminder already carries  ${e}`);
ok(JSON.stringify(e) === JSON.stringify(["#fitness"]), `those containing it come after any starting with it  ${e}`);
ok((await field.getAttribute("aria-expanded")) === "true" && (await field.getAttribute("aria-activedescendant")) !== null, "the field says a list is open and which one is lit");
await page.keyboard.press("Escape");
ok(JSON.stringify(await offered()) === "[]" && (await field.inputValue()) === "e" && (await page.locator(".rx-detail").count()) === 1,
  "Escape closes the list alone: the words typed and the pane stay");
await field.fill("");
await field.pressSequentially("fit");
await page.keyboard.press("Enter");
await page.waitForTimeout(300);
ok((await page.locator(".rx-detail .rx-tag-name").allInnerTexts()).includes("#fitness"), "Return takes the tag offered");
ok((await writes()).some((w) => w.cmd === "reminder_tag_add" && w.name === "fitness" && w.guid === "r2"), "and it is sent as that tag");
ok((await field.inputValue()) === "", "and the field is ready for the next");

// Another reminder: arrows move through several, a click takes one.
await page.getByRole("button", { name: "Details for Water the plants" }).click();
await page.waitForTimeout(300);
await field.click();
await field.pressSequentially("s");
const s = await offered();
ok(JSON.stringify(s) === JSON.stringify(["#errands", "#fitness"]), `several, in order  ${s}`);
await page.keyboard.press("ArrowDown");
ok((await page.locator(".rx-detail .rx-tag-option.is-lit").innerText()) === "#fitness", "the down arrow lights the next");
await page.locator(".rx-detail .rx-tag-option", { hasText: "#errands" }).click();
await page.waitForTimeout(300);
ok((await page.locator(".rx-detail .rx-tag-name").allInnerTexts()).includes("#errands"), "a click takes the one clicked");

// Tab takes the lit one, as a Mac's token fields do, and leaves the caret there for the next.
await field.click();
await field.pressSequentially("fi");
await page.keyboard.press("Tab");
await page.waitForTimeout(300);
ok((await page.locator(".rx-detail .rx-tag-name").allInnerTexts()).includes("#fitness"), "Tab takes the tag offered");
ok((await writes()).some((w) => w.cmd === "reminder_tag_add" && w.name === "fitness" && w.guid !== "r2"), "and it is sent as that tag");
ok(await field.evaluate((el) => el === document.activeElement && el.value === ""), "and the caret stays in the emptied field");

// A new tag, typed whole: added as before.
await field.click();
await field.pressSequentially("garden");
ok(JSON.stringify(await offered()) === "[]", "a word no tag has offers nothing");
await page.keyboard.press("Enter");
await page.waitForTimeout(300);
ok((await page.locator(".rx-detail .rx-tag-name").allInnerTexts()).includes("#garden"), "and Return adds it as a new tag");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
