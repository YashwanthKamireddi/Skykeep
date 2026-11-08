// Sign Out asks, as macOS does: keep a copy here, or remove it. What has not reached iCloud
// yet is said plainly; Cancel and Escape change nothing; the choice reaches the backend as
// asked, and the window starts over.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openSettings } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 900 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const settings = () => openSettings(page);
await settings();
await page.waitForTimeout(400);
const signOut = page.getByRole("button", { name: "Sign Out" });
const sheet = page.getByRole("alertdialog");

await signOut.click();
ok(await sheet.isVisible(), "Sign Out asks first");
ok(/Keep a copy of your iCloud data on this computer\?/.test(await sheet.innerText()), "whether to keep a copy");
ok((await sheet.locator(".signout-unsent").count()) === 0, "nothing said about unsent changes when there are none");
ok(await page.evaluate(() => document.activeElement?.textContent === "Keep a Copy"), "Keep a Copy has the focus: Return never removes");
await page.keyboard.press("Escape");
ok(!(await sheet.isVisible()), "Escape cancels");
await signOut.click();
await sheet.getByRole("button", { name: "Cancel" }).click();
ok(!(await sheet.isVisible()) && (await page.evaluate(() => sessionStorage.getItem("__signedOut"))) === null, "Cancel signs nothing out");

await page.evaluate(() => { window.__unsent = { changes: 2, files: 1, photos: 0 }; });
await signOut.click();
const said = await sheet.locator(".signout-unsent").innerText();
ok(/2 changes and 1 file haven't reached iCloud yet/.test(said) && /lost/.test(said), `what would be lost is said  "${said}"`);
await Promise.all([page.waitForEvent("load"), sheet.getByRole("button", { name: "Remove from This Computer" }).click()]);
ok(JSON.parse(await page.evaluate(() => sessionStorage.getItem("__signedOut")))?.keep === false, "Remove asks the backend to keep nothing");

await page.evaluate(() => sessionStorage.removeItem("__signedOut"));
await page.waitForTimeout(300);
await settings();
await page.waitForTimeout(400);
await signOut.click();
await Promise.all([page.waitForEvent("load"), page.keyboard.press("Enter")]);
ok(JSON.parse(await page.evaluate(() => sessionStorage.getItem("__signedOut")))?.keep === true, "Return keeps a copy");

await b.close();
if (failures) process.exitCode = 1;
