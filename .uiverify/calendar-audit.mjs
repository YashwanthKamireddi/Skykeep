// What the Calendar audit fixed, each checked in the engine: dragging upward to make an event,
// an event carried only to days on screen, a drag drawn per quarter hour rather than per pixel,
// a popover dismissed by a click elsewhere keeping what was typed, the bare arrows paging,
// Day's bars running on past the day, back-to-back short events side by side, the title set
// as a title, Saturday's hairline, and today moving on at midnight even after a sleep.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const open = async (opts = {}) => {
  const ctx = await b.newContext({ viewport: { width: 1300, height: 850 }, locale: "en-US" });
  await ctx.addInitScript(() => {
    window.__commits = 0;
    window.__REACT_DEVTOOLS_GLOBAL_HOOK__ = { supportsFiber: true, renderers: new Map(), inject() { return 1; },
      onCommitFiberRoot() { window.__commits++; }, onCommitFiberUnmount() {}, onPostCommitFiberRoot() {}, checkDCE() {}, onScheduleFiberRoot() {} };
  });
  if (opts.extra) await ctx.addInitScript(opts.extra);
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
  const page = await ctx.newPage();
  if (opts.clock) await page.clock.install({ time: opts.clock });
  await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await openScreen(page, "Calendar");
  return { ctx, page };
};

