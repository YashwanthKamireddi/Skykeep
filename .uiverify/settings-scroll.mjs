// Settings scrolls, whatever screens were opened before it. Screens stay loaded out of sight,
// and each one's layout ("the content does not scroll; the screen scrolls inside") was keyed on
// it merely being there: once any app had been opened, Settings could not scroll at all.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen, openSettings } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 600 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const settings = () => openSettings(page);
// Scrolled by the wheel, as a person scrolls: the page moves.
const scrolls = async () => {
  const box = await page.locator(".settings").boundingBox();
  await page.mouse.move(box.x + box.width / 2, 300);
  const before = await page.evaluate(() => document.querySelector(".content").scrollTop);
  await page.mouse.wheel(0, 600);
  await page.waitForTimeout(300);
  const after = await page.evaluate(() => document.querySelector(".content").scrollTop);
  await page.evaluate(() => { document.querySelector(".content").scrollTop = 0; });
  return after > before;
};
await settings();
await page.waitForTimeout(300);
ok(await scrolls(), "Settings scrolls");
for (const s of ["Notes", "Photos", "Calendar", "Drive", "Reminders", "Contacts"]) {
  await openScreen(page, s);
  await page.waitForTimeout(300);
  await settings();
  await page.waitForTimeout(300);
  ok(await scrolls(), `and still, after ${s} was opened`);
}
// And each app, back in front, lays out as its own screen again (it does not scroll the page).
await openScreen(page, "Notes");
await page.waitForTimeout(300);
ok(await page.evaluate(() => getComputedStyle(document.querySelector(".content")).overflowY === "hidden"), "Notes in front lays out as its own screen again");
await b.close();
process.exitCode = failures ? 1 : 0;
