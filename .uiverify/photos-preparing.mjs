// Photos while Apple is still preparing the library (a pass answered "preparing"): the screen
// says so, calmly and truly — never "Your photos and videos show here" as though there were
// none, and never "Getting your photos" as though they were on their way now. Once a pass
// brings the library, the photos are shown.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(() => {
  window.__photos = [];
  window.__syncExtra = [{
    service: "photos", label: "Photos", outcome: "preparing",
    detail: "Apple is still preparing your photo library. It will appear here when it's ready.",
    lastSyncedAt: null,
  }];
});
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(500);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

const shown = await page.locator(".px-empty").allInnerTexts();
ok(shown.length === 1 && shown[0].includes("Apple is still preparing your photo library. It will appear here when it's ready."),
  `the library being prepared is said as it is  ${JSON.stringify(shown)}`);
ok(!(await page.locator(".px-empty", { hasText: "show here" }).count()), "not told it has no photos");
ok(!(await page.locator(".arriving").count()), "not told they are coming down now");

// Ready, and read: the photos, and the words gone.
await page.evaluate(async () => {
  window.__syncExtra = [{ service: "photos", label: "Photos", outcome: "ok", detail: "1 photo", lastSyncedAt: Math.floor(Date.now() / 1000) }];
  window.__photos = [{ id: "p1", master: "mp1", takenAt: 1792500000, addedAt: 1792500000, kind: "image", width: 4032, height: 3024,
    duration: null, filename: "IMG_p1.JPG", size: 2400000, uti: "public.jpeg", favorite: false, hidden: false, deleted: false,
    deletedAt: null, subtype: 0, changeTag: null }];
  const status = await window.__TAURI_INTERNALS__.invoke("sync_status", {});
  window.__emit?.("quince://synced", { ...status, changed: ["photos"] });
});
await page.waitForTimeout(800);
ok((await page.locator(".px-tile").count()) === 1, "once ready, the library shows");
ok(!(await page.locator(".px-preparing").count()), "and the preparing words are gone");

await b.close();
if (failures) process.exitCode = 1;
