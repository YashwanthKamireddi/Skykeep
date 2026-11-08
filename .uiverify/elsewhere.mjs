import { openScreen } from "./stub.mjs";
// Everything this app was built and checked against was one machine in one place: Asia/
// Kolkata, English, a clock that has never moved. Most people are somewhere else.
//
// This opens every screen as somebody who is — a German speaker in New York whose clocks
// change twice a year, a Japanese speaker in a zone that never changes, and a New Zealander
// on the Chathams, whose clock moves by forty-five minutes — and asserts the app renders,
// says true things, and never lays a date over its own label.
import { engine as chromium } from "./engine.mjs";

const PLACES = [
  { label: "New York, German", timezoneId: "America/New_York", locale: "de-DE" },
  { label: "Tokyo, Japanese", timezoneId: "Asia/Tokyo", locale: "ja-JP" },
  { label: "Chatham, English", timezoneId: "Pacific/Chatham", locale: "en-NZ" },
];

const stub = ({ services, now, edge }) => {
  const DAY = 86_400;
  const lists = [{ guid: "L1", title: "Daily", colour: "#c8912a", ctag: null, sortOrder: 0, enabled: true }];
  const items = [
    { guid: "R1", listGuid: "L1", title: "Tomorrow evening", notes: null,
      dueAt: now + DAY, completedAt: null, priority: 0, flagged: false, parentGuid: null,
      recurFrequency: 0, recurInterval: 1, etag: "t1" },
    { guid: "R2", listGuid: "L1", title: "Next week", notes: null,
      dueAt: now + 7 * DAY, completedAt: null, priority: 0, flagged: false, parentGuid: null,
      recurFrequency: null, recurInterval: null, etag: "t2" },
  ];
  if (edge) {
    items.push({ guid: "R3", listGuid: "L1", title: "Just after midnight", notes: null,
      dueAt: edge, completedAt: null, priority: 0, flagged: false, parentGuid: null,
      recurFrequency: null, recurInterval: null, etag: "t3" });
  }
  const calendars = [{ guid: "c1", title: "Work", colour: "#cc73e1", ctag: null, sortOrder: 0,
                       enabled: true, writable: true, isDefault: true }];
  const events = [{ guid: "e1", calendarGuid: "c1", title: "Standup", location: null,
                    startsAt: now + 3600, endsAt: now + 7200, allDay: false,
                    timezone: null, url: null, notes: null, etag: null }];
  window.__TAURI_INTERNALS__ = {
    transformCallback: (cb) => cb,
    invoke: async (cmd) => {
      switch (cmd) {
        case "auth_status": return { hasStoredSession: true, signedIn: true };
        case "auth_resume": return "library";
        case "account_services": return services;
        case "reminder_counts": return { today: 0, scheduled: 2, flagged: 0, all: 2, lists: [["L1", 2]] };
        case "reminder_lists": return lists;
        case "reminders_in": case "reminders_smart": return items;
        case "reminders_due_between": return items;
        case "calendar_list": return calendars;
        case "calendar_events": return events;
        case "calendar_sync": return { calendars: 1, events: 1 };
        case "sync_all": case "sync_status": return { services: [], asOf: now, tookMs: 0 };
        case "plugin:event|listen": return 0;
        case "note_folders": case "notes_in": case "contacts_list": case "drive_children":
          return [];
        case "drive_sync_folder": return 0;
        case "note_body": return "";
        default: return null;
      }
    },
  };
};

const SERVICES = ["account", "calendar", "ckdatabasews", "contacts", "docws", "drivews",
                  "notes", "reminders"];

const browser = await chromium.launch();
let failures = 0;

