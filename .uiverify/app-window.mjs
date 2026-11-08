// A window opened as one app (`?app=…`, from its launcher entry or the tray) is that app alone,
// as a Mac's Notes is: no switcher of the six, the app's own list as the window's sidebar with
// the account and sync status at its foot, Settings opened in the main window, and another app
// asked for from here opened in its own window. The main window keeps the switcher.
//
//   BASE=http://localhost:4173 ENGINE=webkit node .uiverify/app-window.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
const BASE = process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };
const fixtures = { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true, calendarFixture: true, remindersFixture: true };

const APPS = [
  ["notes", ".notes-folders"],
  ["reminders", ".rx-side"],
  ["calendar", ".cx-side"],
  ["contacts", ".contacts-lists"],
  ["photos", ".px-side"],
  ["drivews", null],
];
for (const [app, side] of APPS) {
  const ctx = await b.newContext({ viewport: { width: 1200, height: 760 } });
  await ctx.addInitScript(installStub, fixtures);
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `${app}: page error: ${e.message}`));
  await page.goto(`${BASE}/?app=${app}`, { waitUntil: "networkidle" });
  await page.locator(`main.content[data-screen="${app}"]`).waitFor({ timeout: 5000 });
  ok((await page.locator("nav.sidebar").count()) === 0, `${app}: no switcher of the six`);
  if (side) {
    const foot = page.locator(`${side} .app-foot`);
    ok(await foot.isVisible(), `${app}: the account sits at the foot of the app's own sidebar`);
    // The card reaches 8px from the window's top and foot, as the main window's sidebar does.
    const box = await page.locator(side).boundingBox();
    ok(box && Math.abs(box.y - 8) <= 1 && Math.abs(760 - (box.y + box.height) - 8) <= 1, `${app}: its sidebar is a card 8px in (${box && `${box.y}, ${760 - box.y - box.height}`})`);
    await foot.locator(".sidebar-account").click();
    const asked = await page.evaluate(() => window.__windows ?? []);
    ok(asked.some((w) => w.cmd === "settings_open"), `${app}: its account opens Settings in the main window`);
    ok((await page.locator(`main.content[data-screen="${app}"]`).count()) === 1, `${app}: and the window stays on ${app}`);
  }
  // Ctrl+, as a Mac app's Settings: the main window's, not this one's.
  await page.evaluate(() => { window.__windows = []; });
  await page.keyboard.press("Control+Comma");
  await page.waitForTimeout(200);
  ok((await page.evaluate(() => window.__windows ?? [])).some((w) => w.cmd === "settings_open")
    && (await page.locator(`main.content[data-screen="${app}"]`).count()) === 1, `${app}: Ctrl+, opens Settings in the main window`);
  await ctx.close();
}

// Calendar's Show in Reminders, from Calendar's own window: Reminders' window, told what to show.
{
  const ctx = await b.newContext({ viewport: { width: 1200, height: 760 } });
  await ctx.addInitScript(installStub, fixtures);
  const page = await ctx.newPage();
  await page.goto(`${BASE}/?app=calendar`, { waitUntil: "networkidle" });
  await page.locator('main.content[data-screen="calendar"]').waitFor();
  // As CalendarView's Show in Reminders asks for the screen.
  await page.evaluate(() => window.dispatchEvent(new CustomEvent("quince:revealed", { detail: { screen: "reminders" } })));
  await page.waitForTimeout(300);
  const asked = await page.evaluate(() => window.__windows ?? []);
  ok(asked.some((w) => (w.cmd === "app_reveal" || w.cmd === "app_open") && w.key === "reminders"), "another app asked for from an app's window opens in its own window");
  ok((await page.locator('main.content[data-screen="calendar"]').count()) === 1, "and Calendar's window stays Calendar");
  await ctx.close();
}

// A window made for a request finds it in its address (apps.rs app_reveal).
{
  const ctx = await b.newContext({ viewport: { width: 1200, height: 760 } });
  await ctx.addInitScript(installStub, fixtures);
  const page = await ctx.newPage();
  const asked = Buffer.from(JSON.stringify({ screen: "notes", id: "nope" })).toString("base64url");
  page.on("pageerror", (e) => ok(false, `reveal in the address: page error: ${e.message}`));
  await page.goto(`${BASE}/?app=notes&reveal=${asked}`, { waitUntil: "networkidle" });
  await page.locator('main.content[data-screen="notes"]').waitFor({ timeout: 5000 });
  ok(true, "a request in the address opens the app without fault");
  await ctx.close();
}

// The main window: the switcher of the six, and no app foot.
{
  const ctx = await b.newContext({ viewport: { width: 1200, height: 760 } });
  await ctx.addInitScript(installStub, fixtures);
  const page = await ctx.newPage();
  await page.goto(`${BASE}/`, { waitUntil: "networkidle" });
  await page.locator("nav.sidebar").waitFor();
  ok(await page.locator("nav.sidebar").isVisible(), "the main window keeps the switcher of the six");
  ok((await page.locator(".app-foot").count()) === 0, "and no app's own foot");
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
