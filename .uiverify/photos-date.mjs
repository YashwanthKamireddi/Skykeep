// Adjust Date and Time, as icloud.com's: from Info's Adjust, the photo, the zone the times are
// in, the original, and the new date to the second; Save only once it differs; the photo moves
// to its new month at once; Escape closes the sheet and nothing else.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 }, timezoneId: "Asia/Kolkata" });
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
await page.locator(".px-adjust").click();
const sheet = page.getByRole("dialog", { name: "Adjust Date and Time" });
ok((await sheet.count()) === 1 && /All times are GMT\+05:30/.test(await sheet.innerText()), "the sheet says what zone the times are in");
ok(await page.getByRole("button", { name: "Save" }).isDisabled(), "Save waits for a different date");
await page.getByLabel("Adjusted:").fill("2025-03-14T09:30:15");
ok((await page.evaluate(() => (window.__flags ?? []).length)) === 0, "typing in it touches nothing else");
await page.getByRole("button", { name: "Save" }).click();
await page.waitForTimeout(300);
const sent = await page.evaluate(() => window.__dates?.at(-1));
ok(sent?.id === "p2" && sent.takenAt === Date.UTC(2025, 2, 14, 4, 0, 15) / 1000, `Save sends the new moment  ${JSON.stringify(sent)}`);
ok((await sheet.count()) === 0 && (await page.locator(".px-viewer-when strong").innerText()) === "March 14, 2025", "the photo shows its new date at once");
await page.keyboard.press("Escape");
await page.waitForTimeout(650);
ok((await page.locator(".px-month-title").allInnerTexts()).includes("March 2025"), "and stands in its new month");
await page.locator(".px-tile").nth(0).dblclick();
await page.waitForTimeout(600);
await page.keyboard.press("i");
await page.locator(".px-adjust").click();
await page.keyboard.press("Escape");
await page.waitForTimeout(150);
ok((await sheet.count()) === 0 && (await page.locator(".px-viewer").count()) === 1, "Escape closes the sheet, the photo stays open");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
