// The folder in front follows Drive's change feed: told by the backend that it changed
// (quince://drive-changed), it redraws from the library with no request of its own; told of
// other folders, or nothing, it does nothing — no timed checks — and it stands aside while a
// change of this computer's own is on its way. Coming back to the window still looks once.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
  calendarFixture: false, notesFixture: false, contactsFixture: false, driveFixture: true, remindersFixture: false });
await ctx.addInitScript(() => {
  const internals = window.__TAURI_INTERNALS__;
  const invoke = internals.invoke;
  internals.invoke = async (cmd, args) => {
    if (cmd === "drive_sync_folder") window.__checks = (window.__checks ?? 0) + 1;
    if (cmd === "drive_rename") await new Promise((r) => setTimeout(r, 12_000));
    const answer = await invoke(cmd, args);
    if (cmd === "drive_children" && window.__phone && Array.isArray(answer)) {
      return [...answer, { drivewsid: "FILE::phone", parentId: null, name: "Scan from phone", extension: "pdf",
        kind: "FILE", size: 2048, childCount: null, createdAt: 1792500000, modifiedAt: 1792500000, etag: "p1" }];
    }
    return answer;
  };
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Drive");
await page.waitForTimeout(600);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const names = () => page.locator(".drive-name").allInnerTexts();

const told = (folders) => page.evaluate((f) => window.__emit("quince://drive-changed", f), folders);
const reads = () => page.evaluate(() => window.__reads ?? 0);
await page.evaluate(() => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  window.__TAURI_INTERNALS__.invoke = (cmd, args) => {
    if (cmd === "drive_children") window.__reads = (window.__reads ?? 0) + 1;
    return invoke(cmd, args);
  };
});
const ROOT = "FOLDER::com.apple.CloudDocs::root";

// No timed checks: a folder nobody changed is never asked about again.
await page.evaluate(() => { window.__firstRow = document.querySelector(".drive-name"); });
const before = await page.evaluate(() => window.__checks);
await page.waitForTimeout(10_600);
ok((await page.evaluate(() => window.__checks)) === before, "no timed check of the open folder");

// Another folder changed: nothing here moves.
const r0 = await reads();
await told(["FOLDER::com.apple.CloudDocs::elsewhere"]);
await page.waitForTimeout(300);
ok((await reads()) === r0, "a change in another folder reads nothing");

// Saved on the phone: told, read from the library, and drawn — without asking Apple.
await page.evaluate(() => { window.__phone = true; });
await told([ROOT]);
await page.waitForFunction(() => [...document.querySelectorAll(".drive-name")].some((e) => e.textContent.startsWith("Scan from phone")), null, { timeout: 3000 }).catch(() => {});
ok((await names()).some((n) => n.startsWith("Scan from phone")), "a file saved elsewhere appears when the backend says so");
ok((await page.evaluate(() => window.__checks)) === before, "and the folder was not asked of Apple again");
ok(await page.evaluate(() => document.querySelector(".drive-name") === window.__firstRow), "and the rows already there stay the same elements");

// A rename on its way (Apple slow to answer): a change told meanwhile stands aside.
await page.evaluate(() => { window.__phone = false; });
await page.locator(".drive-row-hit", { hasText: "Downloads" }).click({ button: "right" });
await page.getByRole("menuitem", { name: "Rename" }).click();
await page.keyboard.press("Control+a");
await page.keyboard.type("Downloads 2");
await page.keyboard.press("Enter");
await page.waitForTimeout(200);
const during = await reads();
await told([ROOT]);
await page.waitForTimeout(500);
ok((await reads()) === during, "nothing read over a rename on its way");
ok((await names()).some((n) => n.startsWith("Downloads 2")), "and the new name stays on screen meanwhile");

// Coming back to the window looks once, for what the feed does not cover.
await page.waitForTimeout(12_500); // the rename's answer lands
const away = await page.evaluate(() => window.__checks);
await page.evaluate(() => window.dispatchEvent(new Event("focus")));
await page.waitForTimeout(300);
ok((await page.evaluate(() => window.__checks)) > away, "and one look at once when the window comes to the front");

await b.close();
if (failures) process.exitCode = 1;
