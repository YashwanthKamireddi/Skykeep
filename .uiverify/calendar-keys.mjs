// Calendar's Mac shortcuts, Ctrl for ⌘: 1–4 switch the view, T goes to today, N starts an event.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1300, height: 850 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Calendar");
await page.waitForTimeout(300);
let failures = 0;
const ok = (pass, msg) => { console.log(`${pass ? "PASS" : "FAIL"} ${msg}`); if (!pass) failures++; };
for (const [key, name] of [["1", "Day"], ["2", "Week"], ["3", "Month"], ["4", "Year"]]) {
  await page.keyboard.press(`Control+${key}`);
  await page.waitForTimeout(200);
  const now = await page.evaluate(() => [...document.querySelectorAll("button")].find((x) =>
    ["Day", "Week", "Month", "Year"].includes(x.textContent.trim()) &&
    (x.getAttribute("aria-pressed") === "true" || x.getAttribute("aria-selected") === "true" || x.className.includes("is-"))
  )?.textContent.trim());
  ok(now === name, `Ctrl+${key} shows ${name}  [${now}]`);
}
await page.keyboard.press("Control+2");
await page.waitForTimeout(200);
await page.keyboard.press("Control+n");
await page.waitForTimeout(400);
ok(await page.locator(".cx-block.is-draft").count() === 1, "Ctrl+N starts a new event");
// Control-Option-Down moves the open event fifteen minutes later, and saves it.
await page.keyboard.press("Escape");
await page.waitForTimeout(200);
await page.evaluate(() => { window.__nudges = []; const t = window.__TAURI_INTERNALS__; const inner = t.invoke;
  t.invoke = async (c, a) => { if (c === "event_save") window.__nudges.push(a.draft); return inner(c, a); }; });
await page.locator(".cx-block", { hasText: "Design review" }).first().click();
await page.waitForTimeout(250);
await page.evaluate(() => document.activeElement?.blur());
await page.keyboard.press("Control+Alt+ArrowDown");
await page.waitForTimeout(300);
const sent = await page.evaluate(() => window.__nudges);
// Design review starts at the fixture's 10 in the morning, local time.
const was = await page.evaluate(() => { const d = new Date(); d.setHours(10, 0, 0, 0); return d.getTime() / 1000; });
ok(sent.length === 1 && sent[0].guid === "e1" && sent[0].startsAt === was + 900,
  `Ctrl+Option+Down moves the event 15 minutes later  ${JSON.stringify(sent.map((d) => d.startsAt - was))}`);
// A time is typed as said, and stepped with the arrows.
await page.keyboard.press("Escape");
await page.waitForTimeout(200);
await page.evaluate(() => { window.__nudges = []; });
await page.locator(".cx-block", { hasText: "Design review" }).first().click();
await page.waitForTimeout(250);
const start = page.getByLabel(/^Starts time$/i).first();
await start.fill("4:30pm");
await start.press("Enter");
await page.waitForTimeout(150);
const typed = await start.inputValue();
ok(/4:30/.test(typed) && /PM|16/i.test(typed), `"4:30pm" typed into a time is read  [${typed}]`);
await start.press("ArrowUp");
await page.waitForTimeout(150);
ok(/4:45/.test(await start.inputValue()), `the arrows step it fifteen minutes  [${await start.inputValue()}]`);
await b.close();
process.exitCode = failures ? 1 : 0;
