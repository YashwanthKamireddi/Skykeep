// The appearance chosen in Settings reaches the windows' own title bars, not just the page: the
// app is asked for it as each window starts, and again with each choice (appearance.rs).
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openSettings } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1180, height: 820 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
const asks = () => page.evaluate(() => window.__themeAsks ?? []);
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openSettings(page);
ok(JSON.stringify(await asks()) === '["auto"]', `as it starts, the system's appearance is asked for  ${JSON.stringify(await asks())}`);

const appearance = page.getByRole("group", { name: "Appearance" });
for (const [label, theme] of [["Dark", "dark"], ["Light", "light"], ["Automatic", "auto"]]) {
  await page.evaluate(() => { window.__themeAsks = []; });
  await appearance.getByRole("button", { name: label, exact: true }).click();
  await page.waitForTimeout(200);
  const asked = await asks();
  ok(JSON.stringify(asked) === JSON.stringify([theme]), `${label}: every window's title bar asked to be ${theme}  ${JSON.stringify(asked)}`);
  const marked = await page.evaluate(() => document.documentElement.dataset.theme ?? "auto");
  ok(marked === theme, `${label}: the page is ${theme} too  (${marked})`);
}

// A window opened later (an app's own, after the choice) says the stored choice as it starts.
await appearance.getByRole("button", { name: "Dark", exact: true }).click();
const app = await ctx.newPage();
await app.goto(`${process.env.URL ?? process.env.BASE ?? "http://localhost:4173"}/?app=notes`, { waitUntil: "networkidle" });
const first = await app.evaluate(() => window.__themeAsks ?? []);
ok(JSON.stringify(first) === '["dark"]', `a window opened after Dark was chosen asks for dark as it starts  ${JSON.stringify(first)}`);
await b.close();
process.exitCode = failures ? 1 : 0;
