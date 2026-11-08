// Keep on This Computer, as Finder's Keep Downloaded: in a file's menu only while the iCloud
// Drive folder is up (that is where a kept file lives); a check beside it when kept; chosen,
// it is kept at once, and chosen again let go of.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
for (const on of [false, true]) {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
  await ctx.addInitScript((on) => { window.__driveFolder = { on, path: "/home/test/iCloud Drive", problem: null }; }, on);
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, driveFixture: true });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
  await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
  await openScreen(page, "Drive");
  await page.waitForTimeout(500);
  const row = page.locator(".drive-row-hit").filter({ hasNot: page.locator("[data-kind='FOLDER']") }).last();
  const name = (await row.innerText()).split("\n")[0];
  await row.click({ button: "right" });
  const item = page.getByRole("menuitemcheckbox", { name: "Keep on This Computer" }).or(page.getByRole("menuitem", { name: "Keep on This Computer" }));
  if (!on) {
    ok((await item.count()) === 0, "with the Drive folder off, no Keep on This Computer (there is nowhere to keep it)");
    await page.keyboard.press("Escape");
  } else {
    ok((await item.count()) === 1, `with it on, "${name}" can be kept`);
    ok((await item.getAttribute("aria-checked")) !== "true", "not kept to begin with");
    await item.click();
    await page.waitForTimeout(150);
    ok((await page.evaluate(() => window.__kept?.length)) === 1, "chosen, it is kept");
    await row.click({ button: "right" });
    ok((await item.getAttribute("aria-checked")) === "true", "and the menu checks it");
    await item.click();
    await page.waitForTimeout(150);
    ok((await page.evaluate(() => window.__kept?.length)) === 0, "chosen again, it is let go of");
  }
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
