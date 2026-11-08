// "New Event" from outside the window — the taskbar's jump list, `skykeep --new event`
// (src-tauri/src/apps.rs): a Calendar window made for it opens with a new event started; one
// already open, even on another app, is told, comes to Calendar and starts one.
import { engine } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";

const b = await engine.launch();
let failures = 0;
const ok = (pass, msg) => { console.log(`${pass ? "PASS" : "FAIL"} ${msg}`); if (!pass) failures++; };
const started = (page) => page.locator('.cx-popover input[placeholder="New Event"]').count();
const stub = { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true, notesFixture: true };

{
  const ctx = await b.newContext({ viewport: { width: 1300, height: 850 } });
  await ctx.addInitScript(installStub, stub);
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); failures++; });
  await page.goto("http://localhost:4173/?app=calendar&new=event", { waitUntil: "networkidle" });
  await page.waitForTimeout(800);
  ok((await page.locator('main.content[data-screen="calendar"]').count()) === 1, "a window made for it is Calendar");
  ok((await started(page)) === 1, "with a new event started in it");
  await ctx.close();
}

// Calendar's own window, told again with Calendar built (apps.rs new_event): another starts.
{
  const ctx = await b.newContext({ viewport: { width: 1300, height: 850 } });
  await ctx.addInitScript(installStub, stub);
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); failures++; });
  await page.goto("http://localhost:4173/?app=calendar", { waitUntil: "networkidle" });
  await page.waitForTimeout(800);
  ok((await started(page)) === 0, "Calendar's window, opened plainly, has no event started");
  await page.evaluate(() => window.__emit("quince://new-event", null));
  await page.waitForTimeout(600);
  ok((await started(page)) === 1, "told, it starts a new event");
  await page.keyboard.press("Escape");
  await page.waitForTimeout(300);
  await page.evaluate(() => window.__emit("quince://new-event", null));
  await page.waitForTimeout(600);
  ok((await started(page)) === 1, "told again, it starts another");
  await ctx.close();
}

// Notes' own window, told: Calendar's own window is asked for, with the new event in the asking.
{
  const ctx = await b.newContext({ viewport: { width: 1300, height: 850 } });
  await ctx.addInitScript(installStub, stub);
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); failures++; });
  await page.goto("http://localhost:4173/?app=notes", { waitUntil: "networkidle" });
  await page.waitForTimeout(800);
  await page.evaluate(() => window.__emit("quince://new-event", null));
  await page.waitForTimeout(400);
  const asked = await page.evaluate(() => window.__windows ?? []);
  ok(asked.some((w) => w.cmd === "app_reveal" && w.key === "calendar" && w.reveal?.startNew), "Notes' own window hands it to Calendar's window");
  ok((await page.locator('main.content[data-screen="notes"]').count()) === 1, "and stays Notes");
  await ctx.close();
}

// Calendar's window made for that hand-off: the new event, from its address.
{
  const ctx = await b.newContext({ viewport: { width: 1300, height: 850 } });
  await ctx.addInitScript(installStub, stub);
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); failures++; });
  const asked = Buffer.from(JSON.stringify({ screen: "calendar", id: "", startNew: true })).toString("base64url");
  await page.goto(`http://localhost:4173/?app=calendar&reveal=${asked}`, { waitUntil: "networkidle" });
  await page.waitForTimeout(800);
  ok((await started(page)) === 1, "Calendar's window made for it starts the new event");
  await ctx.close();
}

await b.close();
process.exitCode = failures ? 1 : 0;
