// Calendar's Day and Week grids, checked in a real browser.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 900 } });
// The moment of the screenshot this came from: 9:51 PM, so "now" sits just above 10 PM.
await ctx.addInitScript(() => {
  const today = new Date(2026, 8, 29, 21, 51).getTime();
  const at = (h, m = 0) => Math.floor(new Date(2026, 8, 29, h, m).getTime() / 1000);
  const r = (guid, title, due) => ({ guid, listGuid: "L", title, notes: null, dueAt: due, completedAt: null,
    priority: 0, flagged: false, parentGuid: null, recurFrequency: null, recurInterval: null, etag: null });
  window.__fixedNow = today;
  window.__TAURI_INTERNALS__ = {
    transformCallback: (cb) => cb,
    invoke: async (cmd, args) => {
      if (cmd === "auth_status") return { hasStoredSession: true, signedIn: true };
      if (cmd === "auth_resume") return "library";
      if (cmd === "account_services") return ["calendar"];
      if (cmd === "calendar_list") return [{ guid: "c", title: "Home", colour: "#0088FF", ctag: null, sortOrder: 0, enabled: true, writable: true, isDefault: true }];
      if (cmd === "calendar_events") return [{ guid: "e", calendarGuid: "c", title: "New Event", location: null,
        startsAt: at(19), endsAt: at(19, 15), allDay: false, timezone: null, url: null, notes: null, etag: null }];
      if (cmd === "reminders_due_between") return [r("a", "Pick up the parcel", at(8)), r("b", "Water the plants", at(8)),
        { ...r("d", "Done already", at(16)), completedAt: at(16) }, r("s", "Study", at(19))];
      if (cmd === "calendar_sync") return { calendars: 1, events: 0 };
      if (cmd === "event_save") { (window.__saved ??= []).push(args.draft); return "E-new"; }
      if (cmd === "event_delete") throw "iCloud didn't accept that change, so nothing was saved.";
      if (cmd === "sync_all" || cmd === "sync_status") return { services: [], asOf: 1, tookMs: 0 };
      if (cmd === "plugin:event|listen") return 0;
      return [];
    },
  };
});
const page = await ctx.newPage();
await page.clock.setFixedTime(new Date(2026, 8, 29, 22, 18));
await page.goto(`${process.env.BASE ?? "http://localhost:4173"}/?app=calendar`, { waitUntil: "networkidle" });
await page.getByRole("button", { name: "Day", exact: true }).click();
await page.waitForSelector(".cal-hours.is-day");
await page.waitForTimeout(300);

let failures = 0;
const ok = (pass, yes, no) => { console.log(pass ? `PASS ${yes}` : `FAIL ${no}`); if (!pass) failures++; };

const boxes = await page.locator(".cx-reminder").evaluateAll((els) => els
  .filter((e) => /parcel|plants/.test(e.textContent))
  .map((e) => { const r = e.getBoundingClientRect(); return [r.top, r.bottom]; }));
const apart = boxes.length === 2 && (boxes[0][1] <= boxes[1][0] || boxes[1][1] <= boxes[0][0]);
ok(apart, "two reminders due at the same time are stacked, not drawn over each other",
  `the reminders at 8 overlap: ${JSON.stringify(boxes)}`);

// A reminder due during an event sits beside it, as in Calendar, not over its title.
const over = await page.evaluate(() => {
  const blocks = [...document.querySelectorAll(".cx-block:not(.is-draft)")].map((b) => b.getBoundingClientRect());
  return [...document.querySelectorAll(".cx-reminder")].filter((e) => {
    const r = e.getBoundingClientRect();
    return blocks.some((b) => r.left < b.right - 1 && r.right > b.left + 1 && r.top < b.bottom - 1 && r.bottom > b.top + 1);
  }).map((e) => e.textContent);
});
ok(over.length === 0, "a reminder due during an event is set beside it, not over it", `drawn over an event: ${JSON.stringify(over)}`);

const covered = await page.evaluate(() => {
  const now = document.querySelector(".cx-now-label")?.getBoundingClientRect();
  if (!now) return ["no now label"];
  return [...document.querySelectorAll(".cx-hour-labels span:not(.cx-now-label)")]
    .filter((s) => { const r = s.getBoundingClientRect(); return r.bottom > now.top && r.top < now.bottom; })
    .map((s) => s.textContent);
});
ok(covered.length === 0, "the time of now never sits over an hour's label", `now covers ${JSON.stringify(covered)}`);

