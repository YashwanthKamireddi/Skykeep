// A narrow window: Photos' sidebar folds into one pop-up above the library, so the photos have
// the window's width (beside 480px of window the sidebar took half of it). Every place the
// sidebar goes is in the pop-up, and it stays there on the Albums and Shared Albums pages, so
// there is always a way back. A wide window keeps the sidebar and no pop-up.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

for (const [w, h] of [[480, 760], [700, 760]]) {
  const ctx = await b.newContext({ viewport: { width: w, height: h } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); failures++; });
  await page.goto(BASE, { waitUntil: "networkidle" });
  await openScreen(page, "Photos");
  await page.locator(".px-tile").first().waitFor();
  await page.waitForTimeout(300);
  const r = await page.evaluate(() => {
    const side = document.querySelector(".px-side").getBoundingClientRect();
    const scroll = document.querySelector(".px-scroll").getBoundingClientRect();
    const pick = document.querySelector(".px-compact .picker-button")?.getBoundingClientRect();
    const bar = document.querySelector(".px-bar").getBoundingClientRect();
    return { side: side.width, grid: scroll.width, pick: pick ? { w: pick.width, b: pick.bottom } : null, barTop: bar.top };
  });
  ok(r.side === 0 && r.grid >= w - 40, `${w}px: the sidebar folds away and the photos have the width  (sidebar ${r.side}px, photos ${Math.round(r.grid)}px)`);
  ok(r.pick && r.pick.w > 40 && r.pick.b <= r.barTop + 1, `${w}px: a pop-up stands for it, above the title  ${JSON.stringify(r.pick)}`);

  const pick = page.locator(".px-compact .picker-button");
  const go = async (label) => {
    await pick.click();
    await page.getByRole("option", { name: label, exact: true }).click();
    await page.waitForTimeout(300);
  };
  await go("Favorites");
  ok((await page.locator(".px-bar h1").innerText()) === "Favorites" && (await pick.innerText()).includes("Favorites"), `${w}px: choosing Favorites shows Favorites`);
  await go("Albums");
  ok((await page.locator(".px-albums-page").isVisible()) && (await pick.isVisible()), `${w}px: the Albums page, with the pop-up still there to leave it`);
  await go("Shared Albums");
  ok((await page.locator(".px-main").getAttribute("aria-label")) === "Shared Albums" && (await pick.isVisible()), `${w}px: Shared Albums too`);
  await go("Library");
  ok((await page.locator(".px-bar h1").innerText()) === "Library" && (await page.locator(".px-tile").count()) > 0, `${w}px: and back to the Library`);

  // Keyboard: the pop-up is reached and worked by keys alone.
  await pick.focus();
  await page.keyboard.press("Enter");
  await page.waitForTimeout(150);
  // The first arrow lights the current choice, as a Mac pop-up's; the next moves.
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await page.waitForTimeout(300);
  ok((await page.locator(".px-bar h1").innerText()) === "Favorites", `${w}px: and with the keys alone`);
  await ctx.close();
}

const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
const page = await ctx.newPage();
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(300);
ok((await page.locator(".px-side").isVisible()) && !(await page.locator(".px-compact").isVisible()), "1280px: the sidebar, and no pop-up");
await b.close();
process.exitCode = failures ? 1 : 0;
