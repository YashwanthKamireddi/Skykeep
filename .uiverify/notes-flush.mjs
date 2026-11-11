// What was typed a moment ago is written down before the window or Skykeep goes. Quit, sign-out
// and a close with no tray ask the window first (`quince://flush`, apps.rs flush_windows) and
// wait for its answer; the window saves the open note, makes a note being composed real, and
// only then answers. Leaving the window (blur, hidden) saves at once too, not a pause later.
// Before this, all of it waited for the 900 ms pause, and Quit `_exit`ed first (audit C1).
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const saves = () => page.evaluate(() => (window.__noteSaves ?? []).map((s) => s.body));

// 1. An open note, typed into, then the backend asks at once: answered with the words saved.
await page.locator(".note-row", { hasText: "Curtains" }).first().click();
await page.waitForTimeout(400);
await page.locator(".note-rich p").first().click();
await page.keyboard.press("End");
await page.evaluate(() => { window.__noteSaves = []; window.__flushed = []; });
await page.keyboard.type(" and rails");
await page.evaluate(() => window.__emit("quince://flush", 7));
await page.waitForTimeout(250);
let flushed = await page.evaluate(() => window.__flushed);
ok(flushed.length === 1 && flushed[0].id === 7, `the window answers the ask  ${JSON.stringify(flushed)}`);
ok(flushed[0]?.saves >= 1, `and answers only once the note is saved  ${JSON.stringify(flushed)}`);
ok((await saves()).some((t) => t.includes("Curtains and rails")), `what was typed is in the save  ${JSON.stringify(await saves())}`);

// 2. A new note being composed: made real, its text in it, before the answer.
await page.evaluate(() => { window.__noteSaves = []; window.__flushed = []; window.__noteCreates = 0; });
await page.getByRole("button", { name: "New note" }).click();
await page.waitForTimeout(150);
await page.keyboard.type("Pack the tent");
await page.evaluate(() => window.__emit("quince://flush", 8));
await page.waitForTimeout(400);
flushed = await page.evaluate(() => window.__flushed);
ok(flushed.length === 1 && flushed[0].creates === 1, `a note being composed is made before the answer  ${JSON.stringify(flushed)}`);
const made = await page.evaluate(() => [...document.querySelectorAll(".note-row-name")].map((x) => x.textContent));
ok(made.some((t) => t?.includes("Pack the tent")), `and is listed with its words  ${JSON.stringify(made)}`);

// 3. Leaving the window saves at once, not a pause later.
await page.locator(".note-rich p").first().click();
await page.keyboard.press("End");
await page.evaluate(() => { window.__noteSaves = []; });
await page.keyboard.type(" today");
await page.evaluate(() => window.dispatchEvent(new Event("blur")));
await page.waitForTimeout(200);
ok((await saves()).some((t) => t.includes("Pack the tent today")), `leaving the window saves at once  ${JSON.stringify(await saves())}`);

// 4. A new note with nothing typed stays being composed when the window loses the focus.
await page.evaluate(() => { window.__noteCreates = 0; });
await page.getByRole("button", { name: "New note" }).click();
await page.waitForTimeout(150);
await page.evaluate(() => window.dispatchEvent(new Event("blur")));
await page.evaluate(() => window.__emit("quince://flush", 9));
await page.waitForTimeout(300);
ok((await page.evaluate(() => window.__noteCreates)) === 0, "an empty new note is not made");
await page.keyboard.type("Still here");
await page.waitForTimeout(150);
const editing = await page.evaluate(() => document.querySelector(".note-editor .note-rich")?.textContent ?? "");
ok(editing.includes("Still here"), `and is still open to type in  ${JSON.stringify(editing)}`);

await b.close();
console.log(failures ? `\n${failures} failure(s)` : "\nall passed");
process.exit(failures ? 1 : 0);
