// Not a check: Calendar's Week and Day views, to look at. node look-calendar.mjs <dir>
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const out = process.argv[2] ?? (await import("node:os")).tmpdir();
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1400, height: 850 }, colorScheme: "dark" });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
const p = await ctx.newPage();
await p.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(p, "Calendar");
for (const [k, n] of [["2", "week"], ["1", "day"], ["4", "year"]]) {
  await p.keyboard.press(`Control+${k}`);
  await p.waitForTimeout(400);
  await p.screenshot({ path: `${out}/cal-${n}.png` });
}
await b.close();
