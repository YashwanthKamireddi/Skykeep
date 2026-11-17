// A calendar or contacts file opened with Skykeep shows what it holds and adds it only on
// confirm, through the same commands the apps use (event_save, contacts_import). The quick
// capture window is one field: Enter makes the reminder or note and the window closes.
import { engine } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";

// Against `npx vite preview --port 4173 --strictPort`; BASE points elsewhere when that port is taken.
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

// What intake.rs hands a window, shaped as it serializes it.
const DAY = 86400;
const base = Math.floor(Date.UTC(2026, 10, 5, 9, 0, 0) / 1000);
const ARRIVALS = {
  events: {
    kind: "events", file: "family.ics",
    events: [
      { title: "Design review", location: "Studio", startsAt: base, endsAt: base + 5400, allDay: false,
        timezone: "Europe/Berlin", notes: null, url: null, alerts: [15], repeat: null, duplicate: false },
      { title: "Gym", location: null, startsAt: base + DAY, endsAt: base + DAY + 3600, allDay: false,
        timezone: "Asia/Kolkata", notes: null, url: null, alerts: null,
        repeat: { freq: "weekly", interval: 1, count: 10 }, duplicate: false },
      { title: "Holiday in Goa", location: null, startsAt: base + 40 * DAY, endsAt: base + 43 * DAY, allDay: true,
        timezone: "Asia/Kolkata", notes: null, url: null, alerts: null, repeat: null, duplicate: false },
      { title: "Dentist", location: null, startsAt: base + 2 * DAY, endsAt: base + 2 * DAY + 1800, allDay: false,
        timezone: "Asia/Kolkata", notes: null, url: null, alerts: null, repeat: null, duplicate: true },
    ],
    refused: [{ title: "Standup", reason: "Its repeat skips some dates, which Skykeep can’t set up." }],
  },
  contacts: {
    kind: "contacts", file: "people.vcf", path: "/home/test/Downloads/people.vcf",
    contacts: [{ name: "Ravi Kumar", detail: "+91 98765 43210", known: false },
               { name: "Zoya Ahuja", detail: "+91 90000 00001", known: true }],
  },
};

// The intake commands, layered over the shared stub.
const intakeStub = ({ arrivals }) => {
  const wrap = () => {
    const t = window.__TAURI_INTERNALS__;
    if (!t || t.__intake) return;
    const inner = t.invoke;
    window.__pending = arrivals.slice();
    t.invoke = async (c, a) => {
      (window.__calls ??= []).push([c, a]);
      switch (c) {
        case "intake_pending": { const out = window.__pending; window.__pending = []; return out; }
        case "intake_capture_close": window.__closed = (window.__closed ?? 0) + 1; return null;
        case "intake_open_main": return null;
        case "window_ready": return null;
        case "sync_some": return { services: [], asOf: null, tookMs: 1 };
        case "reminder_create": (window.__creates ??= []).push(a); return null;
        case "note_create": (window.__notes ??= []).push(a); return "NOTE-1";
        case "event_save": (window.__eventSaves ??= []).push(a.draft); return "E-new";
      }
      return inner(c, a);
    };
    t.__intake = true;
  };
  wrap();
  document.addEventListener("DOMContentLoaded", wrap);
};

async function page(url, arrivals, stub = {}) {
  const ctx = await b.newContext({ viewport: { width: 1040, height: 700 } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, ...stub });
  await ctx.addInitScript(intakeStub, { arrivals });
  const p = await ctx.newPage();
  p.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); failures++; });
  await p.goto(url, { waitUntil: "networkidle" });
  await p.waitForTimeout(500);
  return { ctx, p };
}

