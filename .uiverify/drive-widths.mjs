// Drive at every window width, as a half-screen window on his desktop showed it: the bar is
// laid out once — the name with its count under it, the controls beside it or, with too
// little room, under it — and never moves when the count arrives a moment later. Nothing runs
// off the edge; at half a screen the controls are icons with their names kept for screen
// readers and tooltips.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
for (const [w, h] of [[1920, 1040], [960, 1040], [700, 900], [480, 800]]) {
  const ctx = await b.newContext({ viewport: { width: w, height: h } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, driveFixture: true });
  await ctx.addInitScript(() => { window.__slowDrive = 900; });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
  await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await openScreen(page, "Drive").catch(() => openScreen(page, "Drive"));
  await page.waitForTimeout(150);
  const box = () => page.evaluate(() => {
    const bar = document.querySelector(".drive > .drive-bar"), up = [...bar.querySelectorAll("button")].find((x) => /Upload/.test(x.textContent));
    const r = bar.getBoundingClientRect(), u = up.getBoundingClientRect();
    return { h: Math.round(r.height), upTop: Math.round(u.top), upRight: Math.round(u.right), right: Math.round(r.right),
      scroll: document.querySelector("main.content").scrollWidth - document.querySelector("main.content").clientWidth,
      count: document.querySelector(".drive-count")?.textContent.trim() ?? "" };
  });
  const before = await box();
  await page.waitForFunction(() => (document.querySelector(".drive-count")?.textContent.trim() ?? "").length > 0, null, { timeout: 8000 }).catch(() => {});
  await page.waitForTimeout(200);
  const after = await box();
  ok(after.count.length > 0 && before.h === after.h && before.upTop === after.upTop,
    `${w}px: the bar does not move when the count arrives  (${before.h}→${after.h}px high, Upload at ${before.upTop}→${after.upTop}; "${after.count}")`);
  ok(after.upRight <= after.right + 1 && after.scroll <= 0, `${w}px: nothing runs off the edge`);
  if (w === 960) {
    const newFolder = page.locator(".drive > .drive-bar button[title='New Folder']");
    ok((await newFolder.evaluate((e) => getComputedStyle(e).fontSize)) === "0px" && (await newFolder.getAttribute("title")) === "New Folder",
      "960px: the controls are icons, each still named");
  }
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
