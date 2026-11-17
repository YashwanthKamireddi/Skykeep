// Any window size, as a Mac app keeps its toolbar: the title whole (never squeezed to a letter),
// its dates on one line, the controls together beside it or under it, and the first photos
// below all of it — never under it. Recently Deleted is one grid, never the same month again.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
for (const [w, h] of [[1280, 800], [900, 700], [760, 700], [560, 700]]) {
  const ctx = await b.newContext({ viewport: { width: w, height: h } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); failures++; });
  await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await openScreen(page, "Photos");
  await page.waitForTimeout(500);
  for (const view of ["Library", "Favorites"]) {
    // From 720px down the sidebar is a pop-up above the library (photos-compact).
    if (await page.locator(".px-side").isVisible()) await page.locator(".px-source", { hasText: view }).first().click();
    else {
      await page.locator(".px-compact .picker-button").click();
      await page.getByRole("option", { name: view, exact: true }).click();
    }
    await page.waitForTimeout(300);
    const r = await page.evaluate(() => {
      const t = document.querySelector(".px-bar h1"), span = document.querySelector(".px-bar .px-span");
      const bar = document.querySelector(".px-bar").getBoundingClientRect();
      const first = document.querySelector(".px-month-title, .px-tile, .px-empty, .px-scroll > *")?.getBoundingClientRect();
      const tools = [...document.querySelectorAll(".px-tools > *")].map((e) => e.getBoundingClientRect().top);
      return {
        whole: t.scrollWidth <= t.clientWidth + 1 && t.clientWidth > 40,
        oneLine: !span || span.getBoundingClientRect().height < 24,
        clear: !first || first.top >= bar.bottom - 1,
        rows: new Set(tools.map((x) => Math.round(x / 10))).size,
      };
    });
    ok(r.whole && r.oneLine && r.clear, `${w}px ${view}: title whole, dates on one line, photos clear of the header  ${JSON.stringify(r)}`);
  }
  await ctx.close();
}
// Recently Deleted: one grid in the order things were deleted, no month headings to repeat.
const ctx = await b.newContext({ viewport: { width: 900, height: 700 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(400);
await page.locator(".px-source", { hasText: "Recently Deleted" }).click();
await page.waitForTimeout(300);
ok((await page.locator(".px-month-title").count()) === 0 && (await page.locator(".px-tile").count()) > 0, "Recently Deleted is one grid, with no month headings to repeat");
await b.close();
process.exitCode = failures ? 1 : 0;
