// Screenshots of Settings at a size and theme: node .uiverify/shot-settings.mjs <out.png> [w] [h] [dark|light] [page]
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openSettings } from "./stub.mjs";
const [out, w = "1180", h = "820", theme = "dark", pane] = process.argv.slice(2);
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: +w, height: +h }, colorScheme: theme });
await ctx.addInitScript((st) => { window.__iphone = { state: st, cameraMs: 61.4, microphoneMs: 48.7, everyApp: "ready" }; }, process.env.IPHONE ?? "off");
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
const page = await ctx.newPage();
page.on("pageerror", (e) => console.log("page error:", e.message));
await page.goto(process.env.URL ?? "http://localhost:5273", { waitUntil: "networkidle" });
await openSettings(page);
if (pane) await page.getByRole("tab", { name: pane, exact: true }).click();
await page.waitForTimeout(500);
await page.screenshot({ path: out });
await b.close();
