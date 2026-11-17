// Photos dragged onto an album in the sidebar go into it, as on a Mac: one photo, or the
// whole selection when the dragged photo is part of it. The album lights while over it; its
// count rises at once; the album being shown is not somewhere to drop.
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
const trips = page.locator(".px-albums .px-source", { hasText: "Trips" });
const adds = () => page.evaluate(() => (window.__albumCalls ?? []).filter((c) => c.cmd === "photos_album_add").map((c) => c.args.ids));

await page.locator(".px-tile").nth(0).dragTo(trips);
await page.waitForTimeout(300);
ok(JSON.stringify((await adds()).at(-1)) === JSON.stringify(["p1"]), `one photo dragged onto an album goes into it  ${JSON.stringify((await adds()).at(-1))}`);
ok(/3/.test(await trips.innerText()), "and the album's count rises at once");

await page.locator(".px-tile").nth(2).click();
await page.locator(".px-tile").nth(4).click({ modifiers: ["Control"] });
await page.locator(".px-tile").nth(4).dragTo(trips);
await page.waitForTimeout(300);
ok(JSON.stringify((await adds()).at(-1)) === JSON.stringify(["p3", "p5"]), `a selection dragged goes in whole  ${JSON.stringify((await adds()).at(-1))}`);

// Over it, the album lights; dragging within the album shown, its own row does not.
const tile = await page.locator(".px-tile").nth(1).boundingBox();
const row = await trips.boundingBox();
await page.mouse.move(tile.x + 20, tile.y + 20);
await page.mouse.down();
await page.mouse.move(row.x + 30, row.y + row.height / 2, { steps: 8 });
// Resting over the row a moment, as a hand does: the engine reports the pointer over it.
await page.mouse.move(row.x + 40, row.y + row.height / 2, { steps: 3 });
await page.waitForTimeout(100);
const lit = await trips.getAttribute("data-drop");
await page.mouse.up();
ok(lit !== null, "the album lights while photos are over it");
await page.waitForTimeout(200);
ok((await trips.getAttribute("data-drop")) === null, "and is plain again after");
ok((await page.locator(".px.is-dropping").count()) === 0, "a drag from the library is not taken for files arriving");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
