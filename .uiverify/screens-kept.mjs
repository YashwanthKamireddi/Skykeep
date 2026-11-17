// Screens are kept once opened, as a Mac app's windows are: coming back to one is instant and
// finds it as it was left — the same page, its scroll, its selection, the photo open in it —
// while out of sight it is inert and answers no keys.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
await ctx.addInitScript(() => {
  const P = (id, t) => ({ id, master: `m${id}`, takenAt: t, addedAt: t, kind: "image", width: 4032, height: 3024, duration: null,
    filename: `IMG_${id}.JPG`, size: 1, uti: "public.jpeg", favorite: false, hidden: false, deleted: false, deletedAt: null, subtype: 0, changeTag: null });
  window.__photos = Array.from({ length: 600 }, (_, i) => P(`q${i}`, 1792600000 - i * 40000));
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

await openScreen(page, "Photos");
await page.waitForTimeout(500);
await page.locator(".px-scroll").evaluate((e) => { e.scrollTop = 1500; e.__kept = true; });
await page.waitForTimeout(300);
// A photo on screen there, clicked where it is.
const spot = await page.evaluate(() => {
  const t = [...document.querySelectorAll(".px-tile")].find((e) => { const r = e.getBoundingClientRect(); return r.top > 200 && r.bottom < innerHeight - 100; });
  const r = t.getBoundingClientRect();
  return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
});
await page.mouse.click(spot.x, spot.y);
const picked = await page.locator('.px-tile[aria-selected="true"]').getAttribute("data-id");
await openScreen(page, "Notes");
await page.waitForTimeout(300);
ok((await page.locator(".screen-slot[hidden][inert] .px-scroll").count()) === 1, "Photos, out of sight, is kept — hidden and inert");
await openScreen(page, "Photos");
await page.waitForTimeout(100);
const back = await page.locator(".px-scroll").evaluate((e) => ({ same: e.__kept === true, top: e.scrollTop }));
ok(back.same, "coming back finds the same page, not one built again");
ok(Math.abs(back.top - 1500) < 2, `at the place it was left  (${back.top})`);
ok((await page.locator('.px-tile[aria-selected="true"]').getAttribute("data-id")) === picked, "with the same photo chosen");

// Keys typed on another screen never reach a kept one.
await page.mouse.dblclick(spot.x, spot.y);
await page.waitForTimeout(300);
const at = (await page.locator(".px-viewer-when").innerText()).replace(/\n/g, " ");
await openScreen(page, "Calendar");
await page.waitForTimeout(400);
await page.locator("body").press("ArrowRight");
await page.locator("body").press("Escape");
await page.waitForTimeout(200);
await openScreen(page, "Photos");
await page.waitForTimeout(200);
ok((await page.locator(".px-viewer").count()) === 1 && (await page.locator(".px-viewer-when").innerText()).replace(/\n/g, " ") === at,
  `the photo left open is still open, on the same photo — another screen's keys never reached it  (${at})`);
ok(Math.abs((await page.locator(".px-scroll").evaluate((e) => e.scrollTop)) - 1500) < 2, "and the grid under it still where it was");
ok((await page.locator(".screen-slot:not([hidden])").count()) === 1, "only one screen is ever showing");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
