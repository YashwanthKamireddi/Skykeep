// Not a check: a reminder opened in its list, to look at. node look-reminder.mjs <png>
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1300, height: 820 }, colorScheme: "dark" });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
const p = await ctx.newPage();
await p.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(p, "Reminders");
await p.locator(".rx-list", { hasText: "GYM" }).click();
await p.waitForTimeout(300);
await p.getByRole("button", { name: "Details for Evening run" }).click();
await p.waitForTimeout(300);
await p.screenshot({ path: process.argv[2] ?? `${(await import("node:os")).tmpdir()}/reminder.png` });
await b.close();
