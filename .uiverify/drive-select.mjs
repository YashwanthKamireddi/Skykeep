// Finder's selection and keys in Drive: Ctrl-click adds one, Shift-click a run, Ctrl-A all,
// a click on nothing selects nothing; the menu and Delete act on all of the selection; F2
// renames with the name (not its extension) selected; Space shows Quick Look; a heading
// clicked twice turns the order round.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, driveFixture: true });
await ctx.addInitScript(() => {
  const wrap = () => {
    const t = window.__TAURI_INTERNALS__;
    if (!t || t.__sel) return;
    const inner = t.invoke;
    t.invoke = async (cmd, args) => {
      if (cmd === "drive_delete") (window.__driveDeletes ??= []).push(args.drivewsid);
      return inner(cmd, args);
    };
    t.__sel = true;
  };
  wrap(); document.addEventListener("DOMContentLoaded", wrap);
});
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Drive");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const chosen = () => page.locator('.drive-row-hit[aria-pressed="true"] .drive-name').allInnerTexts();
const row = (name) => page.locator(".drive-row-hit", { hasText: name });

await row("Downloads").click();
await row("Semester Plan").click({ modifiers: ["Control"] });
ok(JSON.stringify(await chosen()) === JSON.stringify(["Downloads", "Semester Plan.pdf"]), `Ctrl-click adds one  ${JSON.stringify(await chosen())}`);
await row("Downloads").click({ modifiers: ["Control"] });
ok(JSON.stringify(await chosen()) === JSON.stringify(["Semester Plan.pdf"]), "and takes one away");
await row("Keynote").click();
await row("Semester Plan").click({ modifiers: ["Shift"] });
ok((await chosen()).length === 3, `Shift-click takes the run  ${JSON.stringify(await chosen())}`);
await page.keyboard.press("Control+a");
ok((await chosen()).length === 4, "Ctrl-A takes everything");
await page.mouse.click(700, 700);
ok((await chosen()).length === 0, "a click on nothing selects nothing");

// The menu on a selection is for all of it.
await row("Downloads").click();
await row("Semester Plan").click({ modifiers: ["Control"] });
await row("Semester Plan").click({ button: "right" });
const offered = await page.getByRole("menuitem").allInnerTexts();
ok(offered.some((t) => t.includes("Delete 2 Items")) && offered.some((t) => t.includes("Move to Folder")), `the menu names how many  ${JSON.stringify(offered)}`);
await page.keyboard.press("Escape");
await page.waitForTimeout(100);
await row("Downloads").click();
await row("Semester Plan").click({ modifiers: ["Control"] });
await page.keyboard.press("Delete");
await page.waitForTimeout(300);
const deleted = await page.evaluate(() => window.__driveDeletes ?? []);
ok(deleted.length === 2 && deleted.includes("FOLDER::a") && deleted.includes("FILE::d"), `Delete sends all of the selection  ${JSON.stringify(deleted)}`);
await page.reload({ waitUntil: "networkidle" });
await openScreen(page, "Drive");
await page.waitForTimeout(400);

// F2: rename in place, the name without its extension selected.
await row("Semester Plan").click();
await page.keyboard.press("F2");
const field = page.locator(".drive-rename-input");
ok(await field.isVisible(), "F2 renames in place");
const range = await field.evaluate((f) => [f.selectionStart, f.selectionEnd, f.value]);
ok(range[0] === 0 && range[1] === "Semester Plan".length, `with the name, not ".pdf", selected  ${JSON.stringify(range)}`);
await page.keyboard.press("Escape");
ok(!(await field.count()), "Escape leaves the name as it was");

// Space: Quick Look.
await row("Semester Plan").click();
await page.keyboard.press(" ");
const peek = page.getByRole("dialog", { name: /Quick Look/ });
ok(await peek.isVisible(), "Space shows Quick Look");
ok((await peek.locator(".drive-peek-name").innerText()) === "Semester Plan.pdf", "of the selected file");
ok(await peek.locator("img").count() === 1, "with Apple's picture of it");
await page.keyboard.press("ArrowUp");
await page.waitForTimeout(100);
ok((await peek.locator(".drive-peek-name").innerText().catch(() => "")) === "Shortcuts", "the arrows walk on and the look follows");
await page.keyboard.press(" ");
ok(!(await peek.count()), "Space again closes it");
await row("Semester Plan").click();
await page.keyboard.press(" ");
await page.keyboard.press("Escape");
ok(!(await peek.count()), "so does Escape");

// A heading clicked twice turns the order round.
const names = async () => { await page.waitForTimeout(150); return page.locator(".drive-rows .drive-name").allTextContents(); };
const before = await names();
await page.locator(".drive-head").getByRole("button", { name: /^Name/ }).click();
const after = await names();
ok(JSON.stringify(after) === JSON.stringify(["Shortcuts", "Keynote", "Downloads", "Semester Plan.pdf"]), `Name clicked again: Z to A, folders still on top  ${JSON.stringify(after)}`);
ok((await page.locator(".drive-head .drive-col.is-on").innerText()).includes("⌃"), "and the heading says so");
await page.locator(".drive-head").getByRole("button", { name: /^Name/ }).click();
ok(JSON.stringify(await names()) === JSON.stringify(before), "a third click puts it back");
await page.reload({ waitUntil: "networkidle" });
await openScreen(page, "Drive");
await page.waitForTimeout(300);
ok(JSON.stringify(await names()) === JSON.stringify(before), "remembered the right way round");
await ctx.close();

// A folder of thousands: the arrows stay quick.
const big = await b.newContext({ viewport: { width: 1280, height: 800 } });
await big.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, driveFixture: true });
await big.addInitScript(() => {
  const wrap = () => {
    const t = window.__TAURI_INTERNALS__;
    if (!t || t.__many) return;
    const inner = t.invoke;
    const files = Array.from({ length: 4000 }, (_, i) => ({ drivewsid: `FILE::n${i}`, parentId: null, name: `Scan ${i}`,
      extension: "pdf", kind: "FILE", size: 1000 + i, childCount: null, createdAt: 1700000000 + i, modifiedAt: 1700000000 + i, etag: null }));
    t.invoke = async (cmd, args) => {
      const out = await inner(cmd, args);
      if (cmd === "drive_children" && Array.isArray(out)) return [...out, ...files];
      return out;
    };
    t.__many = true;
  };
  wrap(); document.addEventListener("DOMContentLoaded", wrap);
});
const p2 = await big.newPage();
await p2.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(p2, "Drive");
await p2.waitForTimeout(800);
await p2.locator("button.drive-row-hit").first().click();
const t0 = await p2.evaluate(() => performance.now());
for (let i = 0; i < 10; i++) await p2.keyboard.press("ArrowDown");
const per = ((await p2.evaluate(() => performance.now())) - t0) / 10;
ok((await p2.locator('.drive-row-hit[aria-pressed="true"]').count()) === 1, "the selection walked");
ok(per < 100, `walking 4000 files stays quick  ${per.toFixed(1)} ms a step`);
await b.close();
process.exitCode = failures ? 1 : 0;