const done = await page.locator(".cx-reminder", { hasText: "Done already" }).count();
ok(done === 0, "a completed reminder is not on the schedule", "a completed reminder is drawn on the day");

const short = await page.locator(".cx-block", { hasText: "New Event" }).evaluate((el) => {
  const box = el.getBoundingClientRect();
  const title = el.querySelector(".cx-block-title").getBoundingClientRect();
  return { box: [box.top, box.bottom], title: [title.top, title.bottom] };
});
ok(short.title[0] >= short.box[0] - 0.5 && short.title[1] <= short.box[1] + 0.5,
  "a quarter-hour event's title is whole inside its block", `the title is cut: ${JSON.stringify(short)}`);

const label = await page.evaluate(() => {
  const now = document.querySelector(".cx-now-label");
  const gutter = now.parentElement.getBoundingClientRect();
  const r = now.getBoundingClientRect();
  return { text: now.textContent, inside: r.left >= gutter.left - 0.5 && r.right <= gutter.right + 0.5, width: r.width };
});
ok(label.inside, `the time of now fits beside the grid  [${label.text}]`, `the time of now is cut: ${JSON.stringify(label)}`);

await page.getByRole("button", { name: "Week", exact: true }).click();
await page.waitForTimeout(300);
const title = (await page.locator(".cx-title").innerText()).replace(/\s+/g, " ").trim();
ok(/Sep/.test(title) && /Oct/.test(title), `a week across two months is named for both  [${title}]`,
  `the week of Sep 27 – Oct 3 is titled "${title}"`);

// Making an event, the way Calendar makes one. The grid is scrolled to a known hour first:
// where it opens depends on the clock and the window, and a press that lands outside the
// grid proves nothing.
{
  await page.evaluate(() => { document.querySelector(".cx-scroll").scrollTop = 12 * 48; });
  await page.waitForTimeout(100);
  const col = await page.locator(".cx-column").nth(3).boundingBox();
  const hour = (h) => col.y + h * 48;
  await page.mouse.click(col.x + 30, hour(14));
  await page.waitForTimeout(250);
  ok((await page.locator(".cx-popover").count()) === 0, "a single click on empty time makes nothing",
    "a single click on empty time opened a new event");

  // Held still, a press is not yet a drag: Calendar draws nothing until the pointer moves.
  await page.mouse.move(col.x + 30, hour(12));
  await page.mouse.down();
  await page.waitForTimeout(700);
  const held = await page.locator(".cx-block.is-draft").count();
  await page.mouse.up();
  await page.waitForTimeout(200);
  const after = await page.locator(".cx-block.is-draft, .cx-popover").count();
  ok(held === 0 && after === 0, "holding a press on empty time draws nothing",
    `a held press drew ${held} draft(s), ${after} left after release`);

  await page.mouse.move(col.x + 30, hour(14));
  await page.mouse.down();
  await page.mouse.move(col.x + 30, hour(15.5), { steps: 6 });
  await page.mouse.up();
  await page.waitForTimeout(300);
  const made = await page.evaluate(() => {
    const pop = document.querySelector(".cx-popover")?.getBoundingClientRect();
    const block = document.querySelector(".cx-block.is-draft")?.getBoundingClientRect();
    return { pop: pop && [pop.left, pop.right, pop.top, pop.bottom], block: block && [block.left, block.right, block.top, block.bottom] };
  });
  ok(Boolean(made.pop), "dragging over empty time makes an event and opens it", "a drag opened nothing");
  ok(Boolean(made.block), "the event being named stays on the grid where it was made", "the new event vanished from the grid");
  const covers = made.pop && made.block && made.pop[0] < made.block[1] && made.pop[1] > made.block[0]
    && made.pop[2] < made.block[3] && made.pop[3] > made.block[2];
  ok(!covers, "the popover sits beside the new event, not over it", `the popover covers it: ${JSON.stringify(made)}`);
  const fields = await page.locator(".cx-popover .cx-when input").evaluateAll((els) => els.map((e) => (e.classList.contains("time-field") ? "time" : e.type)));
  ok(JSON.stringify(fields) === JSON.stringify(["date", "time", "date", "time"]),
    "a start and an end, each a date and a time", `the popover's fields are ${JSON.stringify(fields)}`);
  await page.keyboard.press("Escape");
}

