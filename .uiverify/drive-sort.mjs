// Drive's "by Name" menu: Name as Finder orders it ("File 2" before "File 10"), Date and
// Size newest and largest first, folders kept at the top; remembered across visits.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, driveFixture: true });
await ctx.addInitScript(() => {
  const wrap = () => {
    const t = window.__TAURI_INTERNALS__;
    if (!t || t.__more) return;
    const inner = t.invoke;
    const file = (id, name, size, at) => ({ drivewsid: `FILE::${id}`, parentId: null, name, extension: "txt",
      kind: "FILE", size, childCount: null, createdAt: at, modifiedAt: at, etag: null });
    t.invoke = async (cmd, args) => {
      const out = await inner(cmd, args);
      if (cmd === "drive_children" && Array.isArray(out))
        return [file("z10", "File 10", 10, 1789000000), ...out, file("z2", "File 2", 5000000, 1700000000)];
      return out;
    };
    t.__more = true;
  };
  wrap(); document.addEventListener("DOMContentLoaded", wrap);
});
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Drive");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const names = () => page.locator(".drive-rows .drive-name").allInnerTexts();
const files = async () => (await names()).filter((n) => !["Downloads", "Keynote", "Shortcuts"].includes(n));
const pick = async (label) => {
  await page.getByRole("button", { name: /Sort by/ }).click();
  await page.getByRole("option", { name: `by ${label}` }).click();
  await page.waitForTimeout(200);
};

const byName = await files();
ok(JSON.stringify(byName) === JSON.stringify(["File 2.txt", "File 10.txt", "Semester Plan.pdf"]), `by Name, as Finder counts  ${JSON.stringify(byName)}`);
ok((await names())[0] === "Downloads", "folders stay at the top");
await pick("Size");
const bySize = await files();
ok(JSON.stringify(bySize) === JSON.stringify(["File 2.txt", "Semester Plan.pdf", "File 10.txt"]), `by Size, largest first  ${JSON.stringify(bySize)}`);
// The column heading is the same choice as the menu.
await pick("Name");
await page.locator(".drive-head").getByRole("button", { name: "Size" }).click();
await page.waitForTimeout(150);
ok(JSON.stringify(await files()) === JSON.stringify(bySize), "a click on the Size heading sorts by size");
await pick("Date");
const byDate = await files();
ok(JSON.stringify(byDate) === JSON.stringify(["File 10.txt", "Semester Plan.pdf", "File 2.txt"]), `by Date, newest first  ${JSON.stringify(byDate)}`);
await page.reload({ waitUntil: "networkidle" });
await openScreen(page, "Drive");
await page.waitForTimeout(400);
ok(JSON.stringify(await files()) === JSON.stringify(byDate), "the order chosen is remembered");
// View as Grid: tiles side by side, remembered.
await page.getByRole("button", { name: /View as/ }).click();
await page.getByRole("option", { name: "Grid" }).click();
await page.waitForTimeout(200);
const tops = async () => page.evaluate(() => [...document.querySelectorAll(".drive-rows .drive-row")].map((r) => Math.round(r.getBoundingClientRect().top)));
ok(new Set((await tops()).slice(0, 3)).size === 1, "Grid sets items side by side");
ok((await page.locator(".drive-row", { hasText: "Semester Plan" }).locator("img.drive-thumb").count()) === 1,
  "a file Apple has a picture of shows it, as icloud.com's grid does");
// In Grid, a tile's menu is a menu — not the window's width — and Get Info sits mid-window.
await page.locator(".drive-row", { hasText: "Semester Plan" }).click({ button: "right" });
await page.waitForTimeout(150);
const menuBox = await page.locator(".row-menu").boundingBox();
ok(menuBox && menuBox.width < 320, `a tile's menu is menu-sized  [${Math.round(menuBox?.width ?? 0)}px]`);
await page.getByRole("menuitem", { name: "Get Info" }).click();
await page.waitForTimeout(250);
const sheet = await page.locator(".info-sheet").boundingBox();
const mid = sheet ? Math.abs(sheet.x + sheet.width / 2 - 600) : 999;
ok(mid < 30, `Get Info opens in the middle of the window  [${Math.round(mid)}px off]`);
await page.keyboard.press("Escape");
await page.reload({ waitUntil: "networkidle" });
await openScreen(page, "Drive");
await page.waitForTimeout(400);
ok(new Set((await tops()).slice(0, 3)).size === 1, "and Grid is remembered");
await b.close();
process.exitCode = failures ? 1 : 0;
