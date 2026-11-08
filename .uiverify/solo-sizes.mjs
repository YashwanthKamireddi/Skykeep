// Each app's own window at every size from the 480px minimum up: screenshots for review.
import { engine } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
const BASE = process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
for (const app of ["notes", "reminders", "calendar", "contacts", "drivews", "photos"]) {
  for (const [w, h] of [[480, 600], [700, 600], [900, 640]]) {
    const ctx = await b.newContext({ viewport: { width: w, height: h } });
    await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
    const page = await ctx.newPage();
    await page.goto(`${BASE}/?app=${app}`, { waitUntil: "networkidle" });
    await page.waitForTimeout(1200);
    await page.screenshot({ path: `/tmp/size-${app}-${w}.png` });
    await ctx.close();
  }
}
await b.close();