// ── An .ics file: the events, the calendar picker, the refusals; Add writes each one. ──
{
  const { ctx, p } = await page(`${BASE}/?app=calendar`, [ARRIVALS.events], { calendarFixture: true });
  const sheet = p.getByRole("dialog", { name: "Import “family.ics”" });
  ok(await sheet.isVisible(), "an opened .ics shows the import sheet");
  ok(await sheet.getByRole("heading", { name: "Add 3 events to" }).isVisible(), "headed with how many will be added (the duplicate left out)");
  const picker = sheet.getByRole("button", { name: /Calendar/ });
  const pickerText = await picker.innerText().catch(() => "");
  ok(pickerText.length > 0, `with a calendar picker on the account's default  (${pickerText.trim()})`);
  const rows = sheet.locator(".ix-list li");
  ok((await rows.count()) === 5, "every event is listed, refused ones too");
  ok(await sheet.getByText("Every week, 10 times").isVisible(), "a repeat is spelled out");
  ok(await sheet.getByText("Already in your calendar").isVisible(), "a duplicate is marked");
  ok(await sheet.getByText(/Can’t be added\. Its repeat skips some dates/).isVisible(), "a refusal says why");
  ok(await sheet.getByText(/\(Europe\/Berlin\)/).isVisible(), "an event in another zone says its zone");
  await p.screenshot({ path: ".uiverify/intake-events.png" });

  // Choose another calendar, then add.
  await picker.click();
  const offered = (await p.getByRole("option").allInnerTexts()).map((t) => t.replace("✓", "").trim());
  ok(!offered.some((t) => /Holidays|Birthdays/.test(t)) && offered.length > 1, `only calendars that take events are offered  ${JSON.stringify(offered)}`);
  const option = p.getByRole("option", { selected: false }).first();
  const chosen = (await option.innerText()).replace("✓", "").trim();
  await option.click();
  await sheet.getByRole("button", { name: "Add 3 Events" }).click();
  await p.waitForTimeout(400);
  const saves = await p.evaluate(() => window.__eventSaves ?? []);
  const cals = await p.evaluate(() => window.__cals ?? []);
  const target = cals.find((c) => c.title === chosen)?.guid;
  ok(saves.length === 3 && saves.every((s) => s.guid === null && s.calendarGuid === target),
    `three new events saved through event_save into "${chosen}"  ${JSON.stringify(saves.map((s) => s.calendarGuid))}`);
  ok(saves[0].alerts?.[0] === 15 && saves[0].timezone === "Europe/Berlin" && !("repeat" in saves[0]), "alerts and zone carried, no repeat where none");
  ok(saves[1].repeatChanged === true && saves[1].repeat?.count === 10, "the repeat is passed through");
  ok(saves[2].allDay === true, "all-day stays all-day");
  ok(!saves.some((s) => s.title === "Dentist"), "the duplicate is not added again");
  ok(!(await sheet.isVisible()), "the sheet closes once added");
  ok((await p.evaluate(() => (window.__calls ?? []).some(([c]) => c === "sync_some"))), "and the calendar is refreshed");
  await ctx.close();
}

// ── Cancel sends nothing. ──
{
  const { ctx, p } = await page(`${BASE}/?app=calendar`, [ARRIVALS.events], { calendarFixture: true });
  await p.getByRole("button", { name: "Cancel" }).click();
  await p.waitForTimeout(200);
  ok((await p.evaluate(() => (window.__eventSaves ?? []).length)) === 0 && !(await p.locator(".ix-sheet").count()), "Cancel closes the sheet and adds nothing");
  await ctx.close();
}

// ── A .vcf file, arriving while the window is already open. ──
{
  const { ctx, p } = await page(`${BASE}/?app=contacts`, [], { contactsFixture: true });
  ok((await p.locator(".ix-sheet").count()) === 0, "no sheet until a file arrives");
  await p.evaluate((a) => { window.__pending = [a]; window.__emit("quince://intake", null); }, ARRIVALS.contacts);
  await p.waitForTimeout(300);
  const sheet = p.getByRole("dialog", { name: "Import “people.vcf”" });
  ok(await sheet.getByRole("heading", { name: "Add contact Ravi Kumar" }).isVisible(), "a file forwarded to the open window: \"Add contact Ravi Kumar\"");
  ok(await sheet.getByText("Already in Contacts").isVisible(), "someone already there is marked");
  await p.screenshot({ path: ".uiverify/intake-contacts.png" });
  await sheet.getByRole("button", { name: "Add Contact" }).click();
  await p.waitForTimeout(300);
  const imports = await p.evaluate(() => window.__imports ?? []);
  ok(imports.length === 1 && imports[0].paths[0] === "/home/test/Downloads/people.vcf", "added through contacts_import with the file");
  ok(!(await sheet.isVisible()), "and the sheet closes");
  await ctx.close();
}

// ── A file that can't be read. ──
{
  const { ctx, p } = await page(`${BASE}/?app=calendar`,
    [{ kind: "unreadable", file: "broken.ics", reason: "This isn’t a calendar file." }], { calendarFixture: true });
  ok(await p.getByText("This isn’t a calendar file.").isVisible(), "an unreadable file says why");
  await p.getByRole("button", { name: "OK" }).click();
  ok((await p.locator(".ix-sheet").count()) === 0, "and OK puts it away");
  await ctx.close();
}

