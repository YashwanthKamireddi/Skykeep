// Deleting a folder that files are being uploaded into stops that upload first — and only that
// one: matched by name alone, it missed files sent into the folder itself and stopped a folder
// of the same name going up somewhere else.
//
//   URL=http://localhost:4203 ENGINE=webkit node .uiverify/drive-delete-uploading.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1280, height: 820 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, driveFixture: true });
await ctx.addInitScript(() => {
  const wrap = () => {
    const t = window.__TAURI_INTERNALS__;
    if (!t || t.__up) return;
    const inner = t.invoke;
    window.__cancels = [];
    t.invoke = async (cmd, args) => {
      if (cmd === "plugin:dialog|open" || cmd === "pick_open") return ["/home/test/Holiday"];
      if (cmd === "drive_children") return args.folderId === "FOLDER::com.apple.CloudDocs::DL" ? [] : [
        { drivewsid: "FOLDER::com.apple.CloudDocs::DL", parentId: null, name: "Downloads", extension: null, kind: "FOLDER",
          size: null, childCount: 0, createdAt: 1700000000, modifiedAt: 1700000000, etag: null }];
      if (cmd === "drive_upload_cancel") { window.__cancels.push(args); return null; }
      if (cmd === "drive_upload") {
        const say = (e) => window.__emit?.("quince://upload", { within: "", error: null, sent: 0, folder: false, ...e });
        const batch = args.batch;
        window.__batches = [...(window.__batches ?? []), { batch, into: args.folderId }];
        say({ id: `${batch}:dir:0`, batch, name: "Holiday", total: 0, state: "done", folder: true });
        say({ id: `${batch}:0`, batch, name: "beach.jpg", within: "Holiday", total: 1000, state: "sending" });
        await new Promise(() => {});
      }
      return inner(cmd, args);
    };
    t.__up = true;
  };
  wrap(); document.addEventListener("DOMContentLoaded", wrap);
});
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Drive");
await page.waitForTimeout(400);
// Into Downloads: a folder of files going up there.
await page.locator(".drive-row-hit", { hasText: "Downloads" }).dblclick();
await page.waitForTimeout(400);
await page.getByRole("button", { name: "Upload", exact: true }).click();
await page.getByRole("menuitem", { name: "Folder" }).click();
await page.waitForTimeout(400);
const batches = await page.evaluate(() => window.__batches ?? []);
ok(batches.length === 1, `an upload into Downloads is going  ${JSON.stringify(batches)}`);
// Back up, and Downloads itself deleted.
await page.keyboard.press("Control+ArrowUp");
await page.waitForTimeout(400);
await page.locator(".drive-row-hit", { hasText: "Downloads" }).click();
await page.keyboard.press("Delete");
await page.waitForTimeout(150);
const confirm = page.getByRole("button", { name: /^(Delete|Move to Recently Deleted)$/ });
if (await confirm.count()) await confirm.first().click();
await page.waitForTimeout(400);
const cancels = await page.evaluate(() => window.__cancels);
ok(cancels.some((c) => c.batch === batches[0]?.batch), `deleting Downloads stops the upload going into it  ${JSON.stringify(cancels)}`);
await b.close();
console.log(failures ? `\n${failures} FAILED` : "\nall passed");
process.exit(failures ? 1 : 0);
