// Sort Checked Items Automatically: off, a tick only ticks; on, a ticked item goes to the end
// of its own checklist and an unticked one back above the ticked — one edit, the text saved
// whole, nothing else in the note touched.
//
//   BASE=http://localhost:4173 ENGINE=webkit node .uiverify/notes-move-checked.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };

async function noteWithList(on) {
  const ctx = await b.newContext({ viewport: { width: 1300, height: 850 } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
  await ctx.addInitScript((v) => { try { localStorage.setItem("notes.moveChecked", v); } catch {} }, on ? "1" : "0");
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(BASE, { waitUntil: "networkidle" });
  await openScreen(page, "Notes");
  await page.locator(".note-row", { hasText: "Curtains" }).click();
  await page.locator(".note-rich .nf-p", { hasText: "Washing Machine cover" }).waitFor();
  await page.locator(".note-rich .nf-p").last().click();
  await page.keyboard.press("End");
  await page.waitForTimeout(150);
  await page.keyboard.press("Enter");
  await page.keyboard.press("Enter");
  await page.keyboard.type("- ");
  await page.keyboard.type("[ ] apples");
  for (const item of ["bread", "cheese"]) { await page.keyboard.press("Enter"); await page.keyboard.type(item); }
  await page.waitForTimeout(1300);
  return { ctx, page };
}
// The three items this suite made (the note has a checklist line of its own, earlier on).
const items = (page) => page.locator('.note-rich .nf-p[data-style="103"]').evaluateAll((ps) =>
  ps.filter((p) => ["apples", "bread", "cheese"].includes(p.textContent)).map((p) => `${p.dataset.checked === "true" ? "x" : " "} ${p.textContent}`));
const tick = async (page, name) => {
  const line = page.locator('.note-rich .nf-p[data-style="103"]', { hasText: name });
  const box = await line.boundingBox();
  await page.mouse.click(box.x + 6, box.y + box.height / 2);
  await page.waitForTimeout(1300);
};
const lastSave = (page) => page.evaluate(() => window.__noteSaves?.at(-1)?.body ?? "");

// Off (the default): a tick only ticks.
{
  const { ctx, page } = await noteWithList(false);
  await tick(page, "apples");
  ok(JSON.stringify(await items(page)) === JSON.stringify(["x apples", "  bread", "  cheese"]), `off: ticked in place  ${JSON.stringify(await items(page))}`);
  await ctx.close();
}

// On: to the bottom, and back.
{
  const { ctx, page } = await noteWithList(true);
  const before = await lastSave(page);
  await tick(page, "apples");
  ok(JSON.stringify(await items(page)) === JSON.stringify(["  bread", "  cheese", "x apples"]), `on: a ticked item goes to the end of its list  ${JSON.stringify(await items(page))}`);
  const body = await lastSave(page);
  ok(body.endsWith("bread\ncheese\napples") && body.startsWith(before.slice(0, before.lastIndexOf("apples"))), "the saved text has it moved, the rest of the note as it was");
  await tick(page, "bread");
  ok(JSON.stringify(await items(page)) === JSON.stringify(["  cheese", "x apples", "x bread"]), `a second tick goes below the first  ${JSON.stringify(await items(page))}`);
  await tick(page, "apples");
  ok(JSON.stringify(await items(page)) === JSON.stringify(["  cheese", "  apples", "x bread"]), `unticked, it comes back above the ticked  ${JSON.stringify(await items(page))}`);
  ok(await page.locator(".note-rich .nf-p", { hasText: "Washing Machine cover" }).count() === 1, "the note's other lines are untouched");
  await page.keyboard.press("Control+z");
  await page.waitForTimeout(300);
  ok(JSON.stringify(await items(page)) === JSON.stringify(["  cheese", "x apples", "x bread"]), `one undo puts the tick and the move back together  ${JSON.stringify(await items(page))}`);
  await ctx.close();
}

await b.close();
process.exitCode = failures ? 1 : 0;
