// Opening a folder shows that folder — never the last one's rows while it loads.
//
// A folder not seen before kept the previous level's rows on screen through a slow fetch, so
// a click there opened a folder from the level just left: Drive "went to random folders".
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
  calendarFixture: false, notesFixture: false, contactsFixture: false, driveFixture: true, remindersFixture: false });
// Downloads has not been visited, and Apple takes its time answering for it.
await ctx.addInitScript(() => {
  const internals = window.__TAURI_INTERNALS__;
  const invoke = internals.invoke;
  internals.invoke = async (cmd, args) => {
    if (cmd === "drive_create_folder") { await new Promise((r) => setTimeout(r, 1200)); return null; }
    if (args?.folderId === "FOLDER::a") {
      if (cmd === "drive_children") return window.__downloadsArrived ? [
        { drivewsid: "FILE::x", parentId: "FOLDER::a", name: "Ticket", extension: "pdf", kind: "FILE",
          size: 1000, childCount: null, createdAt: 1, modifiedAt: 1, etag: null }] : [];
      if (cmd === "drive_sync_folder") {
        await new Promise((r) => setTimeout(r, 1500));
        window.__downloadsArrived = true;
        return 1;
      }
    }
    return invoke(cmd, args);
  };
});
const page = await ctx.newPage();
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Drive");
await page.waitForTimeout(400);

let failures = 0;
const ok = (pass, yes, no) => { console.log(pass ? `PASS ${yes}` : `FAIL ${no}`); if (!pass) failures++; };

await page.locator(".drive-row-hit", { hasText: "Downloads" }).dblclick();
await page.waitForTimeout(300);
const during = await page.locator(".drive-name").allInnerTexts();
ok(!during.includes("Keynote") && !during.includes("Semester Plan.pdf"),
  "while a folder loads, the last folder's rows are not shown", `shown while loading: ${JSON.stringify(during)}`);
await page.waitForTimeout(1700);
const after = await page.locator(".drive-name").allInnerTexts();
ok(after.length === 1 && after[0].startsWith("Ticket"), "and then its own rows arrive", `then: ${JSON.stringify(after)}`);
// A new folder shows at once, marked as being made, while Apple takes its time.
await page.locator(".crumb").first().click();
await page.waitForTimeout(400);
await page.getByRole("button", { name: "New Folder" }).click();
await page.keyboard.type("Trips");
await page.keyboard.press("Enter");
await page.waitForTimeout(200);
const making = await page.locator(".drive-row.is-busy").allInnerTexts();
ok(making.some((t) => t.includes("Trips") && t.includes("Creating")), "a new folder shows at once, being made",
  `straight after Return: ${JSON.stringify(making)}`);
// Gone elsewhere before it finished: the folder on screen stays the one on screen. A refresh
// bound to where the work began re-read that folder at the end, and its rows replaced these.
await page.locator(".drive-row-hit", { hasText: "Downloads" }).first().dblclick();
await page.waitForTimeout(2200);
const inDownloads = await page.locator(".drive-name").allInnerTexts();
await page.waitForTimeout(500);
const stayed = await page.locator(".drive-name").allInnerTexts();
const crumb = await page.locator('.crumb[aria-current="true"]').innerText();
ok(crumb === "Downloads" && stayed.some((t) => t.startsWith("Ticket")) && !stayed.includes("Keynote"),
  "a folder made elsewhere finishing does not pull the screen back", `in ${crumb}: ${JSON.stringify(stayed)} (was ${JSON.stringify(inDownloads)})`);
await b.close();
process.exitCode = failures === 0 ? 0 : 1;
