// In the Flatpak: Open at Login is not a switch that says "on" over an entry the desktop never
// reads — the row says how to add Skykeep to the desktop's startup apps instead. Outside it the
// switch is there as before.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openSettings } from "./stub.mjs";
const BASE = process.env.BASE ?? process.env.URL ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
for (const flatpak of [true, false]) {
  const ctx = await b.newContext({ viewport: { width: 1200, height: 900 } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
  if (flatpak) await ctx.addInitScript(() => { window.__login = { available: false, command: "flatpak run io.github.yashwanthkamireddi.Skykeep" }; });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); failures++; });
  await page.goto(BASE, { waitUntil: "networkidle" });
  await openSettings(page);
  await page.waitForSelector(".settings");
  await page.waitForTimeout(300);
  const sw = page.getByRole("switch", { name: "Open at Login" });
  const row = page.locator(".settings-row", { hasText: "Open at Login" });
  if (flatpak) {
    ok((await sw.count()) === 0, "Flatpak: no switch that cannot do what it says");
    ok(/startup apps/.test(await row.innerText()), `and the row says how  [${(await row.innerText()).replace(/\n/g, " · ")}]`);
  } else {
    ok((await sw.count()) === 1, "elsewhere: the switch is there");
  }
  await ctx.close();
}
await b.close();
console.log(failures ? `\n${failures} FAILED` : "\nall passed");
process.exit(failures ? 1 : 0);
