// Finder's keys in Drive: the arrows walk the selection, Command/Ctrl-Down opens what is
// selected, Command/Ctrl-Up goes back to the folder it is in.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, driveFixture: true });
const page = await ctx.newPage();
await page.goto(process.env.BASE ?? process.env.URL ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Drive");
await page.waitForTimeout(400);
let failures = 0;
const ok = (pass, what) => { console.log(`${pass ? "PASS" : "FAIL"} ${what}`); if (!pass) failures++; };
const chosen = () => page.locator('.drive-row-hit[aria-pressed="true"] .drive-name').allInnerTexts();
const names = await page.locator("button.drive-row-hit .drive-name").allInnerTexts();
await page.locator("button.drive-row-hit").first().click();
await page.keyboard.press("ArrowDown");
ok(JSON.stringify(await chosen()) === JSON.stringify([names[1]]), `Down selects the next row  ${JSON.stringify(await chosen())}`);
await page.keyboard.press("ArrowUp");
ok(JSON.stringify(await chosen()) === JSON.stringify([names[0]]), "Up selects the one before");
await page.locator(".drive-row-hit", { hasText: "Downloads" }).click();
await page.keyboard.press("Control+ArrowDown");
await page.waitForTimeout(400);
ok((await page.locator(".crumb").allInnerTexts()).some((t) => t.includes("Downloads")), "Ctrl-Down opens the selected folder");
await page.locator("button.drive-row-hit").first().click();
await page.keyboard.press("Control+ArrowUp");
await page.waitForTimeout(400);
ok(!(await page.locator(".crumb").allInnerTexts()).some((t) => t.includes("Downloads")), "Ctrl-Up goes back to the folder above");

// Back and Forward, as Finder's Ctrl-[ and Ctrl-] (and Alt with the arrows, and the mouse's
// side buttons): through the folders as they were opened, not up the path.
const inDownloads = async () => (await page.locator(".crumb").allInnerTexts()).some((t) => t.includes("Downloads"));
const settle = (want) => page.waitForFunction((want) => [...document.querySelectorAll(".crumb")].some((c) => c.textContent.includes("Downloads")) === want, want, { timeout: 5000 }).catch(() => {});
await page.keyboard.press("Control+BracketLeft");
await settle(true);
ok(await inDownloads(), "Ctrl-[ goes back to the folder opened before (Downloads)");
await page.keyboard.press("Control+BracketRight");
await settle(false);
ok(!(await inDownloads()), "Ctrl-] goes forward again");
await page.keyboard.press("Alt+ArrowLeft");
await settle(true);
ok(await inDownloads(), "Alt-Left goes back too");
await page.mouse.move(600, 400);
await page.evaluate(() => window.dispatchEvent(new MouseEvent("mouseup", { button: 4, bubbles: true })));
await settle(false);
ok(!(await inDownloads()), "the mouse's Forward button goes forward");
// Nothing further forward: the key does nothing.
await page.keyboard.press("Control+BracketRight");
await page.waitForTimeout(200);
ok(!(await inDownloads()), "with nothing ahead, Ctrl-] stays where it is");

// Ctrl-D duplicates the chosen file beside itself, as Finder's Command-D.
const before = await page.evaluate(() => (window.__driveCopies ?? []).length);
const firstFile = await page.evaluate(() => {
  const rows = [...document.querySelectorAll("button.drive-row-hit")];
  const i = rows.findIndex((r) => /\.\w+$/.test(r.querySelector(".drive-name")?.textContent ?? ""));
  return i;
});
if (firstFile >= 0) {
  await page.locator("button.drive-row-hit").nth(firstFile).click();
  await page.keyboard.press("Control+d");
  await page.waitForTimeout(300);
  ok((await page.evaluate(() => (window.__driveCopies ?? []).length)) === before + 1, "Ctrl-D duplicates the chosen file");
} else ok(false, "a file to duplicate is in the fixture");
await b.close();
process.exitCode = failures ? 1 : 0;
