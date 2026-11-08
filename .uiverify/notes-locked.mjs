// A locked note, as Notes shows one: a calm lock screen, the hint on request, a wrong password
// said plainly, then the note read-only with Lock Now — and the password kept nowhere.
//   BASE=http://localhost:4173 ENGINE=webkit node .uiverify/notes-locked.mjs
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const BASE = process.env.BASE ?? "http://localhost:4173";
const PASSWORD = "marigold-7";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 820 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
  calendarFixture: false, notesFixture: true, contactsFixture: false, driveFixture: false, remindersFixture: false });
await ctx.addInitScript(() => { window.__lockedNote = true; });
const page = await ctx.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.waitForTimeout(500);

let failures = 0;
const ok = (pass, yes, no) => { console.log(pass ? `PASS ${yes}` : `FAIL ${no}`); if (!pass) failures++; };

const row = page.locator(".note-row", { hasText: "Bank" });
ok(await row.locator(".note-row-lock").count() === 1 && (await row.textContent()).includes("Locked"),
  "a locked note shows its lock in the list", `the row reads ${await row.textContent()}`);

// From another note, so the lock screen is drawn by the click.
await page.locator(".note-row", { hasText: "Curtains" }).click();
await page.waitForTimeout(300);
await row.click();
await page.waitForTimeout(300);
const lock = page.locator(".note-lock");
ok(await lock.isVisible() && (await lock.textContent()).includes("This note is locked"),
  "it opens on the lock screen", "no lock screen");
const field = page.getByLabel("Note password");
ok(await field.evaluate((el) => el === document.activeElement), "the password field takes the cursor after a click",
  "focus is elsewhere");
ok(await page.getByRole("button", { name: "Unlock" }).isDisabled(), "Unlock waits for a password", "Unlock is live with nothing typed");
ok(await page.locator(".note-rich, [contenteditable='true']").count() === 0, "nothing on it is editable", "an editor is open");

await page.getByRole("button", { name: "Show Hint" }).click();
await page.waitForTimeout(200);
ok((await lock.textContent()).includes("Hint: The fruit"), "the hint shows when asked for", `lock reads ${await lock.textContent()}`);

await field.fill("wrong one");
await field.press("Enter");
await page.waitForTimeout(300);
const problem = (await page.locator(".note-lock-problem").textContent()) ?? "";
ok(/didn’t match/.test(problem) && !/error|invalid/i.test(problem), "a wrong password is said plainly",
  `said ${JSON.stringify(problem)}`);
ok(await field.inputValue() === "" && await field.evaluate((el) => el === document.activeElement),
  "the field is cleared and ready for another try", "the wrong password stayed, or focus left");

await field.fill(PASSWORD);
await page.getByRole("button", { name: "Unlock" }).click();
await page.waitForTimeout(400);
const shown = page.locator(".note-editor.is-unlocked");
ok(await shown.isVisible() && (await shown.textContent()).includes("PIN for the locker 4821"),
  "the right password opens the note", "the note did not open");
ok(await shown.locator("h1").textContent() === "Bank", "with its title set as a title", "title missing");
ok(await shown.locator(".nf-check").count() === 1, "and its checklist drawn", "checklist missing");
ok(await page.locator("[contenteditable='true']").count() === 0 && await shown.locator(".nf-circle:not([disabled])").count() === 0,
  "read-only: no editor, no live checkboxes", "something in it can be changed");
ok(await page.getByRole("button", { name: "Lock Now" }).isVisible(), "Lock Now is in the toolbar", "no Lock Now");

// The password is nowhere the page keeps things.
const kept = await page.evaluate((pw) => {
  const all = [];
  for (const s of [localStorage, sessionStorage]) for (let i = 0; i < s.length; i++) all.push(s.key(i), s.getItem(s.key(i)));
  return all.join("\n").includes(pw) || document.documentElement.outerHTML.includes(pw)
    || [...document.querySelectorAll("input")].some((i) => i.value.includes(pw));
}, PASSWORD);
ok(!kept, "the password is not kept in storage or the page", "the password is still in the page");

// Away and back: still open, without asking again.
await page.locator(".note-row", { hasText: "Curtains" }).click();
await page.waitForTimeout(300);
await row.click();
await page.waitForTimeout(300);
const asks = await page.evaluate(() => (window.__unlockAsks ?? []).length);
ok(await shown.isVisible() && asks === 2, "coming back, it is still open", `asked ${asks} times, open ${await shown.isVisible()}`);

await page.getByRole("button", { name: "Lock Now" }).click();
await page.waitForTimeout(300);
ok(await lock.isVisible() && await page.evaluate(() => window.__lockForgets) === 1,
  "Lock Now locks it again, and the backend lets the password go", "it stayed open");
ok(await page.getByRole("button", { name: "Lock Now" }).count() === 0, "and Lock Now goes with it", "Lock Now still there");

// Arrowing onto a locked note keeps the cursor in the list.
await page.locator(".note-row", { hasText: "Curtains" }).click();
await page.waitForTimeout(300);
await page.keyboard.press("ArrowUp");
await page.waitForTimeout(300);
ok(await lock.isVisible() && await page.evaluate(() => Boolean(document.activeElement?.closest(".note-rows"))),
  "moving onto it with the arrows leaves focus in the list", "focus jumped into the password field");

ok(errors.length === 0, "no errors on the page", errors.join("; "));
await b.close();
process.exitCode = failures === 0 ? 0 : 1;
