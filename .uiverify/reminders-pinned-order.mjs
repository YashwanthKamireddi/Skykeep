// Pinned lists stand as tiles in the order they were pinned, oldest first, as on the phone,
// not in the lists' own order (his iPhone: To-Do, pinned first, before Daily; Skykeep had
// Daily first). Here Daily is first among the lists but pinned last.
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1180, height: 800 } });
await ctx.addInitScript(() => {
  window.__listPins = {
    l1: { pinned: true, pinnedAt: 1790609001389 }, // Daily, pinned later
    l3: { pinned: true, pinnedAt: 1781974802494 }, // Life Goals, pinned first
  };
});
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(`${process.env.URL ?? "http://localhost:4173"}/?app=reminders`, { waitUntil: "networkidle" });
await page.locator(".rx-tile").first().waitFor();
const names = await page.locator(".rx-tile .rx-tile-name").allInnerTexts();
const pinned = names.filter((n) => n === "Daily" || n === "Life Goals");
ok(JSON.stringify(pinned) === JSON.stringify(["Life Goals", "Daily"]), `the tiles stand in pin order  ${JSON.stringify(names)}`);
const rows = await page.locator(".rx-lists .rx-list-row").allInnerTexts();
ok(!rows.some((r) => /Daily|Life Goals/.test(r)), "a pinned list is a tile, not also a row");
await b.close();
process.exitCode = failures ? 1 : 0;
