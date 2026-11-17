// The keys that go anywhere wait while a sheet is up: Ctrl+K took the focus out of an open
// sheet into the search field behind its veil, and Ctrl+, changed the screen under it.
//
//   URL=http://localhost:4203 ENGINE=webkit node .uiverify/app-keys-modal.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1280, height: 820 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.waitForTimeout(300);
await page.keyboard.press("Control+/");
const sheet = page.getByRole("dialog", { name: "Keyboard Shortcuts" });
await sheet.waitFor();
await page.keyboard.press("Control+k");
await page.waitForTimeout(150);
ok(await page.evaluate(() => Boolean(document.activeElement?.closest('[aria-modal="true"]'))), "Ctrl+K leaves the focus in the open sheet");
await page.keyboard.press("Control+,");
await page.waitForTimeout(200);
// Settings is a window of its own (apps.rs): the stub records settings_open in __windows.
const settingsAsked = () => page.evaluate(() => (window.__windows ?? []).filter((w) => w.cmd === "settings_open").length);
ok((await settingsAsked()) === 0, "Ctrl+, does not open Settings while the sheet is up");
await page.keyboard.press("Escape");
await page.waitForTimeout(150);
await page.keyboard.press("Control+,");
await page.waitForTimeout(200);
ok((await settingsAsked()) === 1, "with the sheet put away, Ctrl+, opens Settings");
await b.close();
console.log(failures ? `\n${failures} FAILED` : "\nall passed");
process.exit(failures ? 1 : 0);
