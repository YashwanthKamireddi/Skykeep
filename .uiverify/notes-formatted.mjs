// A note is edited formatted, as Notes edits it on icloud.com and the Mac: a phone's
// checklist and dashed list show as they are; the circle ticks; Aa sets a line's style and
// B/I/U/S the selection, with the current style ticked; the checklist button makes and
// unmakes items; Return carries a list on and ends it on an empty item; and every change
// goes to Apple as the text with its runs.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.locator(".note-row", { hasText: "Curtains" }).click();
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const line = (text) => page.locator(".note-rich .nf-p", { hasText: text }).first();
const lastSave = async () => { await page.waitForTimeout(1200); return page.evaluate(() => window.__noteSaves?.at(-1)); };
const runAt = (save, text) => {
  if (!save) return null;
  const at = save.body.indexOf(text);
  let n = 0;
  for (const r of save.runs ?? []) { if (at < n + r.length) return r; n += r.length; }
  return null;
};

ok((await page.locator('.note-rich .nf-p[data-style="103"][data-checked="true"]').count()) === 1, "a ticked checklist item shows ticked");
ok((await page.locator('.note-rich .nf-p[data-style="101"]').count()) >= 3, "a dashed list shows as one");

// The circle ticks and unticks, and the change is saved with the formatting.
const item = page.locator('.note-rich .nf-p[data-style="103"]').first();
const box = await item.boundingBox();
await page.mouse.click(box.x + 6, box.y + 10);
await page.waitForTimeout(100);
ok((await item.getAttribute("data-checked")) === null, "a click on the circle unticks it at once");
let save = await lastSave();
ok(save?.runs && runAt(save, "Bathroom")?.style === 103 && runAt(save, "Bathroom")?.checked === false, `and the untick is saved  ${JSON.stringify(runAt(save, "Bathroom"))}`);

// Aa → Heading on the line the cursor is in; the panel ticks the current style.
await line("Fan").click();
await page.getByRole("button", { name: "Format" }).click();
ok((await page.getByRole("menuitemradio", { checked: true }).innerText()).includes("Dashed List"), "Aa ticks the line's own style");
await page.getByRole("menuitemradio", { name: "Heading", exact: true }).click();
ok((await line("Fan").getAttribute("data-style")) === "1", "Aa → Heading sets the line at once");
ok(await page.evaluate(() => document.activeElement?.classList.contains("note-rich")), "and the cursor stays in the note");
save = await lastSave();
ok(runAt(save, "Fan")?.style === 1 && runAt(save, "wall clock")?.style === 101, `only that line, saved  ${JSON.stringify([runAt(save, "Fan"), runAt(save, "wall clock")?.style])}`);

// Bold on a selected word, from the panel and from the keyboard.
await line("wall clock").click();
await page.keyboard.press("End");
await page.keyboard.press("Shift+Home");
await page.getByRole("button", { name: "Format" }).click();
await page.getByRole("button", { name: "Bold" }).click();
ok((await line("wall clock").locator("strong").count()) === 1, "B makes the selection bold at once");
ok((await page.getByRole("button", { name: "Bold" }).getAttribute("aria-pressed")) === "true", "and the panel shows it on");
await page.keyboard.press("Escape");
await page.keyboard.press("Control+i");
ok((await line("wall clock").locator("em").count()) === 1, "Ctrl-I makes it italic");
save = await lastSave();
const clock = runAt(save, "clock");
ok(clock?.bold && clock?.italic, `and both are saved  ${JSON.stringify(clock)}`);

// The checklist button makes the line an item, and again makes it a plain line.
await line("Bed").click();
await page.getByRole("button", { name: "Make a checklist" }).click();
ok((await line("Bed").getAttribute("data-style")) === "103", "the checklist button makes an item");
await page.getByRole("button", { name: "Make a checklist" }).click();
ok((await line("Bed").getAttribute("data-style")) === "none", "and pressed again, a plain line");

// Return carries a list on, and Return on an empty item ends it.
await line("Knife").click();
await page.waitForTimeout(50); // a person's pause between a click and a key
await page.keyboard.press("End");
await page.keyboard.press("Enter");
await page.keyboard.type("Spoon");
ok((await line("Spoon").getAttribute("data-style")) === "101", "Return carries the list on");
await page.keyboard.press("Enter");
await page.keyboard.press("Enter");
ok((await page.locator('.note-rich .nf-p[data-style="none"]').count()) >= 2, "Return on an empty item ends the list");
save = await lastSave();
ok(save.body.includes("Spoon") && runAt(save, "Spoon")?.style === 101, "and the new item is saved in the list");
await b.close();
process.exitCode = failures ? 1 : process.exitCode ?? 0;
