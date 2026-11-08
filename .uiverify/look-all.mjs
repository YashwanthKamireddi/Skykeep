// Not a check: every screen with fixture data, to look at. node look-all.mjs <dir> [w] [h] [scheme]
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen, openSettings } from "./stub.mjs";
const [out = (await import("node:os")).tmpdir(), w = "1400", h = "850", scheme = "dark"] = process.argv.slice(2);
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: +w, height: +h }, colorScheme: scheme });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
  calendarFixture: true, notesFixture: true, contactsFixture: true, driveFixture: true, remindersFixture: true });
const p = await ctx.newPage();
await p.goto(process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
for (const s of ["Reminders", "Notes", "Drive", "Calendar", "Contacts", "Photos", "Settings"]) {
  if (s === "Settings") await openSettings(p);
  else await openScreen(p, s);
  await p.waitForTimeout(500);
  await p.screenshot({ path: `${out}/all-${s.replace(/\W/g, "")}-${scheme}.png` });
}
await b.close();
