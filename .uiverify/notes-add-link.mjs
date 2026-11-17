// Add Link in Notes (#22), as Notes has it on a Mac (Command-K): an address put on the selected
// words and saved as Apple keeps a link; the link the cursor is in edited or taken off; with
// nothing selected, the words asked for and put in linked; an address Skykeep would not open
// refused in place; Escape back into the note with nothing changed; the Aa panel's Link button
// opening the same box, which stays on screen.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.locator(".note-row", { hasText: "Curtains" }).first().click();
await page.waitForTimeout(500);

const lastSave = async () => { await page.waitForTimeout(1200); return page.evaluate(() => window.__noteSaves?.at(-1)); };
const runAt = (save, text) => {
  const at = save.body.indexOf(text);
  let n = 0;
  for (const r of save.runs ?? []) { if (at < n + r.length) return r; n += r.length; }
  return null;
};
const box = page.getByRole("dialog", { name: /^(Add|Edit) Link$/ });
const inNote = () => page.evaluate(() => Boolean(document.activeElement?.closest(".note-rich")));
const fan = page.locator(".note-rich .nf-p", { hasText: /^Fan/ }).first();

// The word selected, Ctrl+K, an address typed as people type one.
await fan.click();
await page.keyboard.press("End");
await page.keyboard.press("Shift+Home");
await page.keyboard.press("Control+k");
ok(await box.isVisible(), "Ctrl+K opens Add Link");
ok(await page.evaluate(() => document.activeElement?.closest(".link-box") !== null), "with the address field ready to type in");
ok((await box.getByLabel("Text").count()) === 0, "words selected: no text asked for");
await page.keyboard.type("example.com");
await page.keyboard.press("Enter");
await page.waitForTimeout(150);
ok(!(await box.isVisible()), "Return adds it and closes the box");
ok((await fan.locator('a[href="https://example.com/"]').innerText()) === "Fan", "the word is a link to https://example.com/");
ok(await inNote(), "and the cursor is back in the note");
let save = await lastSave();
ok(save && runAt(save, "Fan")?.link === "https://example.com/", "saved as Apple keeps a link");
// Typing on after it is plain text, not more of the link.
await page.keyboard.type("!");
await page.waitForTimeout(100);
ok((await fan.locator("a").innerText()) === "Fan", "typing on after it is not part of the link");

// The cursor in the link: Edit Link, its address there; Remove Link leaves the words.
await fan.click();
await page.keyboard.press("Home");
await page.keyboard.press("ArrowRight");
await page.keyboard.press("Control+k");
ok((await box.getAttribute("aria-label")) === "Edit Link", "in a link, Ctrl+K edits it");
ok((await box.getByRole("textbox").first().inputValue()) === "https://example.com/", "with its address there");
await box.getByRole("button", { name: "Remove Link" }).click();
await page.waitForTimeout(150);
ok((await fan.locator("a").count()) === 0 && (await fan.innerText()).startsWith("Fan"), "Remove Link takes it off and keeps the words");
save = await lastSave();
ok(save && !runAt(save, "Fan")?.link, "and the save has no link");

// Nothing selected: the words asked for, put in at the cursor, linked.
await page.keyboard.press("End");
await page.keyboard.type(" ");
await page.keyboard.press("Control+k");
ok((await box.getByLabel("Text").count()) === 1, "nothing selected: the text to show is asked for");
await box.getByLabel("Link to").fill("help@example.com");
await box.getByLabel("Text").fill("Write to us");
await page.keyboard.press("Enter");
await page.waitForTimeout(150);
ok((await fan.locator('a[href="mailto:help@example.com"]').innerText()) === "Write to us", "the words go in, linked to the email address");

// An address Skykeep would not open: refused where it is typed, nothing changed.
await page.keyboard.press("End");
await page.keyboard.press("Shift+Home");
const before = await fan.innerHTML();
await page.keyboard.press("Control+k");
await box.getByLabel("Link to").fill("javascript:alert(1)");
await page.keyboard.press("Enter");
ok(await box.getByRole("alert").isVisible(), "a javascript: address is refused with a reason");
ok(await box.isVisible(), "and the box stays for it to be corrected");
await page.keyboard.press("Escape");
await page.waitForTimeout(100);
ok(!(await box.isVisible()) && (await inNote()), "Escape closes it, back in the note");
ok((await fan.innerHTML()) === before, "with nothing changed");

// The Aa panel's Link button: the same box, wholly on screen.
await page.keyboard.press("End");
await page.keyboard.press("Shift+Home");
await page.getByRole("button", { name: "Format", exact: true }).click();
await page.locator(".format-panel").getByRole("button", { name: "Add Link" }).click();
ok(await box.isVisible(), "the Aa panel's Link button opens Add Link");
const r = await box.boundingBox();
ok(r && r.x >= 0 && r.y >= 0 && r.x + r.width <= 1200 && r.y + r.height <= 800, "on screen");
await page.keyboard.press("Escape");

// A link just copied is offered, selected: Return takes it.
await page.evaluate(() => { window.__clipText = "https://www.apple.com/icloud/"; });
const clock = page.locator(".note-rich .nf-p", { hasText: /^wall clock/ }).first();
await clock.click();
await page.keyboard.press("End");
await page.keyboard.press("Shift+Home");
await page.keyboard.press("Control+k");
await page.waitForTimeout(250);
const field = box.getByLabel("Link to");
ok((await field.inputValue()) === "https://www.apple.com/icloud/", "a link on the clipboard is offered");
ok(await field.evaluate((el) => el.selectionStart === 0 && el.selectionEnd === el.value.length), "selected, so typing replaces it");
await page.keyboard.press("Enter");
await page.waitForTimeout(150);
ok((await clock.locator('a[href="https://www.apple.com/icloud/"]').count()) === 1, "and Return links it");
await page.evaluate(() => { window.__clipText = "just some words"; });

// The box stays in the note: opened at the very start of a line, by the list, in a wide and a
// narrow window, it never crosses into the list of notes.
for (const [w, h] of [[1200, 800], [760, 560]]) {
  await page.setViewportSize({ width: w, height: h });
  await page.waitForTimeout(300);
  const line = page.locator(".note-rich .nf-p", { hasText: /^Knife/ }).first();
  await line.scrollIntoViewIfNeeded();
  await line.click();
  await page.keyboard.press("Home");
  await page.keyboard.press("Control+k");
  await page.waitForTimeout(250);
  const pane = await page.locator(".note-editor").boundingBox();
  const at = await box.boundingBox();
  ok(at && pane && at.x >= pane.x && at.x + at.width <= pane.x + pane.width + 1, `${w}x${h}: inside the note, clear of the list  (box ${Math.round(at?.x)}–${Math.round((at?.x ?? 0) + (at?.width ?? 0))}, note ${Math.round(pane?.x)}–${Math.round((pane?.x ?? 0) + (pane?.width ?? 0))})`);
  ok(at && at.y >= 0 && at.y + at.height <= h, `${w}x${h}: wholly on screen`);
  ok((await field.inputValue()) === "", `${w}x${h}: words on the clipboard are not offered as a link`);
  await page.keyboard.press("Escape");
  await page.waitForTimeout(100);
}
await page.setViewportSize({ width: 1200, height: 800 });

// In the shortcuts sheet.
await page.waitForTimeout(150);
await page.keyboard.press("Control+/");
await page.waitForTimeout(300);
ok(await page.getByRole("dialog", { name: "Keyboard Shortcuts" }).getByText("Add or edit a link").isVisible(), "listed in Keyboard Shortcuts");

await b.close();
process.exitCode = failures ? 1 : 0;
