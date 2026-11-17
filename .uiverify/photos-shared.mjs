// Shared Albums, as iOS 27 keeps them and icloud.com lists them: a card each with its newest
// photo, its name, "Created by You" or "Shared with You", and when it expires; a card opens the
// album in the grid. A shared album is looked at, copied and saved — never edited from here:
// no Favorite, Hide, Delete or Remove in its menus or viewer, and Delete does nothing.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
await ctx.addInitScript(() => {
  const P = (id, takenAt) => ({ id, master: `m${id}`, takenAt, addedAt: takenAt, kind: "image", width: 1200, height: 2600, duration: null,
    filename: `IMG_${id}.JPG`, size: null, uti: "public.jpeg", favorite: false, hidden: false, deleted: false, deletedAt: null, subtype: 0, changeTag: null });
  // 19 November 2026, 12:00 UTC: "Expires November 19" wherever the test runs.
  window.__shared = [
    { zone: "SharedCollection-W", database: "private", owner: null, title: "Wallpapers", expiresAt: 1795089600, count: 2, cover: "s2" },
    { zone: "SharedCollection-F", database: "shared", owner: "_other", title: "Family", expiresAt: null, count: 1, cover: "s3" },
  ];
  window.__sharedPhotos = { "SharedCollection-W": [P("s2", 1792600000), P("s1", 1792500000)], "SharedCollection-F": [P("s3", 1792000000)] };
});
// Every cover picture the page puts up, as it puts it up (a cover that fails is replaced at once).
await ctx.addInitScript(() => addEventListener("DOMContentLoaded", () => new MutationObserver(() => {
  for (const img of document.querySelectorAll(".px-albums-page .px-card img")) (window.__coverSrcs ??= new Set()).add(img.getAttribute("src"));
}).observe(document.body, { childList: true, subtree: true })));
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const cards = () => page.locator(".px-albums-page .px-card").allInnerTexts();

await page.locator(".px-source", { hasText: "Shared Albums" }).click();
await page.waitForTimeout(300);
ok((await page.locator(".px-albums-page h1").innerText()) === "Shared Albums" && (await page.locator(".px-tile:visible").count()) === 0, "Sharing → Shared Albums opens its page in place of the photos");
ok((await page.locator(".px-albums-head .px-span").innerText()) === "2 Shared Albums", "with how many there are");
const [w, f] = await cards();
ok(/Wallpapers[\s\S]*Created by You[\s\S]*Expires November 19/.test(w ?? ""), `one of yours: its name, made by you, when it ends  ${JSON.stringify(w)}`);
ok(/Family[\s\S]*Shared with You/.test(f ?? "") && !/Expires/.test(f ?? ""), `one shared with you says so  ${JSON.stringify(f)}`);
ok(await page.evaluate(() => [...(window.__coverSrcs ?? [])].some((src) => src.includes("s2")) && [...window.__coverSrcs].some((src) => src.includes("s3"))), "each card shows its newest photo");
// The test's server sends no pictures: each cover fails to load, as offline, and the album's
// glyph takes its place rather than a broken image.
await page.waitForTimeout(300);
ok((await page.locator(".px-albums-page .px-card img").count()) === 0 && (await page.locator(".px-albums-page .px-card svg").count()) === 2, "a cover that won't load gives way to the album's glyph");
ok((await page.getByRole("button", { name: "New Shared Album" }).count()) === 0, "no New Shared Album: making one isn't possible from here yet");