{
  const { ctx, page } = await open({ extra: () => {
    // Two quarter-hour calls back to back, tomorrow at 15:00 and 15:15.
    const d = new Date(); d.setDate(d.getDate() + 1); d.setHours(15, 0, 0, 0);
    window.__extraEvents = [0, 1].map((i) => ({ guid: `q${i}`, calendarGuid: "work", title: `Quick call ${i}`, location: null,
      startsAt: d.getTime() / 1000 + i * 900, endsAt: d.getTime() / 1000 + (i + 1) * 900, allDay: false, timezone: null, url: null, notes: null, etag: null }));
  } });
  await page.evaluate(() => { const t = window.__TAURI_INTERNALS__; const inner = t.invoke;
    t.invoke = async (c, a) => { const r = await inner(c, a); return c === "calendar_events" && Array.isArray(r) ? [...r, ...(window.__extraEvents ?? [])] : r; }; });
  await page.getByRole("button", { name: "Week", exact: true }).click();
  await page.getByRole("button", { name: "Month", exact: true }).click();
  await page.getByRole("button", { name: "Week", exact: true }).click();
  await page.waitForTimeout(500);

  // Back-to-back quarter hours: each drawn 20px tall, so laid side by side, never overlapping.
  const q = await page.evaluate(() => ["q0", "q1"].map((g) => document.querySelector(`[data-event="${g}"]`)?.getBoundingClientRect()).map((r) => r && { top: r.top, bottom: r.bottom, left: r.left, right: r.right }));
  const overlap = q[0] && q[1] && q[0].bottom > q[1].top && q[0].right > q[1].left && q[1].right > q[0].left;
  ok(q[0] && q[1] && !overlap, `two back-to-back 15-minute events are not drawn over each other  ${JSON.stringify(q)}`);

  // Drag upward from 4 PM to 2 PM in an empty day: an event from 2 to 4.15.
  await page.locator(".cx-scroll").evaluate((el) => { el.scrollTop = 11 * 48; });
  const col = await page.locator(".cx-columns").boundingBox();
  const colW = col.width / 7;
  const yOf = async (h) => (await page.locator(".cx-columns").boundingBox()).y + h * 48;
  const x = col.x + colW * 1 + colW / 2;
  await page.mouse.move(x, await yOf(16) + 2);
  await page.mouse.down();
  const c0 = await page.evaluate(() => window.__commits);
  for (let i = 1; i <= 96; i++) await page.mouse.move(x, (await yOf(16)) + 2 - i);
  const commits = (await page.evaluate(() => window.__commits)) - c0;
  await page.mouse.up();
  await page.waitForTimeout(250);
  ok(commits <= 12, `a 96px drag redraws once per quarter hour, not per pixel  [${commits} commits]`);
  const starts = await page.getByLabel("Starts time").inputValue().catch(() => "");
  const ends = await page.getByLabel("Ends time").inputValue().catch(() => "");
  ok(/^2:00/.test(starts) && /^4:15/.test(ends), `dragging upward makes the event above the press  [${starts} – ${ends}]`);

  // A title typed, then a click elsewhere: the event is made with it.
  await page.getByLabel("Title").fill("Typed then clicked away");
  await page.evaluate(() => { window.__eventSaves = []; });
  await page.mouse.click(col.x + 20, col.y + 10 * 48);
  await page.waitForTimeout(300);
  const saved = await page.evaluate(() => window.__eventSaves.map((d) => d.title));
  ok(saved.includes("Typed then clicked away"), `a click outside keeps what was typed  ${JSON.stringify(saved)}`);
  // Escape still leaves an event as it was.
  await page.locator(".cx-block", { hasText: "Design review" }).first().click();
  await page.waitForTimeout(200);
  await page.getByLabel("Title").fill("Not this");
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);
  ok(!(await page.evaluate(() => window.__eventSaves.some((d) => d.title === "Not this"))), "Escape still discards an edit");
  // Nor is an untouched new event made by clicking away.
  await page.mouse.dblclick(col.x + colW * 2 + 10, (await yOf(13)) + 5);
  await page.waitForTimeout(200);
  const before = await page.evaluate(() => window.__eventSaves.length);
  await page.mouse.click(col.x + 10, (await yOf(18)) + 5);
  await page.waitForTimeout(200);
  const after = [await page.evaluate(() => window.__eventSaves.length) - before, await page.locator(".cx-popover").count()];
  ok(after[0] === 0 && after[1] === 0,
    `an untouched new event is let go by a click elsewhere  [${after}]`);

  // An event carried past the last day shown stays on the week: Saturday at most.
  const block = page.locator(".cx-block", { hasText: "Design review" }).first();
  const bb = await block.boundingBox();
  await page.evaluate(() => { window.__eventSaves = []; });
  await page.mouse.move(bb.x + 10, bb.y + 5);
  await page.mouse.down();
  await page.mouse.move(bb.x + 10 + colW * 12, bb.y + 5, { steps: 10 });
  await page.mouse.up();
  await page.waitForTimeout(300);
  const moved = await page.evaluate(() => window.__eventSaves[0]);
  const sat = await page.evaluate(() => { const d = new Date(); d.setDate(d.getDate() + (6 - d.getDay())); d.setHours(10, 0, 0, 0); return d.getTime() / 1000; });
  ok(moved && moved.startsAt === sat, `an event dragged off the week lands on its last day  [${moved && new Date(moved.startsAt * 1000).toString().slice(0, 21)}]`);

  // The bare arrows page, as ⌘-arrows do; T comes back.
  await page.mouse.click(5, 5);
  const title0 = await page.locator(".cx-title").getAttribute("aria-label");
  const firstHead0 = await page.locator(".cal-col-head").first().getAttribute("aria-label");
  await page.keyboard.press("ArrowRight");
  await page.waitForTimeout(150);
  const firstHead1 = await page.locator(".cal-col-head").first().getAttribute("aria-label");
  ok(firstHead1 !== firstHead0, `→ steps to the next week  [${firstHead0} → ${firstHead1}]`);
  await page.keyboard.press("ArrowLeft");
  await page.keyboard.press("ArrowLeft");
  await page.keyboard.press("t");
  await page.waitForTimeout(150);
  ok((await page.locator(".cal-col-head").first().getAttribute("aria-label")) === firstHead0, "← steps back and T returns to this week");

  // Month: Saturday ends the grid without a hairline in every week, bars or not.
  await page.getByRole("button", { name: "Month", exact: true }).click();
  await page.waitForTimeout(250);
  const borders = await page.evaluate(() => [...document.querySelectorAll(".cx-week")].map((w) => getComputedStyle(w.children[6]).borderRightWidth));
  ok(borders.every((x) => x === "0px"), `no week keeps a hairline on Saturday's right  ${JSON.stringify(borders)}`);

  // The popover's title is a title: 17px bold, not the body's text.
  await page.getByRole("button", { name: "Week", exact: true }).click();
  await page.locator(".cx-block", { hasText: "Design review" }).first().click();
  await page.waitForTimeout(200);
  const font = await page.getByLabel("Title").evaluate((el) => [getComputedStyle(el).fontSize, getComputedStyle(el).fontWeight]);
  ok(font[0] === "17px" && Number(font[1]) >= 700, `the popover's title is set as a title  ${JSON.stringify(font)}`);
  await page.keyboard.press("Escape");

  // Day: a trip running on past the day is drawn running on, square at its right end.
  await page.getByRole("button", { name: "Day", exact: true }).click();
  await page.evaluate(() => { const d = new Date(); d.setDate(d.getDate() + 3); window.__day = d; });
  for (let i = 0; i < 3; i++) await page.keyboard.press("ArrowRight");
  await page.waitForTimeout(250);
  const bar = await page.locator(".cx-allday .cx-bar-event", { hasText: "Flight" }).first().getAttribute("class").catch(() => "");
  ok(/to-after/.test(bar ?? ""), `Day draws a bar that runs on past the day as running on  [${bar}]`);
  await ctx.close();
}

{
  // A minute before midnight; then the machine sleeps past it, and the window is come back to.
  const late = new Date(); late.setHours(23, 59, 0, 0);
  const { ctx, page } = await open({ clock: late });
  await page.getByRole("button", { name: "Week", exact: true }).click();
  await page.waitForTimeout(300);
  const today = () => page.evaluate(() => document.querySelector(".cal-col-head.is-today")?.getAttribute("aria-label"));
  const t0 = await today();
  await page.clock.fastForward("01:30");
  const t1 = await today();
  ok(t0 && t1 && t0 !== t1, `today moves on at midnight  [${t0} → ${t1}]`);
  // Asleep: the clock jumps a day without a timer firing; coming back reads it again.
  await page.clock.setSystemTime(new Date(late.getTime() + 2 * 86400e3 + 3600e3));
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await page.waitForTimeout(100);
  const t2 = await today();
  ok(t2 !== t1, `coming back to the window after a sleep moves today on  [${t1} → ${t2}]`);
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
