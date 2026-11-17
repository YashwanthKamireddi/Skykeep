// Not a check: renders the sidebar's icons large, in both themes, to look at.
import { engine } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
const out = process.argv[2] ?? (await import("node:os")).tmpdir();
const b = await engine.launch();
for (const scheme of ["dark", "light"]) {
  const ctx = await b.newContext({ viewport: { width: 1100, height: 420 }, colorScheme: scheme, deviceScaleFactor: 3 });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
  const p = await ctx.newPage();
  await p.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await p.waitForTimeout(400);
  await p.screenshot({ clip: { x: 0, y: 0, width: 230, height: 260 }, path: `${out}/icons-${scheme}.png` });
}
await b.close();
