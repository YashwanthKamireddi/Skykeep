// A phone edit made while a note is open here is kept: each save sends what the edit began
// from, and when Apple answers with both edits combined, the editor shows both.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.locator(".note-row", { hasText: "Curtains" }).click();
await page.waitForTimeout(300);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
// Edit at the end, as a person adds a line.
await page.locator(".note-rich .nf-p").last().click();
await page.waitForTimeout(50);
await page.keyboard.press("Control+End");
await page.waitForTimeout(50);
// Meanwhile the phone changed the title: Apple answers with both.
await page.evaluate(() => { window.__phoneEdit = (a) => a.body.replace(/^Curtains/, "Curtains (phone)"); });
await page.keyboard.type("\nnew line from Linux");
await page.waitForTimeout(1500);
const sent = await page.evaluate(() => window.__noteSaves?.at(-1));
ok(typeof sent?.base === "string" && sent.base.startsWith("Curtains\n"), "a save says what the edit began from");
const lines = await page.locator(".note-rich .nf-p").allTextContents();
const title = lines[0] ?? "";
const text = lines.join("\n");
ok(title.includes("(phone)") && text.endsWith("cover\nnew line from Linux"), `both edits are on screen  [${title} … ${text.slice(-24)}]`);
// The next save begins from what Apple confirmed, so it does not undo the phone's edit.
await page.evaluate(() => { window.__phoneEdit = null; });
await page.keyboard.type(" again");
await page.waitForTimeout(1500);
const again = await page.evaluate(() => window.__noteSaves?.at(-1));
ok(again?.base?.includes("(phone)") && again?.body?.includes("(phone)"), "the next save keeps the phone's edit");
// And the formatting moved with it: the phone's words did not unset the lines below.
ok((await page.locator('.note-rich .nf-p[data-style="101"]').count()) >= 3 && (await page.locator(".note-rich .nf-p").first().getAttribute("data-style")) === "0",
  "the note keeps its formatting through the merge");
await b.close();
process.exitCode = failures ? 1 : 0;
