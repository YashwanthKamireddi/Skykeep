// A slideshow, as Photos plays one: started from a photo it begins there and goes on by
// itself; from a selection it shows just those; Space pauses, the arrows step, Escape ends
// it and leaves the screen as it was.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
// Real pictures (a flat colour each), so each photo is fetched and decoded as in the app: the
// show passes over a picture that cannot be had.
await ctx.addInitScript(() => { window.__picBase = "http://pics.test"; });
await ctx.route("http://pics.test/**", (route) => {
  const id = /pic%2F([^%]+)%2F/.exec(route.request().url())?.[1] ?? "x";
  const hue = [...id].reduce((h, c) => h + c.charCodeAt(0) * 37, 0) % 360;
  return route.fulfill({ contentType: "image/svg+xml", body: `<svg xmlns="http://www.w3.org/2000/svg" width="1536" height="1152"><rect width="1536" height="1152" fill="hsl(${hue},60%,50%)"/></svg>` });
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const count = () => page.locator(".px-show-count").innerText();

await page.locator(".px-tile").nth(1).click({ button: "right" });
await page.locator(".row-menu").getByText("Start Slideshow").click();
await page.waitForTimeout(200);
ok((await page.locator(".px-show").count()) === 1 && (await count()) === "2 of 5", `it begins at the photo it was started from  "${await count()}"`);
const fills = await page.locator(".px-show").evaluate((el) => {
  const r = el.getBoundingClientRect();
  return r.left === 0 && r.top === 0 && Math.round(r.width) === innerWidth && Math.round(r.height) === innerHeight;
});
ok(fills, "it fills the whole window, not a box inside the screen");
// Each photo holds 4 s, then dissolves into the next over 0.8 s.
await page.waitForTimeout(5200);
ok((await count()) === "3 of 5", `and goes on by itself  "${await count()}"`);
await page.keyboard.press(" ");
await page.waitForTimeout(5200);
ok((await count()) === "3 of 5" && (await page.getByRole("button", { name: "Play" }).count()) === 1, "Space pauses it");
// A step waits for the next picture to be decoded — never half shown — then dissolves in.
await page.keyboard.press("ArrowRight");
await page.waitForTimeout(1000);
ok((await count()) === "4 of 5", "the right arrow steps on");
await page.keyboard.press("ArrowLeft");
await page.waitForTimeout(1000);
await page.keyboard.press("ArrowLeft");
await page.waitForTimeout(1000);
ok((await count()) === "2 of 5", "and the left back");
// The next dissolves in over the one leaving (drawn frame by frame by the page, never the
// compositor's), the leaving one still whole beneath it, then gone.
const layers = () => page.evaluate(() => {
  const ls = [...document.querySelectorAll(".px-slide")].sort((a, b) => +b.style.zIndex - +a.style.zIndex);
  return { arriving: Number(getComputedStyle(ls[0]).opacity), leaving: Number(getComputedStyle(ls[1]).opacity), engine: document.querySelector(".px-show").getAnimations({ subtree: true }).length };
});
await page.keyboard.press("ArrowRight");
let mid = await layers();
for (let i = 0; i < 40 && !(mid.arriving > 0 && mid.arriving < 1); i++) {
  await page.waitForTimeout(20);
  mid = await layers();
}
await page.waitForTimeout(1000);
const after = await layers();
ok(mid.arriving > 0 && mid.arriving < 1 && mid.leaving > 0.9 && mid.engine === 0 && after.arriving === 1 && after.leaving === 0,
  `the next dissolves in over the one leaving, none of it the engine's, then the old one goes  ${JSON.stringify({ mid, after })}`);
await page.keyboard.press("ArrowLeft");
await page.waitForTimeout(1000);
await page.keyboard.press("Escape");
await page.waitForTimeout(200);
ok((await page.locator(".px-show").count()) === 0 && (await page.locator(".px-tile").count()) === 5, "Escape ends it, the library as it was");

await page.locator(".px-tile").nth(0).click();
await page.locator(".px-tile").nth(3).click({ modifiers: ["Control"] });
await page.locator(".px-tile").nth(3).click({ button: "right" });
await page.locator(".row-menu").getByText("Start Slideshow").click();
await page.waitForTimeout(200);
ok(/of 2$/.test(await count()), `from a selection it shows just those  "${await count()}"`);
await page.keyboard.press("Escape");
await page.locator(".px-tile").nth(2).dblclick();
await page.waitForTimeout(600);
await page.getByRole("button", { name: "Start Slideshow" }).click();
await page.waitForTimeout(200);
ok((await page.locator(".px-show").count()) === 1, "the viewer starts one too");
await page.keyboard.press("Escape");
await page.waitForTimeout(200);
ok((await page.locator(".px-viewer").count()) === 1, "and Escape ends the slideshow, not the photo behind it");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
