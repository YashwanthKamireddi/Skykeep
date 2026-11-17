// The phone's line tools in the Aa panel, as Notes has them: Move Line Up and Down take the
// line (and the cursor) past the next one; the colours set Notes' text colour on what is
// selected and None takes it off; Increase and Decrease Indent step the line in and out.
// Each saves to Apple as the phone writes it.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
for (const scheme of ["light", "dark"]) {
  const ctx = await b.newContext({ viewport: { width: 1200, height: 800 }, colorScheme: scheme });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
  await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await openScreen(page, "Notes");
  await page.locator(".note-row", { hasText: "Curtains" }).first().click();
  await page.waitForTimeout(500);
  const lines = () => page.locator(".note-rich .nf-p").allInnerTexts();
  const lastSave = async () => { await page.waitForTimeout(1200); return page.evaluate(() => window.__noteSaves?.at(-1)); };
  const runAt = (save, text) => {
    const at = save.body.indexOf(text);
    let n = 0;
    for (const r of save.runs ?? []) { if (at < n + r.length) return r; n += r.length; }
    return null;
  };
  const panel = page.locator(".format-panel");
  const tool = (name) => panel.getByRole("button", { name, exact: true });
  const open = async () => { if (!(await panel.isVisible())) await page.getByRole("button", { name: "Format", exact: true }).click(); await page.waitForTimeout(120); };
  const fan = page.locator(".note-rich .nf-p", { hasText: /^Fan$/ }).first();

  // Move Line Up, then Down.
  await fan.click();
  await page.keyboard.press("End");
  await open();
  const before = await lines();
  const at = before.indexOf("Fan");
  await tool("Move Line Up").click();
  await page.waitForTimeout(120);
  let now = await lines();
  ok(now[at - 1] === "Fan" && now[at] === before[at - 1], `${scheme}: Move Line Up takes the line above the one before it`);
  let save = await lastSave();
  ok(save && save.body.split("\n").indexOf("Fan") === at - 1, `${scheme}: and saves it there`);
  await tool("Move Line Down").click();
  await tool("Move Line Down").click();
  await page.waitForTimeout(120);
  now = await lines();
  ok(now[at + 1] === "Fan" && now[at] === before[at + 1], `${scheme}: Move Line Down takes it below the next, the cursor with it`);
  await tool("Move Line Up").click();
  await page.waitForTimeout(120);
  ok(JSON.stringify(await lines()) === JSON.stringify(before), `${scheme}: and back where it was`);

  // A colour on the selected word; the swatch shows it; None takes it off.
  // The cursor is on the line still: the whole of it selected from the keyboard.
  await page.keyboard.press("End");
  await page.keyboard.press("Shift+Home");
  await open();
  await panel.getByRole("radio", { name: "Pink" }).click();
  await page.waitForTimeout(120);
  ok(await fan.locator(".nf-em-2").count() === 1, `${scheme}: Pink colours the selected word`);
  ok((await panel.getByRole("radio", { name: "Pink" }).getAttribute("aria-checked")) === "true", `${scheme}: and the panel shows Pink chosen`);
  save = await lastSave();
  ok(save && runAt(save, "Fan")?.emphasis === 2, `${scheme}: saved as Notes' pink (2)`);
  await panel.getByRole("radio", { name: "No colour" }).click();
  await page.waitForTimeout(120);
  ok(await fan.locator(".nf-em-2").count() === 0, `${scheme}: None takes the colour off`);
  save = await lastSave();
  ok(save && !runAt(save, "Fan")?.emphasis, `${scheme}: and the save has none`);

  // Indent in and out.
  await tool("Increase Indent").click();
  save = await lastSave();
  ok(save && runAt(save, "Fan")?.indent === 1, `${scheme}: Increase Indent steps the line in`);
  await tool("Decrease Indent").click();
  save = await lastSave();
  ok(save && runAt(save, "Fan")?.indent === 0, `${scheme}: Decrease Indent steps it back`);
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