// Opened straight after another view — Recently Deleted, as in the recording that found it —
// with Apple slow to answer: not one frame of that view's photos shows before the album's own.
await page.locator(".px-source", { hasText: "Recently Deleted" }).click();
await page.waitForTimeout(400);
await page.locator(".px-source", { hasText: "Shared Albums" }).click();
await page.waitForTimeout(300);
await page.evaluate(() => {
  window.__pageDelay = 150;
  window.__indexDelay = 150;
  window.__seen = new Set();
  window.__titles = new Set();
  const tick = () => {
    if (!document.querySelector(".px-main[data-page]")) {
      for (const t of document.querySelectorAll(".px-tile")) if (t.getClientRects().length) window.__seen.add(t.dataset.id);
      const h = document.querySelector(".px-main h1")?.textContent;
      if (h) window.__titles.add(h);
    }
    if (!window.__stopSeen) requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
});
await page.locator(".px-albums-page .px-card", { hasText: "Wallpapers" }).click();
await page.waitForTimeout(700);
const [seen, titles] = await page.evaluate(() => { window.__stopSeen = true; window.__pageDelay = 0; window.__indexDelay = 0; return [[...window.__seen], [...window.__titles]]; });
ok(seen.length > 0 && seen.every((id) => id.startsWith("s")) && JSON.stringify(titles) === '["Wallpapers"]',
  `opening it shows only its own title and photos, never the last view's  ${JSON.stringify({ seen, titles })}`);
await page.locator(".px-source", { hasText: "Shared Albums" }).click();
await page.waitForTimeout(300);

// Opened: its photos in the grid, under its name.
await page.locator(".px-albums-page .px-card", { hasText: "Wallpapers" }).click();
await page.waitForTimeout(350);
ok((await page.locator(".px-bar h1").innerText()) === "Wallpapers" && (await page.locator(".px-tile").count()) === 2, "a card opens the album: its photos, under its name");
// Left open, the album asks for nothing more: a shared photo the backend couldn't find by id
// had the grid ask for it again every frame, and its tiles flickered.
const gets = () => page.evaluate(() => (window.__gets ?? []).length);
await page.evaluate(() => window.__emit?.("quince://synced", { services: [{ service: "photos", label: "Photos", outcome: "ok", detail: null, lastSyncedAt: Math.floor(Date.now() / 1000) }], asOf: Math.floor(Date.now() / 1000), tookMs: 10, changed: ["photos"] }));
await page.waitForTimeout(600);
const settled = await gets();
await page.mouse.wheel(0, 200);
await page.waitForTimeout(1000);
ok((await gets()) === settled, `an open shared album stops asking for its photos after a sync  (${settled} → ${await gets()})`);
const albumAfterSync = async () => { await page.evaluate(() => window.__emit?.("quince://synced", { services: [{ service: "photos", label: "Photos", outcome: "ok", detail: null, lastSyncedAt: Math.floor(Date.now() / 1000) }], asOf: Math.floor(Date.now() / 1000), tookMs: 10, changed: ["photos"] })); await page.waitForTimeout(300); };
await albumAfterSync();
ok((await page.locator(".px-bar h1").innerText()) === "Wallpapers", "and stays open when a sync lands (it is in no album list of yours)");

ok((await page.getByRole("button", { name: "Add Photos" }).count()) === 0, "no Add Photos in a shared album: it would add to your library, not the album");
// Looked at, never edited.
await page.locator(".px-tile").first().click({ button: "right" });
const menu = await page.locator(".row-menu").innerText();
ok(/Download/.test(menu) && /Copy/.test(menu) && !/Favorite|Hide|Delete|Remove|Add to Album/.test(menu), `its menu saves and copies, and edits nothing  ${JSON.stringify(menu)}`);
await page.keyboard.press("Escape");
await page.locator(".px-tile").first().click();
await page.keyboard.press("Delete");
await page.waitForTimeout(250);
ok((await page.locator(".confirm").count()) === 0 && (await page.locator(".px-tile").count()) === 2, "Delete does nothing in a shared album");
await page.locator(".px-tile").first().dblclick();
await page.waitForTimeout(400);
const buttons = await page.locator(".px-viewer button[aria-label]").evaluateAll((els) => els.map((e) => e.getAttribute("aria-label")));
ok(buttons.includes("Download") && !buttons.some((l) => /Favorite|Delete|Remove/.test(l ?? "")), `the viewer offers Download, not Favorite or Delete  ${JSON.stringify(buttons)}`);
await page.keyboard.press("Escape");
await page.waitForTimeout(250);

// No albums: said plainly.
await page.evaluate(() => { window.__shared = []; });
await page.locator(".px-source", { hasText: "Library" }).click();
await page.waitForTimeout(250);
await page.locator(".px-source", { hasText: "Shared Albums" }).click();
await page.waitForTimeout(300);
ok(/Albums you share from your iPhone/.test(await page.locator(".px-albums-page").innerText()), "with none, the page says where they come from");

await page.locator(".px-source", { hasText: "Library" }).click();
await page.waitForTimeout(300);
ok((await page.locator(".px-albums-page").count()) === 0 && (await page.locator(".px-tile:visible").count()) > 0, "Library brings the photos back");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
