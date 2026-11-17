// Whole photos, as icloud.com's toggle shows them: each in its own shape inside its cell, with
// the badges and the selection on the photo itself; the choice kept after a reload; squares again.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const shape = () => page.locator(".px-tile").nth(1).evaluate((t) => {
  const cell = t.getBoundingClientRect(), pic = t.querySelector(".px-pic").getBoundingClientRect();
  return { ratio: pic.height / pic.width, fills: Math.abs(pic.height - cell.height) < 1 };
});

ok((await shape()).fills, "squares fill each cell");
await page.getByRole("button", { name: "Show Whole Photos" }).click();
await page.waitForTimeout(150);
const s = await shape();
ok(Math.abs(s.ratio - 0.75) < 0.02 && !s.fills, `whole photos keep their own shape  (${s.ratio.toFixed(2)}, a 4:3 photo)`);
ok((await page.getByRole("button", { name: "Show Square Photos" }).getAttribute("aria-pressed")) === "true", "and the toggle shows it is on");
await page.locator(".px-tile").nth(1).click();
await page.waitForTimeout(250);
const frame = await page.locator(".px-tile").nth(1).locator(".px-pic").evaluate((p) => getComputedStyle(p, "::after").opacity);
ok(frame === "1", "the selection is drawn on the photo itself");
await page.reload({ waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(400);
ok(Math.abs((await shape()).ratio - 0.75) < 0.02, "whole photos are kept after a reload");
await page.getByRole("button", { name: "Show Square Photos" }).click();
await page.waitForTimeout(150);
ok((await shape()).fills, "and squares again at a click");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
