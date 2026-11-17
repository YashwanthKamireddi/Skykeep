// The photo open in the viewer leaving the end of the list from elsewhere — hidden, deleted or
// unfavourited on the phone, arriving with a sync — leaves the viewer on the photo before it,
// never on an empty page with the grid hidden behind it and its keys off.
//
//   URL=http://localhost:4203 ENGINE=webkit node .uiverify/photos-viewer-gone.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const URL = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(URL, { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(500);
const tiles = page.locator(".px-tile");
const ids = await tiles.evaluateAll((t) => t.map((x) => x.dataset.id));
const last = ids.at(-1);
await tiles.last().dblclick();
await page.waitForTimeout(300);
ok((await page.locator(".px-viewer").count()) === 1, `the last photo opens  [${last} of ${JSON.stringify(ids)}]`);
// Hidden on the phone; the sync lands.
await page.evaluate(async (id) => {
  window.__photos.find((p) => p.id === id).hidden = true;
  const report = await window.__TAURI_INTERNALS__.invoke("sync_all");
  window.__emit("quince://synced", { ...report, changed: ["photos"] });
}, last);
await page.waitForTimeout(800);
const state = await page.evaluate(() => ({
  viewer: !!document.querySelector(".px-viewer"),
  behind: document.querySelector(".px-scroll")?.hasAttribute("data-behind") ?? false,
  hidden: getComputedStyle(document.querySelector(".px-scroll")).visibility === "hidden",
  tiles: document.querySelectorAll(".px-tile").length,
}));
ok(state.tiles === ids.length - 1, `the photo has left the grid  [${state.tiles} tiles]`);
ok(state.viewer || (!state.behind && !state.hidden), `the viewer is still up, or the grid is shown again — never an empty page  ${JSON.stringify(state)}`);
await b.close();
console.log(failures ? `\n${failures} FAILED` : "\nall passed");
process.exit(failures ? 1 : 0);
