// Not a check: every screen with no data, to look at its empty state. node design-empty.mjs <dir> [scheme]
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const [out = (await import("node:os")).tmpdir(), scheme = "light"] = process.argv.slice(2);
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1400, height: 850 }, colorScheme: scheme });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
const p = await ctx.newPage();
await p.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
for (const s of ["Reminders", "Notes", "Drive", "Calendar", "Contacts", "Photos"]) {
  await openScreen(p, s);
  await p.waitForTimeout(500);
  await p.screenshot({ path: `${out}/empty-${s}-${scheme}.png` });
}
await b.close();
