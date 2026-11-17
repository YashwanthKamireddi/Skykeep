// Info's place and keywords, as icloud.com's Info has them: where it was taken (the coordinates
// Apple keeps, and the map opened at them); its keywords as chips, × to take one away, a word and
// Return to add one — the letters typed are words, not the photo's keys.
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

await page.locator(".px-tile").nth(1).dblclick();
await page.waitForTimeout(600);
await page.keyboard.press("i");
ok((await page.locator(".px-place span").innerText()) === "17.7654° N, 83.3576° E", `the place it was taken  ${await page.locator(".px-place span").innerText()}`);
await page.getByRole("button", { name: "Open in Map" }).click();
await page.waitForTimeout(100);
ok((await page.evaluate(() => window.__maps?.at(-1))) === "p2", "Open in Map opens it there");
ok((await page.locator(".px-chip").allInnerTexts()).join() === "beach", "its keywords, as chips");
await page.getByLabel("Add a Keyword").click();
await page.keyboard.type("ferry");
ok((await page.evaluate(() => (window.__flags ?? []).length)) === 0, "the letters typed are words, not the photo's keys");
await page.keyboard.press("Enter");
await page.waitForTimeout(200);
ok(JSON.stringify(await page.evaluate(() => window.__keywords?.at(-1))) === JSON.stringify({ id: "p2", keywords: ["beach", "ferry"] }),
  "Return adds the word, and it goes to iCloud");
ok((await page.locator(".px-chip").allInnerTexts()).join() === "beach,ferry", "shown at once");
await page.getByRole("button", { name: "Remove beach" }).click();
await page.waitForTimeout(200);
ok(JSON.stringify(await page.evaluate(() => window.__keywords?.at(-1))) === JSON.stringify({ id: "p2", keywords: ["ferry"] }), "× takes one away");
await page.locator(".px-tile").count();
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
