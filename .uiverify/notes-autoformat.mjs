// Lists typed as on the phone: "- " starts a dashed list, "* " a bulleted one, "1. " a
// numbered one, and "[ ] " / "[x] " in a dashed line make a checklist item. The marker typed
// goes, and the line takes the style Apple stores.
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
await page.locator(".note-rich .nf-p", { hasText: "Washing Machine cover" }).waitFor();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const styleOf = (text) => page.locator(".note-rich .nf-p", { hasText: text }).last().evaluate((p) => [p.dataset.style, p.dataset.checked ?? null, p.textContent]);

// A fresh line after the note's last one, out of any list.
await page.locator(".note-rich .nf-p").last().click();
await page.keyboard.press("End");
// Chromium tells the editor of a moved cursor a moment later; a key sent at once used the old one.
await page.waitForTimeout(150);
// Out of the note's own list: Return, then Return on the empty item ends the list.
await page.keyboard.press("Enter");
await page.keyboard.press("Enter");
const plain = await page.evaluate(() => {
  const sel = getSelection().anchorNode;
  return (sel?.nodeType === 1 ? sel : sel?.parentElement)?.closest(".nf-p")?.dataset.style;
});
ok(plain === "none" || plain === "3", `starts on a plain line  ${plain}`);

await page.keyboard.type("- milk");
let [style, , text] = await styleOf("milk");
ok(style === "101" && text === "milk", `"- " makes a dashed list, the dash gone  ${style} ${JSON.stringify(text)}`);

await page.keyboard.press("Enter");
await page.keyboard.type("[x] eggs");
let checked;
[style, checked, text] = await styleOf("eggs");
ok(style === "103" && checked === "true" && text === "eggs", `"[x] " in a dashed line makes a ticked checklist item  ${style} ${checked} ${JSON.stringify(text)}`);

await page.keyboard.press("Enter");
await page.keyboard.press("Enter");
await page.keyboard.type("1. first");
[style, , text] = await styleOf("first");
ok(style === "102" && text === "first", `"1. " makes a numbered list  ${style} ${JSON.stringify(text)}`);

await page.keyboard.press("Enter");
await page.keyboard.press("Enter");
await page.keyboard.type("5. fifth");
await page.keyboard.press("Enter");
await page.keyboard.type("sixth");
const counters = await page.locator(".note-rich .nf-p", { hasText: /fifth|sixth/ }).evaluateAll((ps) => ps.map((p) => [p.dataset.style, p.style.counterSet || ""]));
ok(JSON.stringify(counters) === JSON.stringify([["102", "nf-num 5"], ["102", ""]]), `"5. " starts the list at 5; the next item counts on from it  ${JSON.stringify(counters)}`);
await page.waitForTimeout(1300);
const saved = await page.evaluate(() => window.__noteSaves?.at(-1)?.runs?.filter((r) => r.start));
ok(Array.isArray(saved) && saved.length === 1 && saved[0].start === 5, `and Apple is sent start 5, on that line only  ${JSON.stringify(saved)}`);

await page.keyboard.press("Enter");
await page.keyboard.press("Enter");
await page.keyboard.type("* dot");
[style, , text] = await styleOf("dot");
ok(style === "100" && text === "dot", `"* " makes a bulleted list  ${style} ${JSON.stringify(text)}`);

await page.keyboard.press("Enter");
await page.keyboard.press("Enter");
await page.keyboard.type("a - b");
[style, , text] = await styleOf("a - b");
ok(style !== "101" && text === "a - b", "a dash in the middle of a line is just a dash");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
