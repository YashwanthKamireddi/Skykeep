// Deleting a note, as in Notes: the note below opens in its place, and Delete is ready again at
// once; a second delete while the first is still on its way goes too.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const titles = () => page.locator(".note-row .note-row-title, .note-row strong").allInnerTexts();
const rows = page.locator(".note-row");
const openTitle = () => page.locator(".note-row.is-open, .note-row[aria-selected='true']").first().innerText();

await page.evaluate(() => { window.__slowDelete = 1500; });
const before = await rows.allInnerTexts();
await rows.nth(1).click();
await page.waitForTimeout(300);
const second = before[1].split("\n")[0];
const third = before[2].split("\n")[0];
await page.getByRole("button", { name: "Delete", exact: true }).click();
await page.waitForTimeout(150);
ok((await openTitle()).startsWith(third), `the note below opens in its place  ${second} → ${(await openTitle()).split("\n")[0]}`);
ok(await page.getByRole("button", { name: "Delete", exact: true }).isEnabled(), "and Delete is ready again at once");
await page.getByRole("button", { name: "Delete", exact: true }).click();
await page.waitForTimeout(2200);
const gone = await page.evaluate(() => window.__noteDeletes);
ok(gone?.length === 2 && gone[0] !== gone[1], `a second delete while the first is on its way goes too  ${JSON.stringify(gone)}`);
// The engine's own menu (Back, Reload, Inspect Element) never opens; a text field keeps its own.
const blocked = await page.evaluate(() => {
  const at = (el) => { const e = new MouseEvent("contextmenu", { bubbles: true, cancelable: true }); el.dispatchEvent(e); return e.defaultPrevented; };
  return { empty: at(document.querySelector(".note-rows") ?? document.body), text: at(document.querySelector(".note-rich")) };
});
ok(blocked.empty && !blocked.text, `the engine's menu is kept out, a text field keeps its own  ${JSON.stringify(blocked)}`);
await b.close();
process.exitCode = failures ? 1 : 0;
