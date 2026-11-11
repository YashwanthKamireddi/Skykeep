// Scheduled reminders in Calendar are drawn in their list's colour, as on the Mac — ring
// and words — in Month, Week and Day; a reminder whose list has none takes the system blue.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1300, height: 1100 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true, remindersFixture: true });
const page = await ctx.newPage();
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Calendar");
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const colours = (sel) => page.evaluate((sel) => Object.fromEntries([...document.querySelectorAll(sel)].map((e) => [e.textContent.trim().slice(0, 12), getComputedStyle(e).getPropertyValue("--cal").trim()])), sel);
await page.keyboard.press("Control+3");
await page.waitForTimeout(500);
let seen = await colours(".cx-item.is-reminder");
const colourOf = (start) => Object.entries(seen).find(([k]) => k.startsWith(start))?.[1];
ok(colourOf("Evening run") === "#cc73e1" && colourOf("Water the pl") === "#63da38", `Month draws each reminder in its list's colour  ${JSON.stringify(seen)}`);
await page.keyboard.press("Control+2");
await page.waitForTimeout(500);
await page.evaluate(() => { document.querySelector(".cx-scroll").scrollTop = 4 * 48; });
seen = await colours(".cx-reminder");
ok(Object.values(seen).includes("#cc73e1"), `and Week  ${JSON.stringify(seen)}`);
const ring = await page.evaluate(() => getComputedStyle(document.querySelector(".cx-reminder .cx-ring"), "::after").backgroundColor);
ok(ring !== "rgb(85, 96, 112)" && ring !== "rgba(0, 0, 0, 0)", `its ring in that colour  ${ring}`);
await b.close();
process.exitCode = failures ? 1 : 0;
