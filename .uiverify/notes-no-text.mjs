// A note the library holds no text for (not yet arrived, or one Skykeep can't read): said in
// words, never an "Opening…" that never ends; a note with its text opens as ever.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 820 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
  calendarFixture: false, notesFixture: true, contactsFixture: false, driveFixture: false, remindersFixture: false });
await ctx.addInitScript(() => { window.__noText = "n1"; });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.BASE ?? process.env.URL ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
await page.locator(".note-row", { hasText: "Curtains" }).click();
await page.waitForTimeout(900);
const said = await page.locator(".note-loading").innerText().catch(() => "");
ok(/isn't here yet/.test(said) && /safe in iCloud/.test(said), `a note with no text says so  ${JSON.stringify(said.slice(0, 60))}`);
ok((await page.locator(".note-loading", { hasText: "Opening" }).count()) === 0, "and never shows Opening…");
ok((await page.locator("[contenteditable='true'].note-text, textarea.note-text").count()) === 0, "nothing to type into, so nothing empty is written back");
const other = page.locator(".note-row").filter({ hasNotText: "Curtains" }).first();
await other.click();
await page.waitForTimeout(600);
ok((await page.locator(".note-loading").count()) === 0, "a note with its text opens as ever");
await b.close();
console.log(failures ? `${failures} FAILED` : "all passed");
process.exit(failures ? 1 : 0);
