// An event's popover stays wholly on screen as it grows (#21): opened low in the window, for
// an evening event, then given an alert (which brings a second-alert row) and notes, its foot
// and its buttons stay inside the window; a window made shorter while it is open moves it too.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1300, height: 720 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true,
  extraEvents: [{ guid: "late", calendarGuid: "work", title: "Late review", day: 0, hour: 11, length: 1 }] });
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Calendar");
await page.keyboard.press("Control+2");
await page.waitForTimeout(300);

const block = page.locator(".cx-block", { hasText: "Late review" }).first();
await block.scrollIntoViewIfNeeded();
await block.evaluate((el) => el.scrollIntoView({ block: "end" }));
await page.waitForTimeout(150);
await block.click();
await page.waitForTimeout(300);
const pop = page.locator("form.cx-popover").first();
const fits = async () => page.evaluate(() => {
  const p = document.querySelector("form.cx-popover");
  const r = p.getBoundingClientRect();
  const lastButton = [...p.querySelectorAll("button")].filter((x) => x.offsetParent).at(-1)?.getBoundingClientRect();
  return { bottom: Math.round(r.bottom), top: Math.round(r.top), last: Math.round(lastButton?.bottom ?? 0), h: innerHeight };
});
let f = await fits();
ok(await pop.isVisible() && f.bottom <= f.h - 4, `opened low in the window, it fits  (foot ${f.bottom} of ${f.h})`);

await pop.getByRole("button", { name: "Alert", exact: true }).click();
await page.getByRole("option", { name: "15 minutes before" }).click();
await page.waitForTimeout(250);
ok((await pop.getByRole("button", { name: "Second alert" }).count()) === 1, "an alert brings the second-alert row");
f = await fits();
ok(f.bottom <= f.h - 4 && f.last <= f.h - 4 && f.top >= 4, `grown, its foot and its last button stay on screen  (foot ${f.bottom}, button ${f.last} of ${f.h})`);

// Notes, several lines of them: grown again, still on screen.
const add = pop.getByText("Add Notes", { exact: true });
if (await add.count()) await add.click();
const notes = pop.locator("textarea").last();
await notes.fill("Bring the slides\nAsk about the budget\nBook the room for Friday\nSend the summary");
await page.waitForTimeout(250);
f = await fits();
ok(f.bottom <= f.h - 4 && f.last <= f.h - 4 && f.top >= 4, `with notes, still wholly on screen  (foot ${f.bottom} of ${f.h})`);

// The window made shorter while it is open.
await page.setViewportSize({ width: 1300, height: 600 });
await page.waitForTimeout(300);
f = await fits();
ok(f.bottom <= f.h - 4 && f.top >= 4, `a shorter window moves it with it  (foot ${f.bottom} of ${f.h})`);

await b.close();
process.exitCode = failures ? 1 : 0;
