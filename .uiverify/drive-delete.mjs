// Drive delete, as Finder: no question (it goes to Recently Deleted), gone at once though
// Apple takes 1.5 s, and back with the reason if Apple refuses.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
for (const refuse of [false, true]) {
  const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, driveFixture: true });
  await ctx.addInitScript((refuse) => {
    const wrap = () => {
      const t = window.__TAURI_INTERNALS__;
      if (!t || t.__slow) return;
      const inner = t.invoke;
      t.invoke = async (cmd, args) => {
        if (cmd === "drive_delete") {
          await new Promise((r) => setTimeout(r, 1500));
          if (refuse) throw "iCloud didn't accept that change, so nothing was saved.";
        }
        return inner(cmd, args);
      };
      t.__slow = true;
    };
    wrap(); document.addEventListener("DOMContentLoaded", wrap);
  }, refuse);
  const page = await ctx.newPage();
  let asked = false;
  page.on("dialog", (d) => { asked = true; d.accept(); });
  await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await openScreen(page, "Drive");
  await page.waitForTimeout(300);
  await page.getByRole("button", { name: "Actions for Semester Plan.pdf" }).click();
  await page.getByRole("menuitem", { name: "Delete" }).click();
  await page.waitForTimeout(150);
  const row = () => page.locator(".drive-rows .drive-name", { hasText: "Semester Plan" }).count();
  if (!refuse) {
    ok(!asked, "Delete asks nothing: it goes to Recently Deleted");
    ok((await row()) === 0, "the file leaves at once");
  } else {
    await page.waitForTimeout(1700);
    ok((await row()) === 1 && (await page.locator('[role="alert"]').count()) === 1, "a refused delete brings the file back, with the reason");
  }
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
