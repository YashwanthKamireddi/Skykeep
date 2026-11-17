// The viewer as icloud.com's: where the photo is among them ("2 of 5") under its date, and a
// button for full screen; Escape leaves full screen before it closes the photo, and closing a
// photo leaves full screen too.
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
const asked = () => page.evaluate(() => window.__fullscreen ?? []);

await page.locator(".px-tile").nth(1).dblclick();
await page.waitForTimeout(300);
ok(/2 of 5/.test(await page.locator(".px-viewer-when").innerText()), `where the photo is among them, under its date  "${(await page.locator(".px-viewer-when").innerText()).replace(/\n/g, " ")}"`);
await page.keyboard.press("ArrowRight");
await page.waitForTimeout(150);
ok(/3 of 5/.test(await page.locator(".px-viewer-when").innerText()), "and it follows the arrows");
await page.getByRole("button", { name: "Enter Full Screen" }).click();
await page.waitForTimeout(150);
ok(JSON.stringify(await asked()) === "[true]" && (await page.getByRole("button", { name: "Exit Full Screen" }).count()) === 1, "the button takes the window full screen, and becomes Exit");
// Full screen is the photo alone, as icloud.com's: the viewer covers the whole window, black,
// no sidebar showing; the photo uses the whole height.
const box = await page.locator(".px-viewer").boundingBox();
ok(box.x === 0 && box.y === 0 && box.width === 1280 && box.height === 800, `the viewer covers the whole window  (${JSON.stringify(box)})`);
const hit = await page.evaluate(() => [document.elementFromPoint(100, 400)?.closest(".px-viewer") !== null, getComputedStyle(document.querySelector(".px-backdrop")).backgroundColor]);
ok(hit[0] && hit[1] === "rgb(0, 0, 0)", `nothing of the app beside it, on black  (${hit[1]})`);
const stage = await page.locator(".px-stage").boundingBox();
ok(stage.height === 800 && stage.width === 1280, `the photo has the whole screen  (${stage.width}×${stage.height})`);
ok((await page.locator(".px-strip").count()) === 0, "no thumbnails until asked for");
// Its bar goes out of sight, and comes back when the pointer goes to the top.
await page.mouse.move(640, 400);
await page.waitForTimeout(2000);
ok(!(await page.locator(".px-full-bar").isVisible()), "the bar goes out of sight");
await page.mouse.move(640, 10);
await page.waitForTimeout(200);
ok(await page.locator(".px-full-bar").isVisible(), "and comes up when the pointer reaches the top");
ok(/3 of 5/.test(await page.locator(".px-full-bar .px-viewer-when").innerText()), "with the date and where it is among them");
// The thumbnails button shows the photos along the bottom; the photo fits above them.
await page.getByRole("button", { name: "Show Thumbnails" }).click();
await page.waitForTimeout(200);
const strip = await page.locator(".px-strip").boundingBox();
const stage2 = await page.locator(".px-stage").boundingBox();
ok(strip && strip.y + strip.height === 800 && stage2.y + stage2.height <= strip.y + 1, `Show Thumbnails puts them along the bottom, the photo above  (strip ${JSON.stringify(strip)})`);
await page.getByRole("button", { name: "Hide Thumbnails" }).click();
await page.waitForTimeout(150);
ok((await page.locator(".px-strip").count()) === 0, "and Hide Thumbnails takes them away");
// The slider zooms the photo (a photo, not the video this one is) about its middle; − and + step it.
await page.keyboard.press("ArrowRight");
await page.waitForTimeout(300);
await page.mouse.move(640, 10);
await page.waitForTimeout(150);
await page.locator(".px-full-bar").getByRole("button", { name: "Zoom In" }).click();
await page.waitForTimeout(400);
const k = await page.locator(".px-zoom input").inputValue();
ok(Number(k) > 1.4 && /scale/.test(await page.locator(".px-pan").evaluate((e) => e.style.transform)), `Zoom In zooms the photo, the slider following  (${k})`);
await page.locator(".px-zoom input").fill("1");
await page.waitForTimeout(200);
ok(!/scale\(/.test(await page.locator(".px-pan").evaluate((e) => e.style.transform)), "and the slider back at its start fits it again");
await page.keyboard.press("Escape");
await page.waitForTimeout(150);
ok(JSON.stringify(await asked()) === "[true,false]" && (await page.locator(".px-viewer").count()) === 1 && (await page.locator(".px-viewer-bar").count()) === 1, "Escape leaves full screen first, the photo still open with its own bar");
await page.keyboard.press("Escape");
await page.waitForTimeout(150);
ok((await page.locator(".px-viewer").count()) === 0, "and Escape again closes the photo");
await page.locator(".px-tile").nth(0).dblclick();
await page.waitForTimeout(300);
await page.getByRole("button", { name: "Enter Full Screen" }).click();
await page.waitForTimeout(100);
await page.keyboard.press("Control+.");
await page.waitForTimeout(200);
ok((await asked()).at(-1) === false && (await page.locator(".px-viewer").count()) === 0, "closing a photo leaves full screen too");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
