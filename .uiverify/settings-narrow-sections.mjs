// Settings in a narrow window (a third of a screen, tiled): the sections in one row along the
// top, each its symbol and the open one its name too, all of them inside the window — none cut
// off at the edge, as "G…" was — every one still named for screen readers and as its tooltip,
// and each opening its section. Light and dark, 495 and 420 wide, and 300 (too narrow even for
// the symbols: the row scrolls and the first ones stay reachable).
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openSettings } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const shots = process.env.SHOTS;
const NAMES = ["Home", "Backup", "This Computer", "Notifications", "General", "Privacy", "Software Update"];
for (const [scheme, width] of [["dark", 495], ["light", 495], ["light", 420], ["dark", 300]]) {
  const at = `${scheme} ${width}px`;
  const ctx = await b.newContext({ viewport: { width, height: 700 }, colorScheme: scheme });
  await ctx.addInitScript(() => { window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} }; });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `${at}: page error: ${e.message}`));
  await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await openSettings(page);
  await page.waitForSelector(".prefs-side");
  await page.waitForTimeout(200);
  const nav = page.locator(".prefs-side");
  const box = await nav.boundingBox();
  const tabs = nav.getByRole("tab");
  ok((await tabs.count()) === NAMES.length, `${at}: every section is there`);
  for (const name of NAMES) {
    const tab = nav.getByRole("tab", { name, exact: true });
    ok((await tab.count()) === 1 && (await tab.getAttribute("title")) === name, `${at}: ${name} keeps its name and tooltip`);
  }
  if (width >= 420) {
    ok((await nav.evaluate((e) => e.scrollWidth <= e.clientWidth + 1)), `${at}: the row fits, nothing scrolled out of sight`);
    for (const name of NAMES) {
      const t = await nav.getByRole("tab", { name, exact: true }).boundingBox();
      ok(t && t.x >= box.x - 0.5 && t.x + t.width <= box.x + box.width + 0.5, `${at}: ${name} is inside the window`);
    }
  } else {
    await nav.evaluate((e) => { e.scrollLeft = 0; });
    const first = await nav.getByRole("tab", { name: "Home", exact: true }).boundingBox();
    ok(first && first.x >= box.x - 0.5, `${at}: too narrow even for the symbols, the first stays reachable`);
  }
  // What is drawn, not the text kept for screen readers: a name hidden from sight is 1px wide.
  const drawn = (tab) => tab.locator(".source-name").evaluate((e) => e.getBoundingClientRect().width > 4);
  const open = nav.getByRole("tab", { selected: true });
  ok((await open.getAttribute("title")) === "Home" && await drawn(open), `${at}: the open section shows its name`);
  ok(!(await drawn(nav.getByRole("tab", { name: "Privacy", exact: true }))), `${at}: the others show their symbol only`);
  if (shots) await page.screenshot({ path: `${shots}/narrow-${scheme}-${width}.png`, clip: { x: 0, y: 0, width, height: 260 } });
  await nav.getByRole("tab", { name: "Privacy", exact: true }).click();
  await page.waitForTimeout(150);
  const now = nav.getByRole("tab", { selected: true });
  ok((await now.getAttribute("title")) === "Privacy" && await drawn(now) && await page.locator("#pane-privacy").isVisible(), `${at}: a symbol opens its section, named now`);
  await ctx.close();
}
await b.close();
if (failures) { console.log(`${failures} failed`); process.exit(1); }
console.log("all passed");