// A press whose release never arrives — what the app's WebKitGTK did — must not leave a
// half-made event on the grid growing with every move of the mouse.
{
  await page.evaluate(() => { document.querySelector(".cx-scroll").scrollTop = 12 * 48; });
  await page.waitForTimeout(100);
  const col = await page.locator(".cx-column").nth(2).boundingBox();
  // A real press; the release is swallowed before the app can hear it.
  await page.evaluate(() => {
    const swallow = (e) => e.stopImmediatePropagation();
    window.__swallow = swallow;
    window.addEventListener("pointerup", swallow, true);
    window.addEventListener("mouseup", swallow, true);
  });
  await page.mouse.move(col.x + 30, col.y + 13 * 48);
  await page.mouse.down();
  await page.mouse.up();
  // The mouse moves on with no button held.
  await page.mouse.move(col.x + 30, col.y + 17 * 48, { steps: 8 });
  await page.waitForTimeout(250);
  await page.evaluate(() => {
    window.removeEventListener("pointerup", window.__swallow, true);
    window.removeEventListener("mouseup", window.__swallow, true);
  });
  const left = await page.evaluate(() => ({
    drafts: document.querySelectorAll(".cx-block.is-draft").length,
    popovers: document.querySelectorAll(".cx-popover").length,
  }));
  ok(left.drafts === 0 && left.popovers === 0, "a press whose release is lost leaves nothing behind",
    `a lost release left ${JSON.stringify(left)}`);
}

// The whole of making an event with New Event (Ctrl+N; the + opens Quick Event, its own
// suite): it opens, takes a title, and is saved with what was typed — the thing that has to
// work for Calendar to be usable at all.
{
  await page.evaluate(() => document.activeElement?.blur());
  await page.keyboard.press("Control+n");
  await page.waitForTimeout(250);
  const open = await page.locator(".cx-popover").count();
  ok(open === 1, "New Event opens a new event", "New Event opened nothing");
  await page.keyboard.type("Dinner with Asha");
  await page.getByRole("button", { name: "Add", exact: true }).click();
  await page.waitForTimeout(400);
  const saved = await page.evaluate(() => window.__saved ?? []);
  const good = saved.length === 1 && saved[0].title === "Dinner with Asha" &&
    saved[0].endsAt - saved[0].startsAt === 3600 && saved[0].calendarGuid === "c";
  ok(good, "what was typed is saved: its title, an hour long, in the default calendar",
    `saved ${JSON.stringify(saved)}`);
  ok((await page.locator(".cx-popover").count()) === 0, "and the popover closes once it is saved",
    "the popover stayed open after saving");
}

// In the morning, at the size of a full-screen window: the week opens scrolled to just
// before now, which puts an hour line at the very top — where its label was cut in half —
// and a tall window is where Year spread its months apart.
{
  const tall = await ctx.newPage();
  await tall.setViewportSize({ width: 1920, height: 1040 });
  await tall.clock.setFixedTime(new Date(2026, 8, 29, 8, 30));
  await tall.goto(`${process.env.BASE ?? "http://localhost:4173"}/?app=calendar`, { waitUntil: "networkidle" });
  await tall.getByRole("button", { name: "Week", exact: true }).click();
  await tall.waitForTimeout(400);
  const first = await tall.evaluate(() => {
    const scroll = document.querySelector(".cx-scroll").getBoundingClientRect();
    const labels = [...document.querySelectorAll(".cx-hour-labels span:not(.cx-now-label)")]
      .map((s) => ({ text: s.textContent, top: s.getBoundingClientRect().top, bottom: s.getBoundingClientRect().bottom }))
      .filter((l) => l.bottom > scroll.top);
    return { label: labels[0], edge: scroll.top };
  });
  ok(first.label && first.label.top >= first.edge - 0.5, `the first hour in view is shown whole  [${first.label?.text}]`,
    `the first hour in view is cut: ${JSON.stringify(first)}`);

  await tall.getByRole("button", { name: "Year", exact: true }).click();
  await tall.waitForTimeout(250);
  const gap = await tall.evaluate(() => {
    // The days drawn, not the boxes around them: a stretched row stretches its boxes too.
    const months = [...document.querySelectorAll(".cal-mini")];
    const firstRowBottom = Math.max(...months.slice(0, 4).map((m) => m.querySelector(".cx-minimonth-grid").getBoundingClientRect().bottom));
    return Math.min(...months.slice(4, 8).map((m) => m.querySelector("h2").getBoundingClientRect().top)) - firstRowBottom;
  });
  ok(gap <= 40, `Year's rows of months sit together  [${gap.toFixed(0)}px apart]`, `Year's rows are ${gap.toFixed(0)}px apart`);
  await tall.close();
}

