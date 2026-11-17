// Each engine started once, and the heaviest screen drawn once, before the checks that measure
// time: the first launch after a long build reads the browser and the bundle from a cold disk,
// and its first frames were measured as the app's (photos-scroll run alone first: p50 83 ms,
// 16.7 ms warm). Checks nothing itself.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(500);
await b.close();
