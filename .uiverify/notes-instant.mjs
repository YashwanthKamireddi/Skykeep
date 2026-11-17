// Notes writes show at once: a pin, a delete. The stub holds each for 1.5 s.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
await ctx.addInitScript(() => {
  const wrap = () => {
    const t = window.__TAURI_INTERNALS__;
    if (!t || t.__slow) return;
    const inner = t.invoke;
    t.invoke = async (cmd, args) => {
      if (/^note_(pin|delete|move)$/.test(cmd)) await new Promise((r) => setTimeout(r, 1500));
      return inner(cmd, args);
    };
    t.__slow = true;
  };
  wrap(); document.addEventListener("DOMContentLoaded", wrap);
});
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.locator(".note-row", { hasText: "Curtains" }).click();
await page.waitForTimeout(300);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

await page.getByRole("button", { name: /^(Pin|Unpin) Note$/ }).first().click();
await page.waitForTimeout(150);
const pressed = await page.getByRole("button", { name: /^(Pin|Unpin) Note$/ }).first().getAttribute("aria-pressed");
ok(pressed === "true", "a pin shows at once");

const before = await page.locator(".note-row").count();
await page.getByRole("button", { name: "Delete", exact: true }).click();
await page.waitForTimeout(150);
ok((await page.locator(".note-row", { hasText: "Curtains" }).count()) === 0 && (await page.locator(".note-row").count()) === before - 1,
  "a deleted note leaves the list at once");
await b.close();
process.exitCode = failures ? 1 : 0;
