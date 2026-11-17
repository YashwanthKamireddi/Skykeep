// The folders' column shows whole names, as on the Mac: the first time it is as wide as the
// longest name needs (within reason); its edge drags it wider or narrower and is kept; a
// double-click on the edge fits the names; a name cut short shows whole on hover.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1400, height: 800 } });
await ctx.addInitScript(() => {
  window.__noteFolders = [
    { guid: "f1", title: "Notes", sortOrder: 0 },
    { guid: "f3", title: "Notes Features Testing", sortOrder: 1 },
    { guid: "f2", title: "Recently Deleted", sortOrder: 2 },
  ];
});
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const name = page.locator(".notes-folder-name", { hasText: "Notes Features Testing" });
const whole = () => name.evaluate((e) => e.scrollWidth <= e.clientWidth);
const width = () => page.locator(".notes-folders").evaluate((e) => e.getBoundingClientRect().width);
ok(await whole(), `the longest name shows whole the first time  (${await width()} px)`);
ok((await name.getAttribute("title")) === "Notes Features Testing", "and its whole name on hover");
// Dragged narrower: cut short, with an ellipsis, and kept.
const edge = page.getByRole("separator", { name: "Folder column width" });
const e = await edge.boundingBox();
await page.mouse.move(e.x + e.width / 2, e.y + 200);
await page.mouse.down();
await page.mouse.move(e.x - 60, e.y + 200, { steps: 6 });
await page.mouse.up();
await page.waitForTimeout(100);
const narrow = await width();
ok(narrow < e.x - 20 && !(await whole()), `dragged narrower (${narrow} px), the name is cut short`);
await page.reload({ waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.waitForTimeout(400);
ok(Math.abs((await width()) - narrow) < 2, "the width is kept");
await page.getByRole("separator", { name: "Folder column width" }).dblclick();
await page.waitForTimeout(100);
ok(await whole(), `a double-click on the edge fits the names  (${await width()} px)`);
await b.close();
process.exitCode = failures ? 1 : 0;
