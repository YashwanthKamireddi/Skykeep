// Notes' Sort Checked Items, as Apple has it on the iPhone and the Mac: under a Notes heading,
// Manually or Automatically, Manually until chosen; the choice is what Notes reads.
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openSettings, openPane } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1180, height: 820 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openSettings(page);
await openPane(page, "General");
const pane = page.locator(".prefs-pane:not([hidden])");
ok(await pane.getByRole("heading", { name: "Notes" }).isVisible(), "a Notes heading says whose setting it is");
const choice = pane.getByRole("group", { name: "Sort Checked Items" });
const manually = choice.getByRole("button", { name: "Manually" });
const automatically = choice.getByRole("button", { name: "Automatically" });
ok((await manually.getAttribute("aria-pressed")) === "true", "Manually until chosen");
await automatically.click();
ok((await automatically.getAttribute("aria-pressed")) === "true" && (await manually.getAttribute("aria-pressed")) === "false", "Automatically, once chosen");
ok(await page.evaluate(() => localStorage.getItem("notes.moveChecked")) === "1", "and Notes reads it");
await manually.click();
ok(await page.evaluate(() => localStorage.getItem("notes.moveChecked")) === "0", "Manually again puts it back");
await b.close();
process.exitCode = failures ? 1 : 0;
