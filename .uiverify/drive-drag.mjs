// Dragged onto a folder, a Drive item moves into it — gone from here at once, Apple asked to
// put it there. A file is not a place to drop, nor is an item on itself.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, driveFixture: true });
const page = await ctx.newPage();
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Drive");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const row = (name) => page.locator(".drive-rows .drive-row", { hasText: name }).first();
const names = () => page.locator("button.drive-row-hit .drive-name").allInnerTexts();

await row("Keynote").dragTo(row("Semester Plan"));
await page.waitForTimeout(200);
ok(!(await page.evaluate(() => window.__moves?.length)), "a file is not somewhere to drop");

await row("Semester Plan").dragTo(row("Downloads"));
await page.waitForTimeout(150);
ok(!(await names()).some((n) => n.startsWith("Semester Plan")), "dropped on a folder, it leaves this one at once");
const sent = await page.evaluate(() => window.__moves?.at(-1));
ok(sent?.drivewsid === "FILE::d" && sent?.destinationId === "FOLDER::a", `and Apple is asked to move it there  ${JSON.stringify(sent)}`);
await b.close();
process.exitCode = failures ? 1 : 0;