// An event's bottom edge, dragged, changes when it ends — its start stays where it was.
{
  const p = await ctx.newPage();
  await p.goto(`${process.env.BASE ?? "http://localhost:4173"}/?app=calendar`, { waitUntil: "networkidle" });
  await p.getByRole("button", { name: "Week", exact: true }).click();
  await p.waitForTimeout(300);
  const grip = p.locator(".cx-block", { hasText: "New Event" }).first().locator(".cx-block-grip");
  const box = await grip.boundingBox();
  if (!box) {
    ok(false, "", "the event has no edge to drag");
  } else {
    await p.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await p.mouse.down();
    await p.mouse.move(box.x + box.width / 2, box.y + box.height / 2 + 48, { steps: 6 });
    await p.mouse.up();
    await p.waitForTimeout(300);
    const saved = await p.evaluate(() => (window.__saved ?? []).at(-1));
    const start = Math.floor(new Date(2026, 8, 29, 19, 0).getTime() / 1000);
    ok(saved && saved.startsAt === start && saved.endsAt === start + 75 * 60,
      "dragging an event's bottom edge changes its end, not its start",
      `saved: ${JSON.stringify(saved && { startsAt: saved.startsAt - start, endsAt: saved.endsAt - start })}`);
  }
  await p.close();
}

// A delete iCloud turns down says so in a sentence, with a way to put it away — not a red
// line of the request and Apple's JSON under the toolbar.
{
  const p = await ctx.newPage();
  await p.goto(`${process.env.BASE ?? "http://localhost:4173"}/?app=calendar`, { waitUntil: "networkidle" });
  await p.getByRole("button", { name: "Week", exact: true }).click();
  await p.waitForTimeout(300);
  await p.locator(".cx-block", { hasText: "New Event" }).first().click();
  await p.waitForTimeout(250);
  await p.locator(".cx-pop-delete").click();
  await p.waitForTimeout(300);
  const notice = await p.evaluate(() => {
    const n = document.querySelector('[role="alert"]');
    return n && { text: n.textContent.trim(), dismiss: Boolean(n.querySelector('button[aria-label="Dismiss"]')) };
  });
  ok(notice?.text === "iCloud didn't accept that change, so nothing was saved." && notice.dismiss,
    "a refused delete says so in a sentence, with Dismiss", `the notice: ${JSON.stringify(notice)}`);
  // Delete, pressed with an event open and no field in use, deletes it once confirmed.
  if (notice?.dismiss) {
    await p.getByRole("button", { name: "Dismiss" }).click();
    await p.locator(".cx-block", { hasText: "New Event" }).first().click();
    await p.waitForTimeout(250);
    await p.evaluate(() => document.activeElement?.blur());
    await p.keyboard.press("Delete");
    await p.getByRole("alertdialog").getByRole("button", { name: "Delete" }).click();
    await p.waitForTimeout(300);
    ok((await p.locator('[role="alert"]').count()) === 1, "Delete deletes an open event, as in Calendar",
      "Delete did nothing to the open event");
  }
  if (notice?.dismiss) {
    await p.getByRole("button", { name: "Dismiss" }).click();
    await p.waitForTimeout(150);
    ok((await p.locator('[role="alert"]').count()) === 0, "and Dismiss puts it away", "Dismiss left the notice up");
  }
  await p.close();
}

