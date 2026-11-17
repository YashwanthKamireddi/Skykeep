// A section shows what is true from its first frame: the Backup switches never show on and then
// flip off as the answer arrives (his recording 20.40.28), on the first visit or any after.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openSettings, openPane } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1180, height: 820 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
// Every frame's switches in Backup, from the moment the section is shown.
await ctx.addInitScript(() => {
  window.__seen = [];
  const look = () => {
    const pane = document.querySelector("#pane-backup:not([hidden])");
    if (pane) window.__seen.push([...pane.querySelectorAll('[role="switch"]')].map((s) => s.getAttribute("aria-checked")).join(","));
    requestAnimationFrame(look);
  };
  requestAnimationFrame(look);
});
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openSettings(page);
await page.waitForTimeout(600);
for (const visit of [1, 2, 3]) {
  await page.evaluate(() => { window.__seen = []; });
  await openPane(page, "Backup");
  await page.waitForTimeout(500);
  const seen = [...new Set(await page.evaluate(() => window.__seen))];
  ok(seen.length === 1, `visit ${visit}: the switches never change after the section shows  ${JSON.stringify(seen)}`);
  await openPane(page, "General");
}
await b.close();
process.exitCode = failures ? 1 : 0;