// ── Quick capture: a reminder into the list last added to. ──
{
  const ctx = await b.newContext({ viewport: { width: 460, height: 132 } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true, notesFixture: true });
  await ctx.addInitScript(intakeStub, { arrivals: [] });
  await ctx.addInitScript(() => { try { localStorage.setItem("quince.reminders.defaultList", "l2"); } catch {} });
  const p = await ctx.newPage();
  p.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); failures++; });
  await p.goto(`${BASE}/?capture=reminder`, { waitUntil: "networkidle" });
  await p.waitForTimeout(300);
  const field = p.getByRole("textbox", { name: "New Reminder" });
  ok(await field.isVisible() && await field.evaluate((el) => el === document.activeElement), "the capture window opens on its one field, focused");
  ok((await p.locator("nav.sidebar").count()) === 0, "and nothing of the library");
  ok((await p.getByRole("button", { name: /List/ }).innerText()).includes("GYM"), "into the list last added to");
  ok(await p.evaluate(() => (window.__calls ?? []).some(([c]) => c === "window_ready")), "shown once drawn (window_ready)");
  await field.fill("Call the bank");
  await p.screenshot({ path: ".uiverify/intake-capture.png" });
  await field.press("Enter");
  await p.waitForTimeout(300);
  const made = await p.evaluate(() => window.__creates ?? []);
  ok(made.length === 1 && made[0].listGuid === "l2" && made[0].title === "Call the bank", `saved with reminder_create  ${JSON.stringify(made)}`);
  ok((await p.evaluate(() => window.__closed)) === 1, "and the window closes");

  // Asked for again as a note: the same window turns into a note.
  await p.evaluate(() => window.__emit("quince://capture", "note"));
  await p.waitForTimeout(300);
  const note = p.getByRole("textbox", { name: "New Note" });
  ok(await note.isVisible(), "asked for a note, the window becomes a note capture");
  ok((await p.getByRole("button", { name: /Folder/ }).innerText()).includes("Notes"), "into Notes");
  await note.fill("Ideas for the trip");
  await note.press("Enter");
  await p.waitForTimeout(300);
  const notes = await p.evaluate(() => window.__notes ?? []);
  ok(notes.length === 1 && notes[0].folderGuid === null && notes[0].body === "Ideas for the trip", `saved with note_create into the default folder  ${JSON.stringify(notes)}`);
  // Escape leaves without making anything. (Closed for real, the window is gone; here it is
  // asked for again, which also gives a fresh field.)
  await p.evaluate(() => window.__emit("quince://capture", "note"));
  await p.waitForTimeout(200);
  ok(await note.inputValue() === "" && await note.isEnabled(), "asked for again, the field is fresh");
  await note.fill("never mind");
  await note.press("Escape");
  await p.waitForTimeout(150);
  ok((await p.evaluate(() => window.__closed)) === 3 && (await p.evaluate(() => window.__notes.length)) === 1, "Escape closes without saving");
  await ctx.close();
}

// ── Quick capture before Skykeep is signed in. ──
{
  const ctx = await b.newContext({ viewport: { width: 460, height: 132 } });
  await ctx.addInitScript(installStub, { screen: "signin", services: SERVICES, twoFactor: null, remindersFixture: true });
  await ctx.addInitScript(intakeStub, { arrivals: [] });
  const p = await ctx.newPage();
  await p.goto(`${BASE}/?capture=reminder`, { waitUntil: "networkidle" });
  await p.waitForTimeout(300);
  const field = p.getByRole("textbox", { name: "New Reminder" });
  await field.fill("Pay rent");
  await field.press("Enter");
  await p.waitForTimeout(300);
  ok(await p.getByText("Sign in to Skykeep first.").isVisible() && (await p.evaluate(() => (window.__creates ?? []).length)) === 0,
    "signed out, it says so and writes nothing");
  await p.getByRole("button", { name: "Open Skykeep" }).click();
  ok(await p.evaluate(() => (window.__calls ?? []).some(([c]) => c === "intake_open_main")), "and Open Skykeep brings up the sign-in");
  await ctx.close();
}

await b.close();
process.exitCode = failures ? 1 : 0;
