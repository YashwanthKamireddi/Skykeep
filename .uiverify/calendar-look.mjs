// Every Calendar view at a given size, in the chosen engine, for looking at.
//   ENGINE=webkit node .uiverify/calendar-look.mjs <out-dir> [width] [height]
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
// Never into the repository: with no folder given, the system's temporary one.
const [out = (await import("node:os")).tmpdir(), w = "1920", h = "1040"] = process.argv.slice(2);
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: +w, height: +h }, colorScheme: "dark" });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
  calendarFixture: true, notesFixture: true, contactsFixture: true, driveFixture: true, remindersFixture: true });
const p = await ctx.newPage();
await p.goto(process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(p, "Calendar");
for (const v of ["Day", "Week", "Month", "Year"]) {
  await p.getByRole("button", { name: v, exact: true }).click();
  await p.waitForTimeout(400);
  await p.screenshot({ path: `${out}/cal-${v.toLowerCase()}.png` });
}
await b.close();
