// Search, as icloud.com's toolbar has it: what is typed finds photos by name, title, caption,
// keyword, year or month, as it is typed; the title says what was searched and how many;
// nothing found says so; Escape or a view chosen ends it; the letters are words, not keys.
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
const ids = () => page.locator(".px-tile").evaluateAll((t) => t.map((x) => x.dataset.id).join());
const field = page.getByLabel("Search Photos");

await field.click();
await page.keyboard.type("beach");
await page.waitForTimeout(400);
ok((await ids()) === "p2", `a keyword finds its photo  ${await ids()}`);
ok((await page.locator(".px-bar h1").innerText()) === "“beach”" && (await page.locator(".px-span").innerText()) === "1 Result",
  "the title says what was searched and how many");
ok((await page.evaluate(() => (window.__flags ?? []).length)) === 0, "the letters typed are words, not keys");
await field.fill("september 2026");
await page.waitForTimeout(400);
ok((await ids()) === "p4,p5", `a month and year find that month's  ${await ids()}`);
await field.fill("nothing like this");
await page.waitForTimeout(400);
ok((await page.locator(".px-tile").count()) === 0 && /No photos match/.test(await page.locator(".px-empty").innerText()), "nothing found says so");
await page.keyboard.press("Escape");
await page.waitForTimeout(400);
ok((await field.inputValue()) === "" && (await page.locator(".px-tile").count()) === 5 && (await page.locator(".px-bar h1").innerText()) === "Library",
  "Escape ends the search, back to the library");
await field.fill("img_p3");
await page.waitForTimeout(400);
await page.locator(".px-source", { hasText: "Favorites" }).click();
await page.waitForTimeout(300);
ok((await field.inputValue()) === "" && (await page.locator(".px-bar h1").innerText()) === "Favorites", "choosing a view ends it too");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
