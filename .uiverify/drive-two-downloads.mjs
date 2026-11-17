// Two files downloading at once each show they are downloading until their own download ends:
// the second took the first's spinner, and the first finishing cleared the second's.
//
//   URL=http://localhost:4203 ENGINE=webkit node .uiverify/drive-two-downloads.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, driveFixture: true });
await ctx.addInitScript(() => {
  const wrap = () => {
    const t = window.__TAURI_INTERNALS__;
    if (!t || t.__dl) return;
    const inner = t.invoke;
    window.__finish = {};
    t.invoke = async (cmd, args) => {
      if (cmd === "drive_children") return [
        { drivewsid: "FILE::com.apple.CloudDocs::A", parentId: null, name: "Alpha", extension: "pdf", kind: "FILE", size: 10, childCount: null, createdAt: 1700000000, modifiedAt: 1700000000, etag: null },
        { drivewsid: "FILE::com.apple.CloudDocs::B", parentId: null, name: "Beta", extension: "pdf", kind: "FILE", size: 10, childCount: null, createdAt: 1700000000, modifiedAt: 1700000000, etag: null },
      ];
      if (cmd === "drive_download") return new Promise((r) => { window.__finish[args.drivewsid] = () => r(`/home/test/Downloads/${args.drivewsid}.pdf`); });
      return inner(cmd, args);
    };
    t.__dl = true;
  };
  wrap(); document.addEventListener("DOMContentLoaded", wrap);
});
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Drive");
await page.waitForTimeout(400);
const busy = () => page.locator(".drive-row.is-busy .drive-name").allInnerTexts();
await page.locator(".drive-row-hit", { hasText: "Alpha" }).dblclick();
await page.locator(".drive-row-hit", { hasText: "Beta" }).dblclick();
await page.waitForTimeout(200);
const keys = await page.evaluate(() => Object.keys(window.__finish));
ok(keys.length === 2, `both are downloading  ${JSON.stringify(keys)}`);
ok(JSON.stringify((await busy()).sort()) === JSON.stringify(["Alpha.pdf", "Beta.pdf"]), `and both say so  ${JSON.stringify(await busy())}`);
await page.evaluate(() => window.__finish["FILE::com.apple.CloudDocs::A"]());
await page.waitForTimeout(200);
ok(JSON.stringify(await busy()) === JSON.stringify(["Beta.pdf"]), `the first done, the second still says so  ${JSON.stringify(await busy())}`);
await b.close();
console.log(failures ? `\n${failures} FAILED` : "\nall passed");
process.exit(failures ? 1 : 0);
