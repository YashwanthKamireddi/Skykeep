// Drive's Home and End with nothing selected yet go to the first and the last row, as Finder's
// do (Home used to select the last row, End the first); and by Kind, folders come first even
// beside a file with no extension at all.
//
//   URL=http://localhost:4203 ENGINE=webkit node .uiverify/drive-ends.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const URL = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, driveFixture: true });
await ctx.addInitScript(() => {
  const wrap = () => {
    const t = window.__TAURI_INTERNALS__;
    if (!t || t.__more) return;
    const inner = t.invoke;
    t.invoke = async (cmd, args) => {
      const out = await inner(cmd, args);
      if (cmd === "drive_children" && Array.isArray(out))
        return [...out, { drivewsid: "FILE::makefile", parentId: null, name: "Makefile", extension: null, kind: "FILE",
          size: 120, childCount: null, createdAt: 1700000000, modifiedAt: 1700000000, etag: null }];
      return out;
    };
    t.__more = true;
  };
  wrap(); document.addEventListener("DOMContentLoaded", wrap);
});
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(URL, { waitUntil: "networkidle" });
await openScreen(page, "Drive");
await page.waitForTimeout(400);
const names = () => page.locator("button.drive-row-hit .drive-name").allInnerTexts();
const chosen = () => page.locator('.drive-row-hit[aria-pressed="true"] .drive-name').allInnerTexts();
// Into the folder's blank space below the rows, as a click there: nothing selected.
const blank = async () => { const r = await page.locator(".drive-rows").boundingBox(); await page.mouse.click(r.x + r.width / 2, 790); };
await blank();
const all = await names();
ok((await chosen()).length === 0, "nothing is selected when the folder opens");
await page.keyboard.press("Home");
ok(JSON.stringify(await chosen()) === JSON.stringify([all[0]]), `Home selects the first row  ${JSON.stringify(await chosen())} of ${JSON.stringify(all)}`);
await page.keyboard.press("End");
ok(JSON.stringify(await chosen()) === JSON.stringify([all.at(-1)]), `End selects the last  ${JSON.stringify(await chosen())}`);
await page.reload({ waitUntil: "networkidle" });
await openScreen(page, "Drive");
await page.waitForTimeout(400);
await blank();
await page.keyboard.press("End");
ok(JSON.stringify(await chosen()) === JSON.stringify([(await names()).at(-1)]), `End with nothing selected selects the last row  ${JSON.stringify(await chosen())}`);

await page.getByRole("button", { name: /Sort by/ }).click();
await page.getByRole("option", { name: "by Kind" }).click();
await page.waitForTimeout(200);
const byKind = await names();
const firstFile = byKind.findIndex((n) => n.includes(".") || n === "Makefile");
const lastFolder = Math.max(...["Downloads", "Keynote", "Shortcuts"].map((f) => byKind.indexOf(f)));
ok(firstFile > lastFolder, `by Kind, the folders come before every file, Makefile too  ${JSON.stringify(byKind)}`);
await page.getByRole("button", { name: /Sort by/ }).click();
await page.getByRole("option", { name: "by Name" }).click();

await b.close();
console.log(failures ? `\n${failures} FAILED` : "\nall passed");
process.exit(failures ? 1 : 0);
