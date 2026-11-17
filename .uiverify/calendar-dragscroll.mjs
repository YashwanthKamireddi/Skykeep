// Dragging an event in Week to an hour out of sight, as in Calendar: held near the bottom of
// the hours they scroll, and the hours scrolled past — by the edge or by the wheel — count as
// hours moved.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1300, height: 700 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
const page = await ctx.newPage();
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Calendar");
await page.keyboard.press("Control+2");
const scroller = page.locator(".cx-scroll");
await scroller.waitFor();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const saves = () => page.evaluate(() => window.__eventSaves ?? []);
const ten = await page.evaluate(() => { const d = new Date(); d.setHours(10, 0, 0, 0); return d.getTime() / 1000; });
// Design review, 10 to 11, at the top of the hours in view.
await scroller.evaluate((el) => { el.scrollTop = 9 * 48; });
const review = page.locator(".cx-block", { hasText: "Design review" });

// The wheel while it is held: two hours scrolled are two hours moved.
let at = await review.boundingBox();
await page.mouse.move(at.x + at.width / 2, at.y + 20);
await page.mouse.down();
await page.mouse.move(at.x + at.width / 2, at.y + 24, { steps: 2 });
await scroller.evaluate((el) => { el.scrollTop += 2 * 48; });
await page.waitForFunction(() => {
  const block = [...document.querySelectorAll(".cx-block")].find((x) => x.textContent.includes("Design review"));
  return block && parseFloat(block.style.top) >= 12 * 48;
}, null, { timeout: 2000 }).catch(() => {});
await page.mouse.up();
await page.waitForFunction(() => (window.__eventSaves ?? []).length >= 1, null, { timeout: 3000 }).catch(() => {});
let s = await saves();
ok(s.length === 1 && s[0].guid === "e1" && s[0].startsAt === ten + 2 * 3600, `scrolled two hours while held, it moves two hours  ${JSON.stringify(s.map((x) => (x.startsAt - ten) / 3600))}`);

// Held at the bottom edge, the hours scroll and the event goes with them.
await page.reload({ waitUntil: "networkidle" });
await openScreen(page, "Calendar");
await scroller.waitFor();
await scroller.evaluate((el) => { el.scrollTop = 9 * 48; });
const box = await scroller.boundingBox();
at = await review.boundingBox();
const top = await scroller.evaluate((el) => el.scrollTop);
await page.mouse.move(at.x + at.width / 2, at.y + 20);
await page.mouse.down();
await page.mouse.move(at.x + at.width / 2, at.y + 40, { steps: 3 });
await page.mouse.move(at.x + at.width / 2, box.y + box.height - 6, { steps: 6 });
await page.waitForFunction((t) => document.querySelector(".cx-scroll").scrollTop > t + 3 * 48, top, { timeout: 4000 }).catch(() => {});
const scrolledTo = await scroller.evaluate((el) => el.scrollTop);
ok(scrolledTo > top + 3 * 48, `the hours scroll while it is held at the edge  [${Math.round(scrolledTo - top)}px]`);
await page.mouse.up();
await page.waitForFunction(() => (window.__eventSaves ?? []).length >= 1, null, { timeout: 3000 }).catch(() => {});
s = await saves();
// Where it was let go: the pointer's place in the hours, less where it was taken from.
const expected = Math.round(((box.y + box.height - 6 + scrolledTo) - (at.y + 20 + top)) / 48 * 4) / 4;
const got = s[0] ? (s[0].startsAt - ten) / 3600 : null;
ok(s.length === 1 && got !== null && got >= 3 && Math.abs(got - expected) <= 0.25, `it lands where it was let go  [${got} h, expected ~${expected} h]`);

// A press on an event near the edge, the hand not quite still, scrolls nothing.
await page.reload({ waitUntil: "networkidle" });
await openScreen(page, "Calendar");
await scroller.waitFor();
// Design review's top half at the bottom edge of the hours.
await scroller.evaluate((el) => { el.scrollTop = 10 * 48 + 30 - el.clientHeight; });
const still = await scroller.evaluate((el) => el.scrollTop);
at = await review.boundingBox();
await page.mouse.move(at.x + at.width / 2, at.y + 10);
await page.mouse.down();
await page.mouse.move(at.x + at.width / 2 + 1, at.y + 11);
await page.waitForTimeout(300);
await page.mouse.up();
ok(await scroller.evaluate((el) => el.scrollTop) === still, "a press held still near the edge scrolls nothing");
await page.keyboard.press("Escape");

await b.close();
console.log(failures ? `${failures} failed` : "all passed");
process.exitCode = failures ? 1 : 0;
