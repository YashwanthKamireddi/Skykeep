// Settings → On This Computer: the iCloud Drive folder, off until turned on. The switch moves
// at once; once on, where it is and an Open button; one that cannot come up says why and is
// left off.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen, openSettings } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 900 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
await openSettings(page);
await page.waitForTimeout(400);
const sw = page.getByRole("switch", { name: "Drive folder" });
ok((await sw.getAttribute("aria-checked")) === "false", "off until turned on");
await sw.click();
await page.waitForTimeout(200);
ok((await sw.getAttribute("aria-checked")) === "true", "turned on");
ok(/\/home\/test\/iCloud Drive/.test(await page.locator(".settings-row", { has: sw }).innerText()), "and says where the folder is");
ok(/Up to date/.test(await page.locator(".settings-row", { has: sw }).innerText()), "up to date while nothing waits");
// What the folder has still to send, told as it changes (read every two seconds).
await page.evaluate(() => { window.__driveWaiting = { files: 2, sending: 1, failure: null }; });
await page.waitForTimeout(2300);
ok(/Uploading 2 files…/.test(await page.locator(".settings-row", { has: sw }).innerText()), "uploading, as it happens");
await page.evaluate(() => { window.__driveWaiting = { files: 1, sending: 0, failure: "iCloud is busy right now. Nothing was changed — try again in a minute." }; });
await page.waitForTimeout(2300);
const stuck = page.locator(".settings-row", { has: sw }).locator(".settings-hint");
ok(/1 file waiting — iCloud is busy/.test(await stuck.innerText()) && /is-trouble/.test(await stuck.getAttribute("class")), "and why something waits, marked");
await page.evaluate(() => { window.__driveWaiting = null; });
await page.locator(".settings-row", { has: sw }).getByRole("button", { name: "Open" }).click();
ok((await page.evaluate(() => window.__opened)).includes("drive"), "Open shows it in the file manager");
await sw.click();
await page.waitForTimeout(200);
ok((await sw.getAttribute("aria-checked")) === "false", "and off again");
await page.evaluate(() => { window.__driveFolderRefuse = "/home/test/iCloud Drive already has files in it. Choose an empty folder."; });
await sw.click();
await page.waitForTimeout(300);
const row = await page.locator(".settings-row", { has: sw }).innerText();
ok((await sw.getAttribute("aria-checked")) === "false" && /already has files in it/.test(row), `one that cannot come up is left off, and says why  "${row.replace(/\n/g, " ")}"`);
// The Photos folder: its own switch, beside Drive's, independent of it.
const ph = page.getByRole("switch", { name: "Photos folder" });
ok((await ph.getAttribute("aria-checked")) === "false" && (await sw.getAttribute("aria-checked")) === "false", "the Photos folder has its own switch, off");
await ph.click();
await page.waitForTimeout(200);
ok((await ph.getAttribute("aria-checked")) === "true" && (await sw.getAttribute("aria-checked")) === "false", "turned on alone, Drive's left as it was");
ok(/\/home\/test\/iCloud Photos/.test(await page.locator(".settings-row", { has: ph }).innerText()), "and says where it is");
await page.locator(".settings-row", { has: ph }).getByRole("button", { name: "Open" }).click();
ok((await page.evaluate(() => window.__opened)).includes("photos"), "Open shows it");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
