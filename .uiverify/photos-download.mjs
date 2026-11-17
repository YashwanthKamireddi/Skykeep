// Download Options, as icloud.com's: from a photo's menu (in the grid, or right-clicked in the
// viewer), the photos and how many, then Unmodified Originals, Highest Resolution, Most
// Compatible — each downloads at once; a selection downloads every photo in it; Escape closes the
// sheet and nothing else.
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
const last = () => page.evaluate(() => window.__downloads?.at(-1));

await page.locator(".px-tile").nth(1).click({ button: "right" });
await page.locator(".row-menu").getByText("More Download Options").click();
const sheet = page.getByRole("dialog", { name: "Download Options" });
ok((await sheet.count()) === 1 && /1 Item/.test(await sheet.innerText()) && /Unmodified Originals[\s\S]*Including Edits[\s\S]*Highest Resolution[\s\S]*Most Compatible/.test(await sheet.innerText()),
  "the sheet offers what icloud.com's does");
await page.getByRole("button", { name: "Download Most Compatible" }).click();
await page.waitForTimeout(200);
ok(JSON.stringify(await last()) === JSON.stringify({ id: "p2", want: "mostCompatible" }) && (await sheet.count()) === 0, "a row downloads that kind, and the sheet goes");

await page.locator(".px-tile").nth(0).click();
await page.locator(".px-tile").nth(3).click({ modifiers: ["Control"] });
await page.locator(".px-tile").nth(3).click({ button: "right" });
const before = await page.evaluate(() => window.__downloads.length);
await page.locator(".row-menu").getByText("Download 2 Items").click();
await page.waitForTimeout(300);
ok((await page.evaluate(() => window.__downloads.length)) - before === 2, "a selection downloads every photo in it");

await page.keyboard.press("Escape");
await page.locator(".px-tile").nth(2).dblclick();
await page.waitForTimeout(600);
await page.locator(".px-stage").click({ button: "right", position: { x: 300, y: 300 } });
ok((await page.locator(".row-menu").getByText("More Download Options").count()) === 1, "right-clicking the open photo gives its menu");
await page.locator(".row-menu").getByText("More Download Options").click();
await page.keyboard.press("Escape");
await page.waitForTimeout(200);
ok((await sheet.count()) === 0 && (await page.locator(".px-viewer").count()) === 1, "Escape closes the sheet, and the photo stays open");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