for (const place of PLACES) {
  const ctx = await browser.newContext({
    viewport: { width: 1040, height: 900 },
    timezoneId: place.timezoneId,
    locale: place.locale,
  });
  await ctx.addInitScript(stub, { services: SERVICES, now: Math.floor(Date.now() / 1000), edge: 0 });
  const page = await ctx.newPage();
  const blew = [];
  page.on("pageerror", (e) => blew.push(String(e)));
  await page.goto("http://localhost:4173", { waitUntil: "networkidle" });

  for (const screen of ["Reminders", "Calendar", "Notes", "Contacts", "Drive"]) {
    await openScreen(page, screen);
    await page.waitForTimeout(300);
  }

  // Something due tomorrow must not be called today. Counting days in milliseconds gets this
  // wrong on the two days a year a clock moves.
  await openScreen(page, "Reminders");
  await page.waitForTimeout(400);
  const said = (await page.locator(".rx-rows").allInnerTexts()).join(" ");
  const muddled = /Tomorrow evening[\s\S]{0,60}\bToday\b/.test(said);
  console.log(
    muddled
      ? `FAIL ${place.label}: something due tomorrow was called today`
      : `PASS ${place.label}: every screen renders, and tomorrow is not today`
  );
  if (muddled) failures++;

  // A date in a longer language must not spill out of the row that holds it.
  const spilling = await page.evaluate(() =>
    [...document.querySelectorAll(".rx-rows li, .rem-list, .rx-list, .crumb, .rx-tile")].filter(
      (el) => el.scrollWidth > el.clientWidth + 2
    ).length
  );
  console.log(
    spilling === 0
      ? `PASS ${place.label}: nothing overflows its row`
      : `FAIL ${place.label}: ${spilling} element(s) overflow`
  );
  if (spilling > 0) failures++;

  console.log(
    blew.length === 0
      ? `PASS ${place.label}: raises nothing`
      : `FAIL ${place.label}: ${blew.join(" | ")}`
  );
  if (blew.length > 0) failures++;

  await page.close();
  await ctx.close();
}

// ── The day a clock actually moves ────────────────────────────────────────────────────
// On 8 March 2026 New York has a twenty-three hour day. Something due half an hour after
// midnight is then 23.5 hours away, and a day counted by dividing milliseconds by 86,400,000
// rounds that down to zero and calls it today. Counting dates instead gets it right.
{
  const ctx = await browser.newContext({
    viewport: { width: 1040, height: 900 },
    timezoneId: "America/New_York",
    locale: "en-US",
  });
  // 2026-03-08 20:00 in New York, on the short day itself.
  const now = Math.floor(Date.UTC(2026, 2, 9, 0, 0, 0) / 1000);
  // Only the clock, not the timers — installing those freezes the scheduler the interface
  // renders on, and an empty screen would pass this check for the wrong reason.
  await ctx.clock.setFixedTime(new Date(now * 1000));
  // 00:30 on the 9th — inside the hour that tomorrow starts early, because today was only
  // twenty-three hours long. Exactly 23.5 hours after midnight, which a day counted in
  // milliseconds rounds down to nothing and calls today.
  const edge = Math.floor(Date.UTC(2026, 2, 9, 4, 30, 0) / 1000);
  await ctx.addInitScript(stub, { services: SERVICES, now, edge });
  const page = await ctx.newPage();
  await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
  await openScreen(page, "Reminders");
  await page.waitForTimeout(400);

  // A reminder's title is an editable field, so its text is a value rather than content —
  // reading innerText alone finds the due label and none of the titles.
  const tomorrow = await page.evaluate(() => {
    for (const row of document.querySelectorAll(".rx-rows li")) {
      const title = row.querySelector("input")?.value ?? "";
      if (title.includes("Just after midnight")) return row.innerText;
    }
    return "";
  });
  const wrong = /\bToday\b/.test(tomorrow);
  const found = tomorrow.length > 0;
  console.log(
    !found
      ? "FAIL the short day: the row never rendered, so this checked nothing"
      : wrong
        ? `FAIL the short day: something due tomorrow was called today  [${tomorrow.replace(/\n/g, " ")}]`
        : `PASS the short day: tomorrow is still tomorrow  [${tomorrow.replace(/\n/g, " ").slice(0, 60)}]`
  );
  if (wrong || !found) failures++;
  await page.close();
  await ctx.close();
}

await browser.close();
process.exitCode = failures === 0 ? 0 : 1;
