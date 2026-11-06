// The Hidden album is locked, as icloud.com locks it: opened, it shows that it is locked and
// one button to view it — nothing of it read, no tiles, no count. The button asks for the Apple
// Account password; a wrong one says so and stays locked. Viewed, it locks again (in the app as
// well as the window) when it is left, when Photos or the window goes out of sight, and at
// every start; a hidden photo open in the viewer closes as it locks.
//
//   URL=http://localhost:4173 ENGINE=webkit node .uiverify/photos-hidden-lock.mjs
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
await ctx.addInitScript(() => { window.__photoExtra = { p2: { hidden: true } }; });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? "http://localhost:4173", { waitUntil: "networkidle" });
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
await openScreen(page, "Photos");
await page.locator(".px-tile").first().waitFor();

const hiddenRow = page.locator(".px-source", { hasText: "Hidden" });
const lockedPanel = page.locator(".px-locked");
const view = page.getByRole("button", { name: "View Hidden Photos" });
const tiles = () => page.locator(".px-tile").evaluateAll((t) => t.map((x) => x.dataset.id));
const hiddenReads = () => page.evaluate(() => (window.__indexReads ?? []).filter((s) => s.view === "hidden").length);
const isLocked = async () => (await lockedPanel.count()) === 1 && (await view.count()) === 1 && !(await tiles()).includes("p2");
const openHidden = async () => { await hiddenRow.click(); await page.waitForTimeout(250); };
const sheet = page.getByRole("dialog", { name: "View Hidden Photos" });
const unlock = async () => {
  await view.click();
  await sheet.getByLabel("Password").fill("right");
  await page.keyboard.press("Enter");
  await page.locator('.px-tile[data-id="p2"]').waitFor({ timeout: 3000 });
};
const goAway = () => page.evaluate(() => {
  Object.defineProperty(document, "hidden", { value: true, configurable: true });
  Object.defineProperty(document, "visibilityState", { value: "hidden", configurable: true });
  document.dispatchEvent(new Event("visibilitychange"));
});
const comeBack = () => page.evaluate(() => {
  Object.defineProperty(document, "hidden", { value: false, configurable: true });
  Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
  document.dispatchEvent(new Event("visibilitychange"));
});

ok(!(await tiles()).includes("p2"), "a hidden photo is not in the library");
ok(await hiddenRow.locator(".px-count").count() === 0, "Hidden shows no count while it is locked");

// Opened: locked, and nothing of it read.
await openHidden();
ok(await isLocked(), "Hidden opens locked, with one button to view it");
ok(/password/i.test(await lockedPanel.innerText()), `saying what it needs  (${(await lockedPanel.innerText()).replace(/\s+/g, " ")})`);
ok(await hiddenReads() === 0, "and nothing of it is read while it is locked");

// A wrong password: said so, the field cleared for another try, and still locked.
await view.click();
await sheet.getByLabel("Password").fill("wrong");
await page.keyboard.press("Enter");
await page.waitForTimeout(250);
ok(/isn't right/.test(await sheet.innerText()) && (await sheet.getByLabel("Password").inputValue()) === "" && await hiddenReads() === 0,
  "a wrong password says so, clears the field, and reads nothing");
await page.keyboard.press("Escape");
await page.waitForTimeout(150);
ok(await sheet.count() === 0 && await isLocked(), "Escape leaves it locked");

// Viewed.
await unlock();
ok((await tiles()).includes("p2") && await lockedPanel.count() === 0, "View Hidden Photos shows them");
ok(await hiddenRow.locator(".px-count").count() === 1, "with the count beside Hidden");

// Left, and back: locked again.
await page.locator(".px-source", { hasText: "Library" }).first().click();
await page.waitForTimeout(250);
await openHidden();
ok(await isLocked(), "leaving the album locks it again");
ok(await page.evaluate(() => window.__hiddenCalls.at(-1) === "lock" && !window.__hiddenOpen), "in the app as well, not only the window");

// The window out of sight (minimised, closed to the tray): locked again.
await unlock();
await goAway();
await page.waitForTimeout(250);
ok(await page.locator(".px-tile").count() === 0 && !await page.evaluate(() => window.__hiddenOpen),
  "a hidden window releases photo images and locks Hidden in the backend before returning");
await comeBack();
await page.waitForTimeout(250);
ok(await isLocked(), "the window going out of sight locks it again");

// Another screen in front, and back to Photos: locked again.
await unlock();
await openScreen(page, "Notes");
await page.waitForTimeout(300);
await openScreen(page, "Photos");
await page.waitForTimeout(300);
await openHidden();
ok(await isLocked(), "another screen in front locks it again");

// A hidden photo open in the viewer closes as it locks.
await unlock();
await page.locator('.px-tile[data-id="p2"]').dblclick();
await page.locator(".px-viewer").waitFor({ timeout: 3000 });
await goAway();
await page.waitForTimeout(400);
ok(await page.locator(".px-viewer").count() === 0, "a hidden photo open in the viewer closes as it locks");
await comeBack();
await page.waitForTimeout(200);
ok(await isLocked(), "…and the album is locked behind it");

// Windows Hello, where it is set up: offered beside the password; declined, still locked with
// the password still there; verified, unlocked. Not set up (Linux, or no Hello): not offered.
await openScreen(page, "Photos");
await page.waitForTimeout(200);
await openHidden();
await view.click();
ok((await sheet.getByRole("button", { name: "Use Windows Hello" }).count()) === 0, "without Windows Hello, only the password is asked");
await page.keyboard.press("Escape");
await page.evaluate(() => { window.__hello = "declined"; });
await view.click();
await sheet.getByRole("button", { name: "Use Windows Hello" }).click();
await page.waitForTimeout(250);
ok(await sheet.count() === 1 && await isLocked() && (await sheet.getByLabel("Password").count()) === 1, "Windows Hello declined: still locked, the password still there");
await page.evaluate(() => { window.__hello = "verified"; });
await sheet.getByRole("button", { name: "Use Windows Hello" }).click();
await page.locator('.px-tile[data-id="p2"]').waitFor({ timeout: 3000 });
ok(await sheet.count() === 0 && (await tiles()).includes("p2"), "Windows Hello verified: Hidden opens");
await page.evaluate(() => { window.__hello = undefined; });
await page.locator(".px-source", { hasText: "Library" }).first().click();
await page.waitForTimeout(250);
await openHidden();

// A new start: locked.
await unlock();
await page.reload({ waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.locator(".px-tile").first().waitFor();
await openHidden();
ok(await isLocked(), "it is locked again at every start");

await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
