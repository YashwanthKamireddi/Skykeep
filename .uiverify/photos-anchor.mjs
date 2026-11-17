// Shift-click widens the selection from the photo last clicked, wherever the list has since
// put it: after the order is turned round the range ran from where that photo used to stand,
// and the photo clicked first could be left out of it.
//
//   URL=http://localhost:4203 ENGINE=webkit node .uiverify/photos-anchor.mjs
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
await page.waitForTimeout(500);
const order = () => page.locator(".px-tile").evaluateAll((t) => t.map((x) => x.dataset.id).join(","));
const chosen = () => page.locator('.px-tile[aria-selected="true"]').evaluateAll((t) => t.map((x) => x.dataset.id).sort().join(","));
ok((await order()) === "p1,p2,p3,p4,p5", `newest first  ${await order()}`);
await page.locator('.px-tile[data-id="p2"]').click();
await page.getByRole("button", { name: "Sort" }).click();
await page.locator(".row-menu").getByText("Oldest First").click();
await page.waitForTimeout(300);
ok((await order()) === "p5,p4,p3,p2,p1", `turned round  ${await order()}`);
await page.locator('.px-tile[data-id="p1"]').click({ modifiers: ["Shift"] });
await page.waitForTimeout(150);
ok((await chosen()) === "p1,p2", `Shift-click selects from the photo clicked first to this one  [${await chosen()}]`);
// Put back as it was for the next run.
await page.getByRole("button", { name: "Sort" }).click();
await page.locator(".row-menu").getByText("Newest First").click();
await b.close();
console.log(failures ? `\n${failures} FAILED` : "\nall passed");
process.exit(failures ? 1 : 0);
