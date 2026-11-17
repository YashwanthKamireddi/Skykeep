// An event's popover never runs off a short window: it stays within the window and scrolls
// inside, so its last row can be reached.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1000, height: 420 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Calendar");
await page.keyboard.press("Control+2");
await page.waitForTimeout(500);
const block = page.locator(".cx-block").first();
await block.scrollIntoViewIfNeeded();
await block.click();
const pop = page.locator(".cx-popover").first();
await pop.waitFor({ timeout: 3000 }).catch(() => {});
const m = await pop.evaluate((el) => { const r = el.getBoundingClientRect(); return { top: r.top, bottom: r.bottom, scroll: el.scrollHeight, client: el.clientHeight, oy: getComputedStyle(el).overflowY }; });
ok(m.top >= 0 && m.bottom <= 420, `the popover stays within a 420px window  ${JSON.stringify(m)}`);
const reach = await pop.evaluate((el) => { el.scrollTop = el.scrollHeight; const last = el.lastElementChild.getBoundingClientRect(), r = el.getBoundingClientRect(); return last.bottom <= r.bottom + 1; });
ok(m.scroll <= m.client || (m.oy === "auto" && reach), "and its last row can be reached");
await b.close();
process.exitCode = failures ? 1 : 0;
