// The open folder's pictures are asked for when it opens and when something in it changes —
// not on every ten-second look that found nothing new, which sent and redrew every picture in
// the folder each time. A folder whose pictures never came is asked again.
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
    if (cmd === "drive_thumbnails") {
      window.__thumbs = (window.__thumbs ?? 0) + 1;
      if (window.__thumbsFail) throw new Error("offline");
    }
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
await page.goto(process.env.BASE ?? process.env.URL ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Drive");
await page.waitForTimeout(800);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "ok  " : "FAIL"} ${m}`); if (!p) failures++; };
const thumbs = () => page.evaluate(() => window.__thumbs ?? 0);
const checks = () => page.evaluate(() => window.__checks ?? 0);

const look = async () => {
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await page.waitForTimeout(500);
};
ok((await thumbs()) === 1, "opening the folder asks for its pictures");
ok((await page.locator("img.drive-thumb").count()) > 0, "and draws them");

// Nothing new at a look (the window coming to the front): the folder is looked at, its
// pictures are not asked for again.
let before = await thumbs();
let looked = await checks();
await look();
ok((await checks()) > looked, "the open folder is looked at again when the window comes to the front");
ok((await thumbs()) === before, "and a check that found nothing new asks for no pictures");
ok((await page.locator("img.drive-thumb").count()) > 0, "which stay on screen");

// Something new: its picture is asked for.
await page.evaluate(() => { window.__phone = true; });
await look();
ok((await thumbs()) === before + 1, "a file arriving asks for the pictures again");

// Pictures that never came (offline) are asked for at the next look, changed or not.
await openScreen(page, "Drive");
await page.evaluate(() => { window.__thumbsFail = true; window.__phone = false; });
await page.locator(".drive-row-hit", { hasText: "Downloads" }).dblclick();
await page.waitForTimeout(800);
await page.evaluate(() => { window.__thumbsFail = false; });
before = await thumbs();
await look();
ok((await thumbs()) === before + 1, "a folder whose pictures failed is asked again");
before = await thumbs();
await look();
ok((await thumbs()) === before, "and, once they came, not again while nothing changes");

await b.close();
if (failures) process.exitCode = 1;
