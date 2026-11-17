// A double-click on an event keeps its popover open and still: the second click used to
// close it and the release open it again — a flicker, every time.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1300, height: 850 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Calendar");
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
for (const [key, sel, name] of [["Control+2", ".cx-block", "Week"], ["Control+3", ".cx-item:not(.is-reminder)", "Month"]]) {
  await page.keyboard.press(key);
  await page.waitForTimeout(400);
  await page.evaluate(() => {
    window.__popChanges = 0;
    window.__watch?.disconnect();
    window.__watch = new MutationObserver((m) => {
      for (const r of m) for (const n of [...r.addedNodes, ...r.removedNodes]) if (n.nodeType === 1 && (n.matches?.(".cx-popover") || n.querySelector?.(".cx-popover"))) window.__popChanges++;
    });
    window.__watch.observe(document.body, { childList: true, subtree: true });
  });
  await page.locator(sel, { hasText: "Design review" }).first().dblclick();
  await page.waitForTimeout(300);
  const changes = await page.evaluate(() => window.__popChanges);
  ok(changes === 1 && (await page.locator(".cx-popover").count()) === 1, `${name}: a double-click opens the popover once, with no flicker  [${changes} change(s)]`);
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);
}
await b.close();
process.exitCode = failures ? 1 : 0;
