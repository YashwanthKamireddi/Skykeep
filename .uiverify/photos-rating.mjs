// Star ratings, as icloud.com keeps them: a rated photo carries its stars on its tile; Info's
// stars set them (the same again takes them away); Rated holds the rated, grouped by stars.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

ok((await page.locator(".px-tile .px-stars").allInnerTexts()).join() === "5★", "a rated photo carries its stars on its tile");
await page.locator(".px-source", { hasText: "Rated" }).click();
await page.waitForTimeout(250);
ok((await page.locator(".px-month-title").allInnerTexts()).join() === "5 Stars" && (await page.locator(".px-tile").count()) === 1,
  "Rated holds the rated, under how many stars");
await page.locator(".px-source", { hasText: "Library" }).click();
await page.waitForTimeout(250);

await page.locator(".px-tile").nth(1).dblclick();
await page.waitForTimeout(600);
if (!(await page.locator(".px-info").count())) await page.keyboard.press("i");
await page.getByRole("radio", { name: "3 stars" }).click();
await page.waitForTimeout(200);
ok(JSON.stringify(await page.evaluate(() => window.__ratings?.at(-1))) === JSON.stringify({ id: "p2", rating: 3 }), "Info's third star rates it three");
ok((await page.locator(".px-star.is-on").count()) === 3, "and three stars light at once");
await page.keyboard.press("Escape");
await page.waitForTimeout(650);
await page.locator(".px-source", { hasText: "Rated" }).click();
await page.waitForTimeout(250);
ok((await page.locator(".px-month-title").allInnerTexts()).join() === "5 Stars,3 Stars", `and Rated has it under 3 Stars  ${JSON.stringify(await page.locator(".px-month-title").allInnerTexts())}`);
await page.locator(".px-tile").nth(1).dblclick();
await page.waitForTimeout(600);
await page.keyboard.press("i");
await page.getByRole("radio", { name: "3 stars" }).click();
await page.waitForTimeout(300);
ok((await page.evaluate(() => window.__ratings.at(-1).rating)) === 0, "the same star again takes the rating away");
await page.keyboard.press("Escape");
await page.waitForTimeout(650);
ok((await page.locator(".px-tile").count()) === 1, "and the photo leaves Rated");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
