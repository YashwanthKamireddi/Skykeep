// Screenshots of the calendar for looking at by eye: `node .uiverify/shots.mjs <tag> [scheme]`,
// written to $SHOTS (a temporary folder by default). A tool, not a suite.
import os from "node:os";
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const S = process.env.SHOTS ?? os.tmpdir();
const tag = process.argv[2] || "before";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1300, height: 850 }, colorScheme: process.argv[3] || "light" });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => console.log("pageerror", e.message));
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Calendar");
for (const v of ["Month", "Week", "Day", "Year"]) {
  await page.getByRole("button", { name: v, exact: true }).click();
  await page.waitForTimeout(400);
  await page.screenshot({ path: `${S}/${tag}-${engineName}-${v}.png` });
}
await page.getByRole("button", { name: "Week", exact: true }).click();
await page.waitForTimeout(300);
await page.locator(".cx-block", { hasText: "Design review" }).first().click();
await page.waitForTimeout(300);
await page.screenshot({ path: `${S}/${tag}-${engineName}-pop.png` });
await b.close();
