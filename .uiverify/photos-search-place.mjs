// A search finds across the whole library, so what it finds takes the library's actions: typed
// over Recently Deleted it offered Delete All — every live photo found, deleted for good — and
// Delete asked to delete them permanently.
//
//   URL=http://localhost:4203 ENGINE=webkit node .uiverify/photos-search-place.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(400);
await page.locator(".px-source", { hasText: "Recently Deleted" }).click();
await page.waitForTimeout(300);
ok((await page.getByRole("button", { name: "Delete All" }).count()) === 1, "Recently Deleted offers Delete All");
await page.getByRole("searchbox", { name: "Search Photos" }).or(page.locator('input[aria-label="Search Photos"]')).first().fill("IMG");
await page.waitForTimeout(600);
const found = await page.locator(".px-tile").count();
ok(found > 0, `the search finds photos  [${found}]`);
ok((await page.getByRole("button", { name: "Delete All" }).count()) === 0, "and offers no Delete All over them");
await page.locator(".px-tile").first().click({ button: "right" });
await page.waitForTimeout(150);
const items = await page.locator(".row-menu [role=menuitem]").allInnerTexts();
ok(!items.some((t) => /Recover|Delete Permanently|Delete Now/i.test(t)), `their menu is the library's  ${JSON.stringify(items)}`);
await b.close();
console.log(failures ? `\n${failures} FAILED` : "\nall passed");
process.exit(failures ? 1 : 0);
