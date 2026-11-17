// Reminders' keys after Photos has been opened. Photos keeps its grid — a listbox — mounted
// once visited, and Reminders took any listbox on the page for an open menu, so Ctrl+N did
// nothing until the app restarted. It now asks overlayOpen(), which knows a screen's own list.
//
//   BASE=http://localhost:4173 ENGINE=webkit node .uiverify/reminders-keys-after-photos.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };

const ctx = await b.newContext({ viewport: { width: 1280, height: 820 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(BASE, { waitUntil: "networkidle" });

await openScreen(page, "Photos");
await page.waitForTimeout(500);
ok((await page.locator('[role="listbox"]').count()) > 0, "Photos' grid is on the page");
await openScreen(page, "Reminders");
await page.waitForTimeout(400);
await page.locator(".rx-row").first().click();
await page.keyboard.press("Escape");
await page.keyboard.press("Control+n");
await page.waitForTimeout(300);
const focused = await page.evaluate(() => !!document.activeElement?.closest(".rx-add, .rx-row"));
ok(focused, "Ctrl+N still starts a new reminder after Photos was opened");

await b.close();
process.exitCode = failures ? 1 : 0;