// A small window: the title is shortened, never cut, and a month cell with room for a line
// shows an event in it rather than only "N more".
{
  // The shared fixture — a real week's worth of events and reminders, with an all-day bar.
  const full = await b.newContext({ viewport: { width: 800, height: 600 } });
  await full.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
    calendarFixture: true, notesFixture: true, contactsFixture: true, driveFixture: true, remindersFixture: true });
  const small = await full.newPage();
  await small.setViewportSize({ width: 800, height: 600 });
  // A Wednesday whose week crosses into the next month, so Week's title is its longest.
  await small.clock.setFixedTime(new Date(2026, 8, 30, 18, 0));
  await small.goto(`${process.env.BASE ?? "http://localhost:4173"}/?app=calendar`, { waitUntil: "networkidle" });
  await openScreen(small, "Calendar");
  for (const view of ["Week", "Month"]) {
  await small.getByRole("button", { name: view, exact: true }).click();
  await small.waitForTimeout(400);
  const narrow = await small.evaluate(() => {
    const t = document.querySelector(".cx-title");
    const views = document.querySelector(".cx-tools").getBoundingClientRect();
    const tr = t.getBoundingClientRect();
    // A cell that shows only "N more" though no all-day bar crosses it gave its room away.
    const bars = [...document.querySelectorAll(".cx-bar-event")].map((b) => b.getBoundingClientRect());
    const starved = [...document.querySelectorAll(".cx-day")].filter((d) => {
      if (!d.querySelector(".cx-more") || d.querySelector(".cx-item")) return false;
      const r = d.getBoundingClientRect();
      return !bars.some((b) => b.left < r.right - 1 && b.right > r.left + 1 && b.top < r.bottom && b.bottom > r.top);
    });
    return {
      cut: t.scrollWidth > t.clientWidth + 1,
      // The scopes and the stepping share a row; the title sits under them, as on a Mac.
      oneRow: (() => { const seg = document.querySelector(".cx-bar > .segmented").getBoundingClientRect();
        return Math.abs((seg.top + seg.bottom) / 2 - (views.top + views.bottom) / 2) < 8 && tr.top >= seg.bottom - 1; })(),
      text: t.innerText.trim(),
      starved: starved.length,
    };
  });
  ok(!narrow.cut, `${view}: a narrow calendar's title is whole  [${narrow.text}]`, `${view}: the title is cut: ${JSON.stringify(narrow)}`);
  ok(narrow.oneRow, `${view}: its controls stay one row, the title on its own beneath`, `${view}: the controls broke onto two rows`);
  if (view === "Month")
    ok(narrow.starved === 0, "a month cell with room shows an event, not only \"N more\"",
      `${narrow.starved} cell(s) show only "N more"`);
  }

  // New Event at six: the new event, 7 to 8, crosses the 7:30 Dentist — it takes a column beside it,
  // and its popover opens by it, never over the toolbar.
  await small.getByRole("button", { name: "Week", exact: true }).click();
  await small.waitForTimeout(300);
  await small.evaluate(() => document.activeElement?.blur());
  await small.keyboard.press("Control+n");
  await small.waitForTimeout(300);
  const placed = await small.evaluate(() => {
    const hit = (a, b) => a.left < b.right - 1 && a.right > b.left + 1 && a.top < b.bottom - 1 && a.bottom > b.top + 1;
    const draft = document.querySelector(".cx-block.is-draft")?.getBoundingClientRect();
    const pop = document.querySelector(".cx-popover")?.getBoundingClientRect();
    const bar = document.querySelector(".cx-bar").getBoundingClientRect();
    const others = [...document.querySelectorAll(".cx-block:not(.is-draft)")].map((b) => b.getBoundingClientRect());
    return { draft: Boolean(draft), over: draft ? others.filter((o) => hit(draft, o)).length : -1, onBar: pop ? hit(pop, bar) : null };
  });
  ok(placed.draft && placed.over === 0, "a new event is set beside the event it overlaps", `the new event: ${JSON.stringify(placed)}`);
  ok(placed.onBar === false, "its popover opens clear of the toolbar", `the popover: ${JSON.stringify(placed)}`);
  await small.keyboard.press("Escape");
  await full.close();
}

await b.close();
process.exitCode = failures === 0 ? 0 : 1;
