// A tile's status line fits it: a sync Apple turned away as busy shows a few words of state
// ("iCloud busy, retrying") with the whole sentence as the tile's tooltip and in what it says
// to a screen reader — the sentence as the line was cut off mid-word. At a narrow window and a
// wide one, every tile's status lies inside the tile.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openSettings } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const BUSY = "iCloud is busy right now. Nothing was changed — try again in a minute.";
for (const [w, h] of [[820, 640], [1280, 820]]) {
  const ctx = await b.newContext({ viewport: { width: w, height: h } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
  await ctx.addInitScript((busy) => {
    window.__syncExtra = [{ service: "calendar", label: "Calendar", outcome: "failed", detail: busy, lastSyncedAt: Math.floor(Date.now() / 1000) - 600 }];
  }, BUSY);
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await openSettings(page);
  const tile = page.locator(".launch-tile", { hasText: "Calendar" });
  await tile.waitFor();
  await page.waitForTimeout(300);
  const line = tile.locator(".launch-fresh-text");
  ok((await line.innerText()) === "iCloud busy, retrying", `${w}x${h}: a busy sync says so in a few words  (${JSON.stringify(await line.innerText())})`);
  ok((await tile.getAttribute("title")) === BUSY, `${w}x${h}: the whole sentence is the tile's tooltip`);
  ok((await tile.getAttribute("aria-label")).includes(BUSY), `${w}x${h}: and what it says to a screen reader`);
  const fits = await page.evaluate(() =>
    [...document.querySelectorAll(".launch-tile")].map((t) => {
      const box = t.getBoundingClientRect();
      const text = t.querySelector(".launch-fresh-text");
      const r = text.getBoundingClientRect();
      return { name: t.querySelector(".launch-name")?.textContent, inside: r.right <= box.right - 8 + 1, cut: text.scrollWidth > text.clientWidth + 1, ellipsis: getComputedStyle(text).textOverflow };
    }),
  );
  ok(fits.every((f) => f.inside), `${w}x${h}: every tile's status lies inside its tile  ${JSON.stringify(fits.filter((f) => !f.inside))}`);
  ok(fits.every((f) => !f.cut || f.ellipsis === "ellipsis"), `${w}x${h}: any line too long ends in an ellipsis, never cut mid-word`);
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
