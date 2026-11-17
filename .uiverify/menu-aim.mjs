// A menu with a submenu open answers the pointer as a Mac menu does: moving back up to the
// lines above highlights each at once and closes the submenu; only a move heading into the
// submenu keeps it open while it crosses another line.
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
const active = () => page.locator(".row-menu-item.is-active").allInnerTexts();
const centre = async (loc) => { const r = await loc.boundingBox(); return [r.x + r.width / 2, r.y + r.height / 2]; };

await page.locator(".px-tile").first().click({ button: "right" });
const add = page.locator(".row-menu-item", { hasText: "Add to Album" });
const [ax, ay] = await centre(add);
await page.mouse.move(ax, ay, { steps: 4 });
await page.waitForTimeout(80);
ok((await page.locator(".row-menu").count()) === 2, "hovering Add to Album opens its albums beside it");

// Straight back up, to the first line: highlighted at once, the submenu gone.
const first = page.locator(".row-menu").first().locator(".row-menu-item").first();
const [fx, fy] = await centre(first);
await page.mouse.move(fx, fy, { steps: 6 });
await page.waitForTimeout(40);
ok((await active()).join() === (await first.innerText()), `moving back up highlights the line under the pointer at once  ${JSON.stringify(await active())}`);
ok((await page.locator(".row-menu").count()) === 1, "and the submenu closes");

// Down again to Add to Album, then diagonally into the submenu across the line below it.
await page.mouse.move(ax, ay, { steps: 6 });
await page.waitForTimeout(80);
const sub = page.locator(".row-menu").nth(1).locator(".row-menu-item").first();
const [sx, sy] = await centre(sub);
const below = page.locator(".row-menu").first().locator(".row-menu-item").nth((await page.locator(".row-menu").first().locator(".row-menu-item").allInnerTexts()).findIndex((t) => t.startsWith("Add to Album")) + 1);
const [, by] = await centre(below);
// From near Add to Album's right end, down and right toward the submenu, over the line below.
const ab = await add.boundingBox();
await page.mouse.move(ab.x + ab.width - 14, ay, { steps: 2 });
await page.mouse.move(ab.x + ab.width - 4, by, { steps: 2 });
await page.waitForTimeout(40);
ok((await page.locator(".row-menu").count()) === 2, "a move heading into the submenu keeps it open as it crosses the line below");
await page.mouse.move(sx, sy, { steps: 3 });
await page.waitForTimeout(80);
ok((await page.locator(".row-menu").count()) === 2 && (await active()).some((t) => t.startsWith("Add to Album")), "and arrives in it, Add to Album still marked");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
