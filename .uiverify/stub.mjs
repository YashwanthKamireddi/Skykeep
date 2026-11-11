// The IPC stub and sample data the browser suites share: the shapes the Rust commands
// return, with sample rows so every screen has something real-shaped to draw.
export const SERVICES = ["account","archivews","calendar","ckdatabasews","ckdeviceservice",
                  "cksharews","contacts","contactsweb","developerapi","docws","drivews",
                  "findme","geows","iwmb","iworkexportws","iworkthumbnailws","keyvalue",
                  "mail","mcc","mccgateway","mpp","notes","photos","photosupload",
                  "premiummailsettings","push","reminders","settings","sharedlibrary",
                  "sharedstreams","ubiquity","uploadimagews"];

/* addInitScript SERIALIZES this function — closures do not cross into the page, so every
   value it needs must arrive through the single argument. */
/* Fixture events, so the agenda's rendering is verified without inventing rows in anyone's
   real library. Times are unix seconds relative to now. */
export function installStub({ screen, services, twoFactor, calendarFixture, notesFixture, contactsFixture, driveFixture, remindersFixture, demoFixture, extraEvents = [] }) {
  const HOUR = 3600, DAY = 86400;
  // Today at 9 in the morning, on this machine's clock. Counted from UTC's midnight it was
  // yesterday for the first hours after local midnight in a zone east of UTC — and the
  // week's events fell into last week.
  const base = Math.floor(new Date(new Date().setHours(0, 0, 0, 0)).getTime() / 1000) + 9 * HOUR;
  // Local midnight, n days from today: where an all-day day begins, as the app makes them.
  const localDay = (n) => { const d = new Date(); d.setHours(0, 0, 0, 0); d.setDate(d.getDate() + n); return d.getTime() / 1000; };
  const events = !calendarFixture ? [] : [
    { guid: "e1", calendarGuid: "work", title: "Design review", location: "Studio",
      startsAt: base + HOUR, endsAt: base + 2 * HOUR, allDay: false,
      timezone: "Asia/Kolkata", url: null, notes: null, etag: null },
    { guid: "e2", calendarGuid: "home", title: "Dentist", location: null,
      startsAt: base + 5 * HOUR, endsAt: base + 6 * HOUR, allDay: false,
      timezone: "Asia/Kolkata", url: null, notes: null, etag: null },
    { guid: "e3", calendarGuid: "work", title: "Sprint planning", location: "Room 2",
      startsAt: base + DAY + 2 * HOUR, endsAt: base + DAY + 3 * HOUR, allDay: false,
      timezone: "Asia/Kolkata", url: null, notes: null, etag: null },
    // One day of a weekly series, as Apple names an occurrence: the series' id and its start.
    { guid: "SERIES__20261020T040000Z", calendarGuid: "work", title: "Weekly standup", location: null,
      startsAt: base + 2 * HOUR, endsAt: base + 2 * HOUR + 1800, allDay: false,
      timezone: "Asia/Kolkata", url: null, notes: null, etag: null },
    { guid: "e4", calendarGuid: "personal", title: "Flight to Hyderabad", location: null,
      startsAt: base + 3 * DAY, endsAt: base + 4 * DAY, allDay: true,
      timezone: "Asia/Kolkata", url: null, notes: null, etag: null },
    { guid: "quince:birthdays:c1:2026", calendarGuid: "quince:birthdays", title: "Zoya Ahuja’s Birthday",
      location: null, startsAt: localDay(2), endsAt: localDay(3), allDay: true,
      timezone: null, url: null, notes: null, etag: null },
    { guid: "quince:holidays:diwali:2026", calendarGuid: "quince:holidays", title: "Diwali",
      location: null, startsAt: localDay(5), endsAt: localDay(6), allDay: true,
      timezone: null, url: null, notes: null, etag: null },
    // A suite's own events, placed `day`/`hour` from today at 9 AM, with fields as given.
    ...extraEvents.map(({ day = 0, hour = 0, length = 1, ...rest }) => ({
      calendarGuid: "work", location: null, allDay: false, timezone: null, url: null, notes: null, etag: null,
      startsAt: base + day * DAY + hour * HOUR, endsAt: base + day * DAY + (hour + length) * HOUR, ...rest,
    })),
  ];
  const dueReminders = !calendarFixture ? [] : [
    { guid: "r1", listGuid: "l1", title: "Water the plants", notes: null,
      dueAt: base + DAY + 8 * HOUR, completedAt: null, priority: 1, etag: null },
    { guid: "r2", listGuid: "l2", title: "Evening run", notes: null,
      dueAt: base + 6 * HOUR, completedAt: null, priority: 0, etag: null },
    { guid: "r3", listGuid: "l1", title: "Plan tomorrow", notes: null,
      dueAt: base + 13 * HOUR, completedAt: base + 13 * HOUR, priority: 0, etag: null },
  ];
  const noteFolders = [
    { guid: "f1", title: "Notes", sortOrder: 0 },
    { guid: "f2", title: "Recently Deleted", sortOrder: 1 },
  ];
  const noteList = !notesFixture ? [] : [
    { guid: "n1", folderGuid: "f1", title: "Curtains",
      snippet: "Bathroom accessories like mirror holder…",
      body: "Curtains\nBathroom accessories like mirror holder, soap holder, shampoo, clothes hanger\nFan\nwall clock\nBed\nDining table\nTea filter\nKnife\nWashing Machine cover",
      locked: false, editable: true, pinned: false, createdAt: null, modifiedAt: 1788500000, etag: null },
    { guid: "n2", folderGuid: "f1", title: "🔴 UNIT 1: SOFTWARE TESTING METHODOLOGIES",
      snippet: "📌 SECTION A: 2-MARK HIGH-YIELD…",
      body: "🔴 UNIT 1: SOFTWARE TESTING METHODOLOGIES\n\n📌 SECTION A: 2-MARK HIGH-YIELD QUESTIONS\n\nSoftware testing is the process of evaluating a system to find whether it satisfies the specified requirements, and to detect the differences between expected and actual results.",
      locked: false, editable: true, pinned: true, createdAt: null, modifiedAt: 1787900000, etag: null },
    { guid: "n3", folderGuid: "f1",
      title: "These sources outline the fundamental principles and methodologies",
      snippet: "of **Software Testing Methodologies…",
      body: "These sources outline the fundamental principles and methodologies of **Software Testing Methodologies (CSEN3311)**, emphasizing its evolution from simple debugging to a structured, process-oriented discipline. The material details the **Software Testing Life Cycle (STLC)**, covering essential phases from initial planning and test design to execution and post-implementation review.",
      locked: false, editable: true, pinned: false, createdAt: null, modifiedAt: 1787800000, etag: null },
  ];
  const contactList = !contactsFixture ? [] : [
    { guid: "c1", firstName: "Zoya", lastName: "Ahuja", company: null, isCompany: false,
      birthday: null, notes: null, etag: null,
      methods: [{ kind: "phone", label: "mobile", value: "+91 90000 00001" },
                { kind: "email", label: "home", value: "zoya@example.com" }] },
    { guid: "c2", firstName: "Amma", lastName: null, company: null, isCompany: false,
      birthday: null, notes: null, etag: null,
      methods: [{ kind: "phone", label: null, value: "+91 98765 43210" }] },
    { guid: "c3", firstName: null, lastName: null, company: "Northfield University",
      isCompany: true, birthday: null, notes: null, etag: null,
      methods: [{ kind: "phone", label: "work", value: "+1 555 010 0199" },
                { kind: "url", label: null, value: "northfield.example" }] },
    { guid: "c4", firstName: null, lastName: null, company: null, isCompany: false,
      birthday: null, notes: null, etag: null,
      methods: [{ kind: "phone", label: null, value: "+91 90000 11111" }] },
  ].concat(contactsFixture ? (window.__contactsExtra ?? []) : []);
  // The book as the library holds it, for a test to change "on the phone" before playing a
  // pass in with __emit.
  window.__contactList = contactList;
  const driveItems = !driveFixture ? [] : [
    { drivewsid: "FOLDER::a", parentId: null, name: "Downloads", extension: null,
      kind: "FOLDER", size: null, childCount: 3, createdAt: 1785000000,
      modifiedAt: 1788000000, etag: null },
    { drivewsid: "APPLIB::b", parentId: null, name: "Keynote", extension: null,
      kind: "APP_LIBRARY", size: null, childCount: null, createdAt: 1780000000,
      modifiedAt: null, etag: null },
    { drivewsid: "APPLIB::c", parentId: null, name: "Shortcuts", extension: null,
      kind: "APP_LIBRARY", size: null, childCount: null, createdAt: 1780000000,
      modifiedAt: null, etag: null },
    { drivewsid: "FILE::d", parentId: null, name: "Semester Plan", extension: "pdf",
      kind: "FILE", size: 2411724, childCount: null, createdAt: 1787000000,
      modifiedAt: 1788600000, etag: null },
  ];
  const reminderLists = !remindersFixture ? [] : [
    { guid: "l1", title: "Daily", colour: "#63da38", ctag: null, sortOrder: 0, enabled: true },
    { guid: "l2", title: "GYM", colour: "#cc73e1", ctag: null, sortOrder: 1, enabled: true },
    { guid: "l3", title: "Life Goals", colour: "#1badf8", ctag: null, sortOrder: 2, enabled: true },
  ];
  const reminderItems = !remindersFixture ? [] : [
    { guid: "r1", listGuid: "l1", title: "Plan tomorrow", notes: null,
      dueAt: base - 2 * HOUR, completedAt: null, priority: 0, flagged: false, parentGuid: null, etag: null },
    { guid: "r2", listGuid: "l1", title: "Pick up the parcel", notes: null,
      dueAt: base + DAY + 7 * HOUR, completedAt: null, priority: 0, flagged: true, parentGuid: null, etag: null },
    { guid: "r3", listGuid: "l2", title: "Evening run", notes: "Legs today",
      dueAt: base + 6 * HOUR, completedAt: null, priority: 0, flagged: false, parentGuid: null, etag: null },
    { guid: "r4", listGuid: "l1", title: "Water the plants", notes: null,
      dueAt: base - DAY, completedAt: base - DAY, priority: 0, flagged: false, parentGuid: null, etag: null },
  ];
  // Opt-in: the long reminder Calendar shows (`__longReminder`), in its list here too.
  if (remindersFixture && window.__longReminder)
    reminderItems.push({ guid: "r9", listGuid: "l2", title: "Sort out everything for the trip to the coast before Friday, including the tickets and the bags",
      notes: "Ask Alex Rivera about the car", dueAt: base + 4 * HOUR, completedAt: null, priority: 0, flagged: false, parentGuid: null, etag: null });
  // Shaped like a real account rather than a tidy one: the default is not the first, and one
  // of them is a subscribed feed that nothing may be written to.
  const calendars = [
    { guid: "work", title: "Work", colour: "#cc73e1", ctag: null, sortOrder: 0, enabled: true,
      writable: true, isDefault: false },
    { guid: "home", title: "Home", colour: "#0088ff", ctag: null, sortOrder: 1, enabled: true,
      writable: true, isDefault: true },
    { guid: "personal", title: "Personal", colour: "#1badf8", ctag: null, sortOrder: 2, enabled: true,
      writable: true, isDefault: false },
    // As the app adds them: drawn from contacts and Apple's holiday feed, read-only.
    { guid: "quince:birthdays", title: "Birthdays", colour: "#8e8e93", ctag: null, sortOrder: 900,
      enabled: true, writable: false, isDefault: false },
    { guid: "quince:holidays", title: "India Holidays", colour: "#34c759", ctag: null, sortOrder: 901,
      enabled: true, writable: false, isDefault: false },
  ];
  // Opt-in: the README's demo library (readme-fixture.mjs, installed first as window.__demo)
  // takes the place of the rows above, and answers the commands it knows before the switch.
  const demo = demoFixture ? window.__demo : null;
  if (demo) {
    for (const [mine, theirs] of [[events, demo.events], [dueReminders, demo.dueReminders], [noteFolders, demo.noteFolders],
      [noteList, demo.noteList], [contactList, demo.contactList], [driveItems, demo.driveItems], [reminderLists, demo.reminderLists],
      [reminderItems, demo.reminderItems], [calendars, demo.calendars]]) mine.splice(0, mine.length, ...theirs);
    // One store, so a write to either is seen by both.
    demo.noteList = noteList; demo.reminderItems = reminderItems;
  }
  // Tauri's event plugin keeps this beside the internals: an unlisten (React's development
  // mode unmounts every effect once) calls it.
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  window.__TAURI_INTERNALS__ = {
    transformCallback: (cb) => cb,
    // As Tauri builds it on Linux (tauri/scripts/core.js): the whole path escaped.
    // A test that needs real pictures sets __picBase and serves them there (page.route).
    convertFileSrc: (path, protocol = "asset") =>
      window.__picBase ? `${window.__picBase}/${encodeURIComponent(path)}` : `${protocol}://localhost/${encodeURIComponent(path)}`,
    invoke: async (cmd, args) => {
      if (demo) { const answer = demo.answer(cmd, args); if (answer !== undefined) return answer; }
      switch (cmd) {
        case "auth_status":
          return { hasStoredSession: screen === "library", signedIn: screen === "library" };
        case "auth_resume":      return screen === "library" ? "library" : "signin";
        case "auth_sign_in":     return screen === "code" ? "needs_two_factor" : "complete";
        case "auth_submit_code": window.__codeVia = "devices"; return null;
        case "account_services": return services;
        case "calendar_events": {
          window.__eventReads = (window.__eventReads ?? 0) + 1;
          // A heavy account, for measuring: `window.__manyEvents = n` adds n events spread over
          // the year around today, several a day.
          const n = window.__manyEvents ?? 0;
          // A subscription's fixtures: two matches this week, read-only.
          const subbed = (window.__subs ?? []).flatMap((x) => [
            { guid: `quince:sub:${x.id}:m1:1`, calendarGuid: `quince:sub:${x.id}`, title: "Arsenal v Chelsea", location: "Emirates Stadium",
              startsAt: base + DAY + 8 * HOUR, endsAt: base + DAY + 10 * HOUR, allDay: false, timezone: "Europe/London", url: null, notes: null, etag: null },
            { guid: `quince:sub:${x.id}:m2:1`, calendarGuid: `quince:sub:${x.id}`, title: "Liverpool v Spurs", location: null,
              startsAt: base + 2 * DAY + 8 * HOUR, endsAt: base + 2 * DAY + 10 * HOUR, allDay: false, timezone: "Europe/London", url: null, notes: null, etag: null },
          ]);
          // A suite's own events at fixed moments (`window.__fixedEvents`), for a given day —
          // and, with `__eventsSlow` set, only those inside the range asked for, answered that
          // many milliseconds late, as a library read over a range is.
          const slow = window.__eventsSlow ?? 0;
          if (slow) await new Promise((r) => setTimeout(r, slow));
          const fixed = (window.__fixedEvents ?? []).filter((e) => !slow || (e.endsAt > args.from && e.startsAt < args.to));
          if (!n) return [...events, ...subbed, ...fixed];
          const extra = (window.__extra ??= Array.from({ length: n }, (_, i) => ({
            guid: `many${i}`, calendarGuid: i % 2 ? "work" : "home", title: `Event ${i}`, location: null,
            startsAt: base + Math.floor((i - n / 2) / 8) * DAY + (8 + (i % 8)) * HOUR, endsAt: base + Math.floor((i - n / 2) / 8) * DAY + (9 + (i % 8)) * HOUR,
            allDay: false, timezone: "Asia/Kolkata", url: null, notes: null, etag: null })));
          return events.concat(extra);
        }
        case "calendar_search": {
          const words = String(args?.query ?? "").toLowerCase().split(/\s+/).filter(Boolean);
          return events.filter((e) => words.every((w) => `${e.title} ${e.location ?? ""}`.toLowerCase().includes(w)))
            .sort((a, b) => a.startsAt - b.startsAt);
        }
        case "calendar_list": {
          const subCals = (window.__subs ?? []).map((x, i) => ({ guid: `quince:sub:${x.id}`, title: x.title, colour: x.colour, ctag: null, sortOrder: 800 + i, enabled: true, writable: false, isDefault: false }));
          return [...(window.__cals ??= calendars.map((c) => ({ ...c }))).map((c) => ({ ...c })), ...subCals];
        }
        // Calendars subscribed to by address (calendar tests). `window.__feedFails` makes the
        // next subscribe refuse, as a wrong address is refused.
        // Calendar accounts other than iCloud (calendar-accounts). `window.__davFails` makes the
        // next add refuse; `window.__davApproved` is the person approving in their browser.
        case "dav_accounts": return (window.__davAccounts ?? []).map((x) => ({ ...x }));
        case "nextcloud_begin": {
          (window.__davWrites ??= []).push({ begin: args.server });
          if (window.__davFails) throw window.__davFails;
          window.__davApproved = false;
          return `https://${args.server}/index.php/login/v2/flow/abc`;
        }
        case "nextcloud_finish": {
          window.__davCancelled = false;
          while (!window.__davApproved && !window.__davCancelled) await new Promise((r) => setTimeout(r, 20));
          if (window.__davCancelled) throw "cancelled";
          const account = { id: "n1", kind: "nextcloud", title: "Nextcloud", server: "https://cloud.example.org", username: "felix",
            home: "https://cloud.example.org/remote.php/dav/calendars/felix/", hosts: ["cloud.example.org"], syncedAt: 1, problem: null, needsSignIn: false };
          (window.__davAccounts ??= []).push(account);
          (window.__cals ??= calendars.map((c) => ({ ...c }))).push(
            { guid: "quince:dav:n1:aaa", title: "Familie", colour: "#0082c9", ctag: "1", sortOrder: 500, enabled: true, writable: true, isDefault: false },
            { guid: "quince:dav:n1:bbb", title: "Team (shared)", colour: "#65db39", ctag: "1", sortOrder: 501, enabled: true, writable: false, isDefault: false },
          );
          return { ...account };
        }
        case "nextcloud_reopen": { (window.__davWrites ??= []).push({ reopen: true }); return null; }
        case "nextcloud_cancel": { (window.__davWrites ??= []).push({ cancel: true }); window.__davCancelled = true; return null; }
        case "caldav_add": {
          (window.__davWrites ??= []).push({ caldav: { server: args.server, username: args.username } });
          if (window.__davFails) throw window.__davFails;
          const account = { id: "c1", kind: "caldav", title: args.server, server: `https://${args.server}`, username: args.username,
            home: `https://${args.server}/dav/`, hosts: [args.server], syncedAt: 1, problem: null, needsSignIn: false };
          (window.__davAccounts ??= []).push(account);
          return { ...account };
        }
        case "dav_refresh": { (window.__davWrites ??= []).push({ refresh: args.id }); return null; }
        case "dav_remove": {
          (window.__davWrites ??= []).push({ remove: args.id });
          window.__davAccounts = (window.__davAccounts ?? []).filter((a) => a.id !== args.id);
          window.__cals = (window.__cals ?? []).filter((c) => !c.guid.startsWith(`quince:dav:${args.id}:`));
          return null;
        }
        case "calendar_subscriptions": return (window.__subs ?? []).map((x) => ({ ...x }));
        case "calendar_subscribe": {
          (window.__subWrites ??= []).push({ subscribe: args.url });
          if (window.__feedFails) throw window.__feedFails;
          const sub = { id: `s${(window.__subs ?? []).length + 1}`, url: String(args.url).replace(/^webcal:/i, "https:"),
            title: "Premier League", colour: "#ff9500", refreshMinutes: 1440, fetchedAt: Math.floor(Date.now() / 1000), problem: null };
          (window.__subs ??= []).push(sub);
          return { ...sub };
        }
        case "calendar_subscription_update": {
          (window.__subWrites ??= []).push({ update: args });
          const x = (window.__subs ?? []).find((y) => y.id === args.id);
          if (x && args.title) x.title = args.title;
          if (x && args.colour) x.colour = args.colour;
          if (x && args.refreshMinutes !== null && args.refreshMinutes !== undefined) x.refreshMinutes = args.refreshMinutes;
          return null;
        }
        case "calendar_unsubscribe": {
          (window.__subWrites ??= []).push({ unsubscribe: args.id });
          window.__subs = (window.__subs ?? []).filter((y) => y.id !== args.id);
          return null;
        }
        case "calendar_subscription_refresh": {
          const x = (window.__subs ?? []).find((y) => y.id === args.id);
          return { ...x, fetchedAt: Math.floor(Date.now() / 1000) };
        }
        case "calendar_create": {
          const cals = (window.__cals ??= calendars.map((c) => ({ ...c })));
          if (cals.some((c) => c.title === args.title)) throw `A calendar with the name \u201c${args.title}\u201d already exists.`;
          (window.__calWrites ??= []).push({ create: args.title });
          cals.push({ guid: `cal-${cals.length + 1}`, title: args.title, colour: "#ff2d55", ctag: null, sortOrder: 9, enabled: true, writable: true, isDefault: false });
          return `cal-${cals.length}`;
        }
        case "calendar_update": {
          (window.__calWrites ??= []).push({ update: args });
          const c = (window.__cals ??= calendars.map((c) => ({ ...c }))).find((c) => c.guid === args.guid);
          if (c && args.title) c.title = args.title;
          if (c && args.colour) c.colour = args.colour;
          return null;
        }
        case "calendar_delete": {
          (window.__calWrites ??= []).push({ delete: args.guid });
          window.__cals = (window.__cals ?? calendars.map((c) => ({ ...c }))).filter((c) => c.guid !== args.guid);
          return null;
        }
        // A test sets __repeat to give the series another repeat.
        case "event_repeat": return String(args.guid).startsWith("SERIES__") ? (window.__repeat ?? { freq: "weekly", interval: 1, count: null }) : null;
        case "event_alerts": return args.guid === "e2" ? [15] : [];
        // A test sets __guests[guid] to give an event its guests.
        case "event_guests": return (window.__guests ?? {})[args.guid] ?? [];
        case "event_save": (window.__eventSaves ??= []).push(args.draft); return "E-new";
        case "event_delete": (window.__eventDeletes ??= []).push(args); return null;
        case "calendar_sync": return { calendars: calendars.length, events: events.length };
        case "reminder_counts": return { today: 4, scheduled: 7, flagged: 2, all: 12, completed: 3, lists: [["L1", 5], ["L2", 3], ["lists", 4]] };
        case "reminder_lists": return !remindersFixture ? [] : reminderLists;
        case "reminders_in": {
          if (!remindersFixture || remindersFixture === "lists") return [];
          // Opt-in for measuring a switch: each list's own reminders, after a real read's wait.
          if (window.__slowItems) await new Promise((r) => setTimeout(r, window.__slowItems));
          // A test's own extra reminders (__extraReminders), kept as Apple keeps them.
          const every = [...reminderItems, ...(window.__extraReminders ?? [])];
          return window.__filterLists ? every.filter((r) => r.listGuid === args.listGuid) : every;
        }
        case "reminders_due_between":
          // Opt-in: one with a title far longer than a cell, and notes, as a real list has.
          if (window.__longReminder && calendarFixture && !dueReminders.some((r) => r.guid === "r9"))
            dueReminders.push({ guid: "r9", listGuid: "l2", title: "Sort out everything for the trip to the coast before Friday, including the tickets and the bags",
              notes: "Ask Alex Rivera about the car", dueAt: base + 4 * HOUR, completedAt: null, priority: 0, etag: null });
          // A suite's own (`window.__dueExtra`), chosen as the library chooses them: by the
          // moment Apple stores, inside the range asked for.
          if (!calendarFixture) return [];
          return [...dueReminders, ...(window.__dueExtra ?? []).filter((r) => r.dueAt >= args.from && r.dueAt < args.to)];
        case "reminder_create":
          // Kept, as Apple keeps it, with what the view implied.
          {
            // Numbered on from the fixture, never twice: one moved away by a reorder does not
            // free its number for the next.
            window.__madeSeq = (window.__madeSeq ?? reminderItems.length - 1) + 1;
            const guid = `made-${window.__madeSeq}`;
            (window.__creates ??= []).push(args);
            reminderItems.push({ guid, listGuid: args.listGuid, title: args.title,
              notes: null, dueAt: args.dueAt ?? null, allDay: args.allDay ?? false, completedAt: null, priority: 0,
              flagged: args.flagged ?? false, parentGuid: null, etag: null });
            // Its new id, as the app's command answers.
            return guid;
          }
        case "reminder_delete": return null;
        case "reminder_set_done":
          (window.__done ??= []).push(args);
          // A slow network, when a test asks for one.
          if (window.__doneDelay) await new Promise((r) => setTimeout(r, window.__doneDelay));
          {
            // A repeating one rolls on, as Apple rolls it: open again, a day (or its unit) on.
            const r = (window.__extraReminders ?? []).find((x) => x.guid === args.guid);
            // Its last time before End Repeat (__ending): completed, as Apple completes it.
            const last = (window.__ending ?? []).includes(args.guid);
            if (r && last && args.done) { r.completedAt = Math.floor(Date.now() / 1000); window.__ending = window.__ending.filter((g) => g !== args.guid); }
            else if (r && r.recurFrequency !== null && r.recurFrequency !== undefined && args.done) r.dueAt += 86400 * (r.recurInterval ?? 1);
            // Unticked, a fixture reminder is open again in the library.
            const own = reminderItems.find((x) => x.guid === args.guid);
            if (own && !args.done) own.completedAt = null;
          }
          return null;
        case "reminder_set_repeat": {
          (window.__repeats ??= []).push(args);
          // Kept, as Apple keeps it: the rule on the reminder, and its end beside it.
          const r = [...reminderItems, ...(window.__extraReminders ?? [])].find((x) => x.guid === args.guid);
          if (r) { r.recurFrequency = args.repeat?.frequency ?? null; r.recurInterval = args.repeat?.interval ?? null; }
          const more = (window.__more ??= {})[args.guid];
          if (more) more.repeat = args.repeat ? { ...args.repeat, endAt: args.endAt ?? null, occurrenceCount: 0, daysOfWeek: args.daysOfWeek ?? [], other: [] } : null;
          ((window.__marks ??= {})[args.guid] ??= {}).daysOfWeek = args.repeat ? (args.daysOfWeek ?? []) : [];
          return null;
        }
        // The rest of a reminder's Details (reminders_cmds.rs reminder_more). A test sets
        // __more[guid] to give one a place, a person, Urgent, a zone or a rule with chosen days.
        case "reminders_ending": return window.__ending ?? [];
        // Row marks (reminders_cmds.rs reminder_marks): a test sets __marks[guid].
        case "reminder_marks":
          return args.guids.filter((g) => (window.__marks ?? {})[g]).map((g) => ({ guid: g, daysOfWeek: [], urgent: false,
            location: null, messaging: null, url: null, ...window.__marks[g] }));
        case "reminder_more": {
          const r = [...reminderItems, ...(window.__extraReminders ?? [])].find((x) => x.guid === args.guid);
          const base = { guid: args.guid, repeat: r && r.recurFrequency != null ? { frequency: r.recurFrequency, interval: r.recurInterval ?? 1, endAt: null, occurrenceCount: 0,
            daysOfWeek: (window.__marks ?? {})[args.guid]?.daysOfWeek ?? [], other: [] } : null,
            timeZone: r?.timezone ?? null, urgent: false, location: null, messaging: null };
          return { ...base, ...((window.__more ?? {})[args.guid] ?? {}) };
        }
        case "reminder_edit": {
          (window.__edits ??= []).push(args.edit);
          // Kept, as Apple keeps it, so the read after a write shows the write.
          const r = [...reminderItems, ...(window.__extraReminders ?? [])].find((x) => x.guid === args.guid);
          // A move to another list is kept only where a suite asks (`__keepMoves`).
          if (r) for (const [k, v] of Object.entries(args.edit)) if (k !== "listGuid" || window.__keepMoves) r[k] = v;
          return null;
        }
        case "reminder_reorder": {
          (window.__reorders ??= []).push(args);
          // In whichever list of rows holds it: a suite's own (__extraReminders) too.
          const rows = reminderItems.some((x) => x.guid === args.guid) ? reminderItems : (window.__extraReminders ?? []);
          const i = rows.findIndex((x) => x.guid === args.guid);
          if (i < 0) return null;
          const [moved] = rows.splice(i, 1);
          // Put before one in the other list of rows (one made here, before a suite's own), it
          // goes into that one, so the order read back is the order asked for.
          const into = rows.some((x) => x.guid === args.before) ? rows : [reminderItems, window.__extraReminders ?? []].find((l) => l.some((x) => x.guid === args.before)) ?? rows;
          const at = into.findIndex((x) => x.guid === args.before);
          into.splice(at < 0 ? into.length : at, 0, moved);
          return null;
        }
        case "reminder_list_colour": {
          (window.__listColours ??= []).push([args.listGuid, args.colour]);
          const hex = { purple: "#C077DB", red: "#FF3B30", blue: "#007AFF" }[args.colour];
          const list = reminderLists.find((l) => l.guid === args.listGuid);
          if (list && hex) list.colour = hex;
          return null;
        }
        case "reminder_list_create": case "reminder_list_rename": case "reminder_list_delete":
          return null;
        // A suite's own reminders in the smart lists (`window.__smartExtra`).
        case "reminders_smart": {
          if (!remindersFixture || remindersFixture === "lists") return [];
          const every = [...reminderItems, ...(window.__smartExtra ?? [])];
          // Completed: those ticked off, the latest first (reminders.rs reminders_smart).
          if (args?.kind === "completed") return every.filter((r) => r.completedAt).sort((a, b) => b.completedAt - a.completedAt);
          return every.filter((r) => !r.completedAt);
        }
        // Reminders' details (reminders_cmds.rs): tags, a link, early alerts, custom smart lists
        // and sections. A test sets __tagMap / __details / __smartLists / __sections to change them.
        case "reminder_sections": return (window.__sections ?? []).filter((s) => s.listGuid === args.listGuid);
        case "reminder_tags": case "reminders_tagged": case "reminder_detail": case "reminder_extras": {
          const tagMap = (window.__tagMap ??= !remindersFixture || remindersFixture === "lists" ? {} : { r1: ["home"], r2: ["errands", "home"], r3: ["fitness"] });
          const every = [...reminderItems, ...(window.__extraReminders ?? [])];
          if (cmd === "reminder_detail") {
            const extra = (window.__details ?? (remindersFixture ? { r2: { url: "https://www.apple.com/shop/order", earlyAlerts: [{ id: "a1", count: 30, unit: 0 }] } } : {}))[args.guid] ?? {};
            return { guid: args.guid, tags: tagMap[args.guid] ?? [], url: null, earlyAlerts: [], otherAttachments: 0, images: [], ...extra };
          }
          const open = (name) => every.filter((r) => !r.completedAt && (tagMap[r.guid] ?? []).some((t) => t.toLowerCase() === name.toLowerCase()));
          if (cmd === "reminders_tagged") return open(args.name);
          const names = [...new Set(Object.values(tagMap).flat())].sort((a, b) => a.localeCompare(b));
          // One read for the rows' tags and the custom smart lists' counts (reminder_extras).
          if (cmd === "reminder_extras") {
            const tagsByReminder = {};
            for (const name of names) for (const r of open(name)) (tagsByReminder[r.guid] ??= []).push(name);
            const lists = window.__smartLists ?? (!remindersFixture || remindersFixture === "lists" ? [] : [{ guid: "SL-1", kind: "custom" }]);
            const held = every.filter((r) => !r.completedAt && r.flagged).length;
            return { tagsByReminder, customCounts: Object.fromEntries(lists.filter((x) => x.kind === "custom").map((x) => [x.guid, held])) };
          }
          return names.map((name) => ({ name, open: open(name).length }));
        }
        case "reminder_smart_lists": return window.__smartLists ?? (!remindersFixture || remindersFixture === "lists" ? [] : [
          { guid: "SL-today", kind: "today", title: null, colour: null, emblem: null, pinned: true, sorting: null, filter: null },
          { guid: "SL-1", kind: "custom", title: "Important", colour: "#FF9500", emblem: null, pinned: false, sorting: null, filter: '{"priorities":["medium"]}' },
        ]);
        case "reminders_in_smart_list":
          return { reminders: [...reminderItems, ...(window.__extraReminders ?? [])].filter((r) => !r.completedAt && r.flagged), unknown: window.__smartUnknown ?? [] };
        case "reminder_tag_add": case "reminder_tag_remove": case "reminder_url_set": case "reminder_early_alert_set":
        case "reminder_section_create": case "reminder_section_rename": case "reminder_section_move": {
          (window.__detailWrites ??= []).push({ cmd, ...args });
          if (cmd === "reminder_section_create") {
            const made = { guid: `sec-${Date.now()}`, listGuid: args.listGuid, title: args.title, createdAt: null };
            (window.__sections ??= []).push(made);
            return made;
          }
          if (cmd === "reminder_section_rename") { const s = (window.__sections ?? []).find((x) => x.guid === args.guid); if (s) s.title = args.title; }
          if (cmd === "reminder_section_move") { const r = [...reminderItems, ...(window.__extraReminders ?? [])].find((x) => x.guid === args.guid); if (r) r.sectionGuid = args.section; }
          if (cmd === "reminder_tag_add" || cmd === "reminder_tag_remove") {
            const tagMap = (window.__tagMap ??= {});
            const now = tagMap[args.guid] ?? [];
            tagMap[args.guid] = cmd === "reminder_tag_add" ? [...new Set([...now, args.name])] : now.filter((t) => t !== args.name);
          }
          return null;
        }
        case "note_paragraph": (window.__paragraphs ??= []).push(args); return null;
        case "note_runs": {
          if (window.__savedRuns?.[args.guid]) return window.__savedRuns[args.guid];
          if (args.guid === "nM") {
            const plain = (length) => ({ length, style: null, indent: 0, checked: null, bold: false, italic: false, underline: false, strike: false, link: null });
            return [plain(5), { ...plain(1), attachment: "M1" }, plain(1), { ...plain(1), attachment: "M2" }, plain(4)];
          }
          if (args.guid === "nG") {
            // "Pasta\n", "Dinner for ", a #tag, " tonight": the tag one inline attachment in its line.
            const plain = (length) => ({ length, style: null, indent: 0, checked: null, bold: false, italic: false, underline: false, strike: false, link: null });
            return [{ ...plain(6), style: 0 }, plain(11), { ...plain(1), attachment: "H1" }, plain(8)];
          }
          if (args.guid === "nD") {
            const plain = (length) => ({ length, style: null, indent: 0, checked: null, bold: false, italic: false, underline: false, strike: false, link: null });
            // "Sketch\n", "See " then "The plan" linked to that note (lower-cased, as a phone links), "\n", the drawing, "\nend".
            return [{ ...plain(7), style: 0 }, plain(4), { ...plain(8), link: "applenotes:note/1a2b3c4d-0000-4000-8000-00000000ab12?ownerIdentifier=_x" }, plain(1), { ...plain(1), attachment: "D1" }, plain(4)];
          }
          if (args.guid === "nL") {
            // "Links", then the address line, then the last line quoted (Block Quote, field 8).
            const body = (noteList.find((n) => n.guid === "nL") ?? {}).body ?? "";
            const lines = body.split("\n");
            return lines.map((l, i) => ({ length: l.length + (i < lines.length - 1 ? 1 : 0), style: i === 0 ? 0 : null, indent: 0,
              checked: null, bold: false, italic: false, underline: false, strike: false, link: null, ...(i === 2 ? { quote: true } : {}) }));
          }
          if (args.guid === "nA") {
            // Shaped like a real note made on a phone (text made generic): Apple's runs come in
            // pieces — a word split in two, a sentence in five — that say nothing more than one
            // run would. Eight attachments on their own lines, a sentence, then a ninth.
            const plain = (length) => ({ length, style: null, indent: 0, checked: null, bold: false, italic: false, underline: false, strike: false, link: null });
            const att = (id) => ({ ...plain(1), attachment: id });
            const lines = [];
            for (let i = 1; i <= 8; i++) lines.push(att(`A${i}`), plain(1));
            return [plain(10), plain(1), plain(1), ...lines, plain(2), plain(10), plain(8), plain(5), plain(1), plain(1), att("A9")];
          }
          if (args.guid === "nT") {
            // "Groceries", a table on its own line, then a last line: the table's run names it.
            const plain = (length) => ({ length, style: null, indent: 0, checked: null, bold: false, italic: false, underline: false, strike: false, link: null });
            // "end" coloured mint on a phone (field 14).
            // `__unreadTable`: a table this app cannot read (U1).
            return [plain(10), { ...plain(1), attachment: window.__unreadTable ? "U1" : "T1" }, plain(1), { ...plain(3), emphasis: 4 }];
          }
          // Curtains, set as a phone sets one: a title, then a dashed list and a ticked item.
          if (args.guid !== "n1") return [];
          const body = (noteList.find((n) => n.guid === "n1") ?? {}).body ?? "";
          const lines = body.split("\n");
          const runs = lines.map((l, i) => ({ length: l.length + (i < lines.length - 1 ? 1 : 0),
            style: i === 0 ? 0 : i === 1 ? 103 : 101, indent: 0, checked: i === 1 ? true : null,
            bold: false, italic: false, underline: false, strike: false, link: null }));
          return runs;
        }
        // Locked notes (notes_cmds.rs). The password is "marigold-7"; `__unlockAsks` keeps what was
        // sent, `__lockForgets` counts Lock Now. `__lockHint` overrides the hint (null for none).
        case "note_unlock": {
          (window.__unlockAsks ??= []).push(args);
          if (window.__slowUnlock) await new Promise((r) => setTimeout(r, window.__slowUnlock));
          if (args.password !== "marigold-7") throw "That password didn’t open this note.";
          const text = "Bank\nPIN for the locker 4821\nCall before Friday";
          const line = (length, extra = {}) => ({ length, style: null, indent: 0, checked: null, bold: false, italic: false, underline: false, strike: false, link: null, ...extra });
          return { body: text, runs: [line(5, { style: 0 }), line(24), line(18, { style: 103, checked: false })] };
        }
        case "note_lock_hint": return window.__lockHint !== undefined ? window.__lockHint : "The fruit";
        case "note_lock_forget": window.__lockForgets = (window.__lockForgets ?? 0) + 1; return null;
        // Search across every folder but Recently Deleted, by every word typed.
        case "notes_search": {
          const words = String(args.query ?? "").toLowerCase().split(/\s+/).filter(Boolean);
          return !notesFixture ? [] : noteList
            .filter((n) => n.folderGuid !== "f2" && words.every((w) => `${n.title} ${n.body ?? ""}`.toLowerCase().includes(w)))
            // The preview is the line where a word is, past the title, as the library gives it
            // (search.rs note_line); the stored one when the words are only in the title.
            .map((n) => {
              const rest = (n.body ?? "").split("\n").slice(1);
              const line = rest.find((l) => words.some((w) => l.toLowerCase().includes(w)));
              return { ...n, snippet: line ?? n.snippet };
            });
        }
        // Adding files: off unless a test turns it on (`__attachOn`), as the backend's switch is.
        case "note_attach_enabled": return Boolean(window.__attachOn);
        // A picture of the file being added: a tiny generated PNG, or none (__attachPreview false).
        case "note_attach_preview": {
          if (window.__attachPreview === false) return new ArrayBuffer(0);
          const c = document.createElement("canvas"); c.width = 8; c.height = 8;
          const g = c.getContext("2d"); g.fillStyle = "#2f9bff"; g.fillRect(0, 0, 8, 8);
          const blob = await new Promise((r) => c.toBlob(r, "image/png"));
          return await blob.arrayBuffer();
        }
        case "note_attach_file": {
          (window.__attaches ??= []).push(args);
          // A slow upload, when a test asks for one.
          if (window.__attachDelay) await new Promise((r) => setTimeout(r, window.__attachDelay));
          // A computer's own file, refused as the app refuses it (notes_cmds.rs).
          if (/desktop\.ini$/i.test(args.path)) throw "desktop.ini is a system file, so it wasn’t added.";
          const n = noteList.find((x) => x.guid === args.guid);
          const base = args.base ?? n?.body ?? "";
          const body = base.slice(0, args.atOffset) + "\uFFFC" + base.slice(args.atOffset);
          if (n) n.body = body;
          return { attachment: "A-new", media: "M-new", uti: "public.jpeg", offset: args.atOffset, body };
        }
        case "note_folders": return !notesFixture ? [] : (window.__noteFolders ??= noteFolders.slice()).slice();
        case "note_folder_create": {
          const guid = `f-${(window.__noteFolders ??= noteFolders.slice()).length + 1}`;
          window.__noteFolders.push({ guid, title: args.title, sortOrder: 0, parentGuid: args.parent ?? null });
          (window.__folderCalls ??= []).push(["create", args.title]);
          return guid;
        }
        case "note_folder_rename":
          window.__noteFolders = (window.__noteFolders ?? noteFolders.slice()).map((f) => (f.guid === args.guid ? { ...f, title: args.title } : f));
          (window.__folderCalls ??= []).push(["rename", args.guid, args.title]);
          return null;
        case "note_folder_delete":
          window.__noteFolders = (window.__noteFolders ?? noteFolders.slice()).filter((f) => f.guid !== args.guid);
          (window.__folderCalls ??= []).push(["delete", args.guid]);
          return null;
        case "note_save": {
          (window.__noteSaves ??= []).push(args);
          // A test can play the other device: what Apple answers is what it now holds.
          const kept = window.__phoneEdit ? window.__phoneEdit(args) : args.body;
          // Kept, as Apple keeps it: the next read of the note gives what was saved. Its row as
          // the library keeps it (outbox.rs lay_note): the title Apple holds while the title line
          // is untouched, else the first line with words; the time edited moved on by a change.
          const n = noteList.find((x) => x.guid === args.guid);
          if (n && kept !== n.body) {
            const words = (l) => l.replace(/[\uFFFC\u2028]/g, " ").split(/\s+/).filter(Boolean).join(" ");
            const lines = (t) => t.split("\n").map(words).filter((l) => /[\p{L}\p{N}]/u.test(l));
            const head = (t) => {
              let from = 0;
              for (const l of t.split("\n")) {
                if (/[\p{L}\p{N}]/u.test(words(l))) return t.slice(0, from + l.length);
                from += l.length + 1;
              }
              return t;
            };
            if (n.body == null || head(n.body) !== head(kept)) n.title = lines(kept)[0] ?? "";
            n.snippet = lines(kept)[1] ?? null;
            n.modifiedAt = Math.max(n.modifiedAt ?? 0, Math.floor(Date.now() / 1000));
          }
          if (n) n.body = kept;
          if (args.runs && kept === args.body) (window.__savedRuns ??= {})[args.guid] = args.runs;
          return kept;
        }
        // The window's answer to `quince://flush` (lib/flush): kept with what had been saved by then.
        case "window_flushed":
          (window.__flushed ??= []).push({ id: args.id, saves: (window.__noteSaves ?? []).length, creates: window.__noteCreates ?? 0 });
          return null;
        case "note_delete":
          (window.__noteDeletes ??= []).push(args.guid);
          if (window.__slowDelete) await new Promise((r) => setTimeout(r, window.__slowDelete));
          return null;
        // A move, kept as the library keeps it: in its new folder once it has gone through
        // (after `__slowMove` ms, Apple slow to answer). Left where it was, every read after a
        // slow move found the note back in its old folder — which the window then drew.
        case "note_move": {
          if (window.__slowMove) await new Promise((r) => setTimeout(r, window.__slowMove));
          const n = noteList.find((x) => x.guid === args?.guid);
          if (n) n.folderGuid = args.folderGuid ?? null;
          return null;
        }
        case "note_pin": {
          const n = noteList.find((x) => x.guid === args?.guid);
          if (n) n.pinned = args.pinned;
          (window.__pins ??= []).push([args?.guid, args?.pinned]);
          return null;
        }
        case "note_create": {
          // Kept, as the library keeps a note made here (it is listed from then on): left out,
          // the next read of the list lost it, and the screen went back to the note before.
          const body = args?.body ?? "";
          window.__noteCreates = (window.__noteCreates ?? 0) + 1;
          const title = body.split("\n").find((l) => l.trim()) ?? "";
          if (!noteList.some((x) => x.guid === "Note/new"))
            noteList.unshift({ guid: "Note/new", folderGuid: args?.folderGuid ?? "f1", title, snippet: "", body,
              locked: false, editable: true, pinned: false, createdAt: null, modifiedAt: Math.floor(Date.now() / 1000), etag: null });
          return "Note/new";
        }
        case "notes_in":
          // A suite's own notes (`window.__notesExtra`), their formatting in `__savedRuns`.
          for (const n of window.__notesExtra ?? []) if (!noteList.some((x) => x.guid === n.guid)) noteList.unshift({ ...n });
          // Opt-in: a note holding a table, as a phone makes one.
          if (window.__mediaNote && !noteList.some((n) => n.guid === "nM"))
            noteList.unshift({ guid: "nM", folderGuid: "f1", title: "Trip", snippet: "", body: "Trip\n\uFFFC\n\uFFFC\nend",
              locked: false, editable: true, pinned: false, createdAt: null, modifiedAt: 1789100000, etag: null });
          // Opt-in: a note linking to another note, with a drawing in it, as a phone makes them.
          if (window.__drawNote && !noteList.some((n) => n.guid === "nD"))
            noteList.unshift({ guid: "nD", folderGuid: "f1", title: "Sketch", snippet: "", body: "Sketch\nSee The plan\n\uFFFC\nend",
              locked: false, editable: true, pinned: false, createdAt: null, modifiedAt: 1789300000, etag: null },
              // The note it links to: a made-up record name, in another folder.
              { guid: "1A2B3C4D-0000-4000-8000-00000000AB12", folderGuid: "f3", title: "The plan", snippet: "Step one", body: "The plan\nStep one",
              locked: false, editable: true, pinned: false, createdAt: null, modifiedAt: 1781000000, etag: null });
          // Opt-in: a note with web addresses in its text and no links of its own.
          if (window.__linkNote && !noteList.some((n) => n.guid === "nL"))
            noteList.unshift({ guid: "nL", folderGuid: "f1", title: "Links", snippet: "", body: "Links\nYashps://www.youtube.com/watch?v=5vVh0ZGx8aU\nRead https://example.org/a. Done",
              locked: false, editable: true, pinned: false, createdAt: null, modifiedAt: 1789200000, etag: null });
          // Opt-in: a note of pictures, files, a recording, a film and a table, as a phone makes one.
          if (window.__attachNote && !noteList.some((n) => n.guid === "nA"))
            noteList.unshift({ guid: "nA", folderGuid: "f1", title: "Attachments", snippet: "", body: `Attachments\n${"\uFFFC\n".repeat(8)}A few words to try it out \n\uFFFC`,
              locked: false, editable: true, pinned: false, createdAt: null, modifiedAt: 1789500000, etag: null });
          // Opt-in: a note with a #tag in its line, as a phone sets one.
          if (window.__tagNote && !noteList.some((n) => n.guid === "nG"))
            noteList.unshift({ guid: "nG", folderGuid: "f1", title: "Pasta", snippet: "", body: "Pasta\nDinner for \uFFFC tonight",
              locked: false, editable: true, pinned: false, createdAt: null, modifiedAt: 1789600000, etag: null });
          if (window.__tableNote && !noteList.some((n) => n.guid === "nT"))
            noteList.unshift({ guid: "nT", folderGuid: "f1", title: "Groceries", snippet: "", body: "Groceries\n\uFFFC\nend",
              locked: false, editable: true, pinned: false, createdAt: null, modifiedAt: 1789000000, etag: null });
          // Opt-in: a note locked with a password, as a phone locks one.
          if (window.__lockedNote && !noteList.some((n) => n.guid === "nK"))
            noteList.unshift({ guid: "nK", folderGuid: "f1", title: "Bank", snippet: null, body: null,
              locked: true, editable: false, pinned: false, createdAt: null, modifiedAt: 1789400000, etag: null });
          // A folder's own notes, as the library answers (Library::notes_in); every note for none.
          return !notesFixture ? [] : noteList.filter((n) => args?.folderGuid == null || n.folderGuid === args.folderGuid).map((n) => ({ ...n }));
        case "note_attachment_info":
          if (/^A\d$/.test(String(args.id))) {
            (window.__infoAsks ??= []).push(args.id);
            // `__flakyInfo[id] = n`: the first n lookups of it fail, as one offline or mid-renewal does.
            if (window.__flakyInfo?.[args.id] > 0) { window.__flakyInfo[args.id] -= 1; throw "Apple couldn’t be reached."; }
            const A = {
              A1: { uti: "com.apple.m4a-audio", kind: "audio", filename: "recording.m4a", size: 48200 },
              A2: { uti: "public.json", kind: "file", filename: "servers.json", size: 2100 },
              A3: { uti: "com.apple.quicktime-movie", kind: "video", filename: "IMG_0001.mov", size: 8_400_000, width: 1080, height: 1920, hasPreview: true },
              A4: { uti: "public.heic", kind: "image", filename: "IMG_0002.HEIC", width: 3024, height: 4032, size: 1_900_000, hasPreview: true },
              A5: { uti: "public.heic", kind: "image", filename: "IMG_0003.HEIC", width: 4032, height: 3024, size: 2_100_000, hasPreview: true },
              A6: { uti: "com.apple.paper.doc.pdf", kind: "document", filename: "Form.pdf", size: 120_000, hasPreview: true },
              A7: { uti: "public.jpeg", kind: "image", filename: "IMG_0004.JPG", width: 1200, height: 800, size: 310_000, hasPreview: true },
              A8: { uti: "com.apple.notes.table", kind: "table", size: 1116 },
              A9: { uti: "public.png", kind: "image", filename: "new.png", width: 800, height: 600, size: 52_000, hasPreview: true },
            }[args.id];
            const info = { id: args.id, title: null, width: null, height: null, size: null, hasPreview: false, filename: null, ...A };
            return window.__slowInfo ? new Promise((r) => setTimeout(() => r(info), window.__slowInfo)) : info;
          }
          if (args.id === "H1") return { id: "H1", uti: "com.apple.notes.inlinetextattachment.hashtag", kind: "inline", filename: null, title: "#Recipes", width: null, height: null, size: null, hasPreview: false };
          if (args.id === "D1") return { id: "D1", uti: "com.apple.paper", kind: "drawing", filename: null, title: null, width: 768, height: 621, size: null, hasPreview: true };
          if (args.id === "U1") return { id: "U1", uti: "com.apple.notes.table", kind: "table", filename: null, title: null, width: null, height: null, size: 1116, hasPreview: false };
          // Any other id a table is drawn for (note_table answers it): a table, as the backend
          // describes one (note_media.rs kind_of: com.apple.notes.table → "table").
          if (!/^M[12]$/.test(String(args.id))) return { id: args.id, uti: "com.apple.notes.table", kind: "table", filename: null, title: null, width: null, height: null, size: 1116, hasPreview: false };
          return args.id === "M1"
            ? { id: "M1", uti: "public.jpeg", kind: "image", filename: "beach.jpeg", title: null, width: 1200, height: 800, size: 271872, hasPreview: true }
            : { id: "M2", uti: "org.openxmlformats.spreadsheetml.sheet", kind: "file", filename: "Budget.xlsx", title: null, width: null, height: null, size: 9100, hasPreview: false };
        case "note_attachment_open": (window.__opened ??= []).push(args.id); return null;
        // Each note's first picture kept on this computer (`__notePictures`, by note); none by default.
        case "note_pictures": window.__pictureAsks = (window.__pictureAsks ?? 0) + 1; return { ...(window.__notePictures ?? {}) };
        case "note_table": if (/^[MDU]/.test(String(args.id)) || /^A[1-79]$/.test(String(args.id))) return null; return (window.__table ??= [["Item", "Qty"], ["Milk", "2"], ["", ""]]).map((r) => r.slice());
        case "photos_list": case "photos_counts": {
          const P = (id, takenAt, extra = {}) => ({ id, master: `m${id}`, takenAt, addedAt: takenAt, kind: "image",
            width: 4032, height: 3024, duration: null, filename: `IMG_${id}.JPG`, size: 2400000, uti: "public.jpeg",
            favorite: false, hidden: false, deleted: false, deletedAt: null, subtype: 0, changeTag: null, ...extra,
            // A test's own marks on a photo (a burst, a Live Photo), set before the first read.
            ...(window.__photoExtra?.[id] ?? {}) });
          const all = (window.__photos ??= [
            P("p1", 1792500000, { favorite: true }), P("p2", 1792400000, { keywords: ["beach"], latitude: 17.7653751, longitude: 83.357605 }), P("p3", 1792300000, { kind: "video", duration: 12.4, uti: "public.mpeg-4", filename: "clip.mp4" }),
            P("p4", 1789900000, { addedAt: 1792700000 }), P("p5", 1789800000, { rating: 5 }), P("p6", 1789700000, { deleted: true, deletedAt: 1792600000, expungeAt: Math.floor(Date.now() / 1000) + 2 * 86400 + 600 }),
          ]);
          // The library switcher, as the library keeps it: a photo with a `zone` is the Shared
          // Library's.
          const inLibrary = (p) => (window.__library === "personal" ? !p.zone : window.__library === "shared" ? !!p.zone : true);
          const live = all.filter((p) => !p.deleted && !p.hidden && inLibrary(p));
          const views = {
            library: live, recents: live, favorites: live.filter((p) => p.favorite), videos: live.filter((p) => p.kind === "video"),
            screenshots: live.filter((p) => p.subtype === 3), panoramas: live.filter((p) => p.subtype === 1), portrait: live.filter((p) => p.rendered === 8), recentlyDeleted: all.filter((p) => p.deleted),
            hidden: all.filter((p) => !p.deleted && p.hidden),
            rated: live.filter((p) => (p.rating ?? 0) > 0).sort((a, b) => b.rating - a.rating || b.takenAt - a.takenAt),
          };
          if (cmd === "photos_counts") return Object.entries(views).map(([k, v]) => [k, v.length]);
          // A read that takes its time, so a test can have it land after another view is chosen.
          const answer = (views[args.view] ?? []).map((p) => ({ ...p }));
          if (window.__slowView === args.view) return new Promise((r) => setTimeout(() => r(answer), 600));
          return answer;
        }
        // Paged: the index (ids, grouping dates, marks), the first page, and photos by id — each
        // made from the same lists the whole-list commands answer, sorted as the backend sorts.
        // Hidden, as the app keeps it: unlocked with the Apple Account password ("right" here),
        // refused until then; a wrong one said so. Every lock and unlock is noted.
        case "photos_unlock_hidden":
          (window.__hiddenCalls ??= []).push("unlock");
          if (args.password !== "right") throw "That password isn't right.";
          window.__hiddenOpen = true;
          return null;
        case "photos_lock_hidden": (window.__hiddenCalls ??= []).push("lock"); window.__hiddenOpen = false; return null;
        // Windows Hello: offered only where a test says it is set up (`__hello`: its answer).
        case "photos_hello_available": return Boolean(window.__hello);
        case "photos_unlock_hidden_hello":
          (window.__hiddenCalls ??= []).push("hello");
          if (window.__hello === "verified") window.__hiddenOpen = true;
          return window.__hello ?? "unavailable";
        case "photos_index": case "photos_page": {
          const self = window.__TAURI_INTERNALS__.invoke;
          const src = args.source;
          if (src.view === "hidden" && !window.__hiddenOpen) throw "The Hidden album is locked.";
          let list = src.view ? await self("photos_list", { view: src.view })
            : src.album ? await self("photos_in_album", { album: src.album })
            : src.query ? await self("photos_search", { query: src.query })
            : src.shared ? (window.__sharedPhotos?.[src.shared] ?? []).map((p) => ({ ...p }))
            : await self("photos_burst", { burst: src.burst });
          const sortable = src.view && src.view !== "rated" && src.view !== "recentlyDeleted";
          const order = sortable ? args.order : null;
          const key = (p) => (order?.by === "added" || (!order && src.view === "recents") ? (p.addedAt ?? p.takenAt) : p.takenAt);
          if (order) list = [...list].sort((a, b) => (key(b) - key(a)) * (order.oldest ? -1 : 1) || (a.id < b.id ? -1 : 1));
          // A page that takes its time (`__pageDelay` ms), as the real app's does over IPC.
          if (cmd === "photos_page") { if (window.__pageDelay) await new Promise((r) => setTimeout(r, window.__pageDelay)); const from = args.after ? list.findIndex((p) => p.id === args.after.id) + 1 : 0; return list.slice(from, from + args.limit); }
          (window.__indexReads ??= []).push(src);
          // A large library's index takes a moment: a test can make it take one.
          if (window.__indexDelay) await new Promise((r) => setTimeout(r, window.__indexDelay));
          const taken = list.map((p) => p.takenAt);
          return {
            ids: list.map((p) => p.id),
            keys: list.map(key),
            marks: list.map((p) => (p.kind === "video" ? 1 : p.kind === "live" ? 2 : 0) | (p.favorite ? 4 : 0) | ((p.rating ?? 0) << 3)),
            first: taken.length ? Math.min(...taken) : null,
            last: taken.length ? Math.max(...taken) : null,
          };
        }
        // The library's months, as quince-core photo_months chooses them: counted, with a
        // favourite (or else the still nearest the middle) as the key and three spread through.
        case "photos_months": {
          (window.__monthReads ??= []).push(Date.now());
          // Worked out once per fixture: on fifty thousand, the stub's own work showed as the app's.
          if (window.__monthsOf?.all === window.__photos) return window.__monthsOf.months;
          const self = window.__TAURI_INTERNALS__.invoke;
          const lib = (await self("photos_list", { view: "library" })).sort((a, b) => a.takenAt - b.takenAt || (a.id < b.id ? -1 : 1));
          const groups = new Map();
          for (const p of lib) {
            const d = new Date(p.takenAt * 1000);
            const k = d.getFullYear() * 100 + d.getMonth() + 1;
            if (!groups.has(k)) groups.set(k, []);
            groups.get(k).push(p);
          }
          const months = [...groups.entries()].sort((a, b) => b[0] - a[0]).map(([k, ps]) => {
            const n = ps.length;
            const mid = ps[Math.floor(n / 2)].takenAt;
            const score = (p) => (p.favorite ? 0 : 4e12) + (p.kind === "video" ? 2e12 : 0) + Math.abs(p.takenAt - mid);
            const key = ps.reduce((best, p) => (score(p) < score(best) ? p : best));
            const more = n >= 4 ? [...new Set([Math.floor((3 * n) / 4), Math.floor(n / 2), Math.floor(n / 4)].map((i) => ps[i].id))].filter((id) => id !== key.id) : [];
            return { year: Math.floor(k / 100), month: k % 100, count: n, videos: ps.filter((p) => p.kind === "video").length, key: key.id, keyFavorite: !!key.favorite, more };
          });
          window.__monthsOf = { all: window.__photos, months };
          return months;
        }
        case "photos_get": {
          (window.__gets ??= []).push(args.ids.length);
          // Shared albums' photos too: the backend finds a shared photo by its id as well.
          const all = [...(window.__photos ?? []), ...Object.values(window.__sharedPhotos ?? {}).flat()];
          const burst = window.__burstShots ?? [];
          // Made once per fixture: rebuilt per call, the stub's own work showed as the grid's.
          const src = window.__photos, shared = window.__sharedPhotos;
          if (window.__byIdOf?.src !== src || window.__byIdOf?.shared !== shared || window.__byIdOf?.burst !== burst) window.__byIdOf = { src, shared, burst, map: new Map([...burst, ...all].map((p) => [p.id, p])) };
          const byId = window.__byIdOf.map;
          return args.ids.map((id) => byId.get(id)).filter(Boolean).map((p) => ({ ...p }));
        }
        case "photos_prepare": (window.__prepared ??= []).push(args.shown); return null;
        // Shared albums as the library keeps them (`__shared`), and each one's photos by zone
        // (`__sharedPhotos`), made like the library's.
        case "photos_shared_albums": return [...(window.__shared ??= [])];
        // The library switcher: a Shared Library when `__hasShared`; the choice kept in `__library`.
        case "photos_library": return { showing: window.__library ?? "both", hasShared: !!window.__hasShared };
        case "photos_set_library": window.__library = args.which; return null;
        case "photos_burst": {
          // Three shots; the library's own photo is the chosen one, in the middle.
          const pick = (window.__photos ?? []).find((p) => p.burst === args.burst);
          if (!pick) return [];
          const shot = (n) => ({ ...pick, id: `${pick.id}-s${n}`, burstPick: false, takenAt: pick.takenAt - 1 + n, filename: `IMG_burst_${n}.JPG` });
          return (window.__burstShots = [shot(0), { ...pick }, shot(2)].map((p) => ({ ...p, burstCount: 3 })));
        }
        case "photos_albums": {
          const albums = (window.__albums ??= [{ id: "AL1", name: "Trips", position: 0, changeTag: null, items: ["p4", "p2"] }]);
          return albums.map(({ items, ...a }) => [a, items.length]);
        }
        case "photos_in_album": {
          const all = window.__photos ?? [];
          const a = (window.__albums ?? [{ id: "AL1", items: ["p4", "p2"] }]).find((x) => x.id === args.album);
          return a ? a.items.map((id) => all.find((p) => p.id === id)).filter(Boolean) : [];
        }
        case "photos_album_create": case "photos_album_rename": case "photos_album_delete":
        case "photos_album_add": case "photos_album_remove": {
          (window.__albumCalls ??= []).push({ cmd, args });
          if (window.__albumFail) throw "iCloud didn’t accept that change.";
          const albums = (window.__albums ??= [{ id: "AL1", name: "Trips", position: 0, changeTag: null, items: ["p4", "p2"] }]);
          const find = (id) => albums.find((x) => x.id === id);
          if (cmd === "photos_album_create") {
            const made = { id: `AL${albums.length + 1}`, name: args.name, position: -1, changeTag: "t1", folder: !!args.folder, parent: args.parent ?? null };
            albums.unshift({ ...made, items: [] });
            return made;
          }
          if (cmd === "photos_album_rename") find(args.id).name = args.name;
          if (cmd === "photos_album_delete") {
            // A folder goes with everything in it, as the app's own backend deletes it.
            const gone = new Set([args.id]);
            for (let grew = true; grew; ) {
              grew = false;
              for (const x of albums) if (x.parent && gone.has(x.parent) && !gone.has(x.id)) { gone.add(x.id); grew = true; }
            }
            for (let i = albums.length - 1; i >= 0; i--) if (gone.has(albums[i].id)) albums.splice(i, 1);
          }
          if (cmd === "photos_album_add") find(args.album).items.push(...args.ids.filter((i) => !find(args.album).items.includes(i)));
          if (cmd === "photos_album_remove") find(args.album).items = find(args.album).items.filter((i) => !args.ids.includes(i));
          return null;
        }
        case "photos_set_flag": {
          (window.__batches ??= []).push(args);
          if (window.__flagFail) throw "iCloud didn’t accept that change.";
          const out = [];
          for (const id of args.ids) {
            (window.__flags ??= []).push({ id, flag: args.flag, on: args.on });
            const p = (window.__photos ?? []).find((x) => x.id === id);
            if (!p) continue;
            if (args.flag === "favorite") p.favorite = args.on;
            if (args.flag === "hidden") p.hidden = args.on;
            if (args.flag === "expunged") { window.__photos.splice(window.__photos.indexOf(p), 1); continue; }
            if (args.flag === "deleted") { p.deleted = args.on; p.deletedAt = args.on ? 1792700000 : null; }
            out.push(p);
          }
          return out;
        }
        case "photo_set_flag": {
          (window.__flags ??= []).push(args);
          if (window.__flagFail) throw "iCloud didn’t accept that change.";
          const p = (window.__photos ?? []).find((x) => x.id === args.id);
          if (p) {
            if (args.flag === "favorite") p.favorite = args.on;
            if (args.flag === "hidden") p.hidden = args.on;
            if (args.flag === "expunged") window.__photos.splice(window.__photos.indexOf(p), 1);
            if (args.flag === "deleted") { p.deleted = args.on; p.deletedAt = args.on ? 1792700000 : null; }
          }
          return p;
        }
        case "photo_set_keywords": {
          (window.__keywords ??= []).push(args);
          const p = (window.__photos ?? []).find((x) => x.id === args.id);
          if (p) p.keywords = args.keywords;
          return p;
        }
        case "photos_search": {
          const months = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];
          const words = args.query.toLowerCase().split(/\s+/).filter(Boolean);
          return (window.__photos ?? [])
            .filter((p) => !p.deleted && !p.hidden)
            .filter((p) => {
              const d = new Date(p.takenAt * 1000);
              const hay = [p.filename, p.title, p.caption, ...(p.keywords ?? []), String(d.getFullYear()), months[d.getMonth()]].filter(Boolean).join(" ").toLowerCase();
              return words.every((w) => hay.includes(w));
            })
            .sort((a, b) => b.takenAt - a.takenAt);
        }
        case "photo_set_date": {
          (window.__dates ??= []).push(args);
          const p = (window.__photos ?? []).find((x) => x.id === args.id);
          if (p) p.takenAt = args.takenAt;
          return p;
        }
        case "photo_open_map": (window.__maps ??= []).push(args.id); return null;
        case "photo_set_rating": {
          (window.__ratings ??= []).push(args);
          const p = (window.__photos ?? []).find((x) => x.id === args.id);
          if (p) p.rating = args.rating;
          return p;
        }
        case "photo_set_text": {
          (window.__texts ??= []).push(args);
          const p = (window.__photos ?? []).find((x) => x.id === args.id);
          if (p) { p.title = args.title || null; p.caption = args.caption || null; }
          return p;
        }
        case "photo_play": (window.__plays ??= []).push(args); return null;
        case "video_base": return window.__videoBase ?? null;
        case "video_support": return window.__videoSupport ?? { h264: true, hevc: true, missing: [] };
        case "window_tint": (window.__tints ??= []).push(args.rgb.join(",")); return null;
        case "window_fullscreen": (window.__fullscreen ??= []).push(args.on); return args.on;
        case "photo_download": (window.__downloads ??= []).push(args); return `/home/test/Downloads/IMG_${args.id}.JPG`;
        // Linux's answer (the app put it on the clipboard: nothing back) unless a test hands the
        // PNG back as Windows and macOS do (__copyPng), after __copyDelay ms when one is set.
        case "photo_copy": {
          (window.__copies ??= []).push(args.id);
          if (window.__copyDelay) await new Promise((r) => setTimeout(r, window.__copyDelay));
          if (window.__copyFail) throw "The clipboard didn’t take the photo.";
          return window.__copyPng ? Uint8Array.from(atob(window.__copyPng), (c) => c.charCodeAt(0)).buffer : new ArrayBuffer(0);
        }
        // Each file is answered at once with where it stands; a test plays the rest in with __emit.
        case "photos_upload": {
          (window.__photoUploads ??= []).push(args.paths);
          (window.__photoUploadAlbums ??= []).push(args.album ?? null);
          const batch = `B${window.__photoUploads.length}`;
          const say = (e) => window.__emit?.("quince://photo-upload", { batch, within: "", sent: 0, total: 1000, folder: false, error: null, ...e });
          window.__photoLive = args.paths.map((path, i) => ({ id: `${batch}:${i}`, name: path.split("/").pop() }));
          setTimeout(() => {
            window.__photoLive.forEach((u) => say({ ...u, state: /\.txt$/.test(u.name) ? "failed" : "waiting",
              error: /\.txt$/.test(u.name) ? "Photos can’t add this kind of file." : null }));
            say({ ...window.__photoLive[0], state: "planned" });
          }, 0);
          return batch;
        }
        case "photos_upload_cancel": {
          (window.__photoCancels ??= []).push(args);
          for (const u of window.__photoLive ?? [])
            if (args.ids.includes(u.id) || (args.batch && u.id.startsWith(`${args.batch}:`)))
              window.__emit?.("quince://photo-upload", { ...u, batch: u.id.split(":")[0], within: "", sent: 0, total: 1000, folder: false, error: null, state: "cancelled" });
          return null;
        }
        case "note_table_add_row": {
          const t = (window.__table ??= [["Item", "Qty"], ["Milk", "2"], ["", ""]]);
          if (args.column) t.forEach((r) => r.splice(args.at, 0, ""));
          else t.splice(args.at, 0, t[0].map(() => ""));
          (window.__rowAdds ??= []).push(args);
          return t.map((r) => r.slice());
        }
        case "note_table_remove": {
          const t = (window.__table ??= [["Item", "Qty"], ["Milk", "2"], ["", ""]]);
          if (args.column) t.forEach((r) => r.splice(args.at, 1));
          else t.splice(args.at, 1);
          (window.__removes ??= []).push(args);
          return t.map((r) => r.slice());
        }
        case "note_table_set_cell": {
          (window.__cellWrites ??= []).push(args);
          await new Promise((r) => setTimeout(r, 300));
          if (window.__cellFail) throw "Apple refused that change.";
          window.__table[args.row][args.column] = args.text;
          return window.__table.map((r) => r.slice());
        }
        // `__noText`: a note the library holds no text for (not yet arrived, or unreadable).
        case "note_body": return args?.guid === window.__noText ? null : window.__noteBodies?.[args?.guid] ?? (noteList.find((n) => n.guid === args?.guid) ?? {}).body ?? null;
        case "note_row": {
          const n = noteList.find((x) => x.guid === args?.guid);
          return n ? { ...n, body: null } : null;
        }
        case "note_counts": {
          const per = new Map();
          for (const n of noteList) per.set(n.folderGuid, (per.get(n.folderGuid) ?? 0) + 1);
          return [noteList.length, [...per.entries()]];
        }
        case "contact_save": (window.__contactSaves ??= []).push(args); return "C-new";
        case "contact_delete": (window.__contactDeletes ??= []).push(args); return null;
        case "contacts_list": {
          if (!contactsFixture) return [];
          // Opt-in: a search answered with its matches only, as the library answers it.
          const q = window.__contactsSearch && args?.search ? String(args.search).toLowerCase() : "";
          return q ? contactList.filter((c) => [c.firstName, c.lastName, c.company, ...c.methods.map((m) => m.value)].join(" ").toLowerCase().includes(q)) : contactList.slice();
        }
        // These cards alone, those still here: what a pass that named them has read.
        case "contacts_get": {
          if (!contactsFixture) return [];
          const want = new Set(args?.guids ?? []);
          return contactList.filter((c) => want.has(c.guid));
        }
        // Duplicates as contacts_merge.rs finds them, on the fixture: a shared number (its
        // last ten digits), a shared email, or the same name.
        case "contacts_duplicates": {
          const keys = (c) => [
            ...[c.firstName, c.middleName, c.lastName].filter(Boolean).join(" ").toLowerCase().trim() ? [`name:${[c.firstName, c.middleName, c.lastName].filter(Boolean).join(" ").toLowerCase().trim()}`] : [],
            ...(c.methods ?? []).filter((m) => m.kind === "phone").map((m) => m.value.replace(/\D/g, "")).filter((d) => d.length >= 7).map((d) => `phone:${d.slice(-10)}`),
            ...(c.methods ?? []).filter((m) => m.kind === "email").map((m) => `email:${m.value.trim().toLowerCase()}`),
          ];
          const groups = [];
          for (const c of contactList) {
            const mine = keys(c);
            const g = groups.find((x) => x.keys.some((k) => mine.includes(k)));
            if (g) { g.guids.push(c.guid); for (const k of mine) if (g.keys.includes(k)) g.shared.add(k.split(":")[0]); g.keys.push(...mine); }
            else groups.push({ guids: [c.guid], keys: mine, shared: new Set() });
          }
          return groups.filter((g) => g.guids.length > 1).map((g) => ({ guids: g.guids, reasons: ["name", "phone", "email"].filter((r) => g.shared.has(r)) }));
        }
        case "contacts_merge": {
          (window.__merges ??= []).push(args);
          const cards = args.guids.map((id) => contactList.find((c) => c.guid === id)).filter(Boolean);
          const kept = { ...cards[0], methods: [...cards[0].methods] };
          for (const o of cards.slice(1)) {
            for (const k of ["firstName", "lastName", "company", "birthday", "jobTitle", "nickname"]) if (!kept[k] && o[k]) kept[k] = o[k];
            const key = (m) => `${m.kind}:${m.kind === "phone" ? m.value.replace(/\D/g, "").slice(-10) : m.value.trim().toLowerCase()}`;
            for (const m of o.methods) if (!kept.methods.some((x) => key(x) === key(m))) kept.methods.push(m);
            if (o.notes && !(kept.notes ?? "").includes(o.notes)) kept.notes = kept.notes ? `${kept.notes}\n\n${o.notes}` : o.notes;
          }
          (window.__mergeUndo ??= {})[kept.guid] = contactList.slice();
          contactList.splice(0, contactList.length, ...contactList.filter((c) => !args.guids.includes(c.guid) || c.guid === kept.guid).map((c) => (c.guid === kept.guid ? kept : c)));
          return kept.guid;
        }
        case "contacts_merge_undo": {
          (window.__mergeUndos ??= []).push(args);
          const before = window.__mergeUndo?.[args.kept];
          if (!before || window.__mergeSent) return false;
          contactList.splice(0, contactList.length, ...before);
          delete window.__mergeUndo[args.kept];
          return true;
        }
        case "contact_me": return window.__me ?? (contactsFixture ? "c1" : null);
        case "contact_set_me": await new Promise((r) => setTimeout(r, 600)); window.__me = args.guid; return null;
        case "contacts_export": (window.__exports ??= []).push(args); return `/home/test/Downloads/${args.fileName}`;
        case "drive_folder_status": {
          const f = (window.__driveFolder ??= { on: false, path: "/home/test/iCloud Drive", problem: null });
          return f.on && window.__driveWaiting ? { ...f, waiting: window.__driveWaiting } : f;
        }
        case "drive_folder_set":
          if (window.__driveFolderRefuse) return { ...window.__driveFolder, on: false, problem: window.__driveFolderRefuse };
          return (window.__driveFolder = { on: args.on, path: "/home/test/iCloud Drive", problem: null, waiting: args.on ? { files: 0, sending: 0, failure: null } : null });
        case "drive_folder_open": if (window.__driveOpenFail) throw window.__driveOpenFail; (window.__opened ??= []).push("drive"); return null;
        case "photos_folder_status": return window.__photosFolder ??= { on: false, path: "/home/test/iCloud Photos", problem: null };
        case "photos_folder_set": return (window.__photosFolder = { on: args.on, path: "/home/test/iCloud Photos", problem: null });
        case "photos_folder_open": (window.__opened ??= []).push("photos"); return null;
        case "photo_video_zoom": return window.__zoom ?? 1;
        case "photo_video_turn": (window.__turnAsks ??= []).push(args); return window.__turn ?? [1, 0, 0, 1];
        case "contacts_import": (window.__imports ??= []).push(args); return window.__importAnswer ?? [2, 0, 0];
        case "plugin:dialog|open": return window.__pick ?? null;
        case "plugin:dialog|save": window.__saveAsked = args; return window.__savePath ?? null;
        // The dialogs as the app shows them now, from its Rust side (grants.rs).
        case "pick_open": { const p = window.__pick ?? null; return p == null ? null : Array.isArray(p) ? p : [p]; }
        case "pick_save": window.__saveAsked = { ...args, options: { defaultPath: args?.defaultName, filters: args?.filters } }; return window.__savePath ?? null;
        case "note_export_markdown": (window.__exported ??= []).push(args); return null;
        case "contact_lists": return !contactsFixture ? [] : (window.__lists ??= [
          { guid: "L1", name: "Family", members: ["c2"], etag: "e1" },
        ]).map((l) => ({ ...l, members: [...l.members] }));
        case "contact_list_write": {
          // Slow on purpose, as Apple is: the screen must not wait for this to redraw.
          (window.__listWrites ??= []).push(args);
          await new Promise((r) => setTimeout(r, 700));
          if (window.__listFail) throw "Contacts refused the list.";
          const lists = (window.__lists ??= [{ guid: "L1", name: "Family", members: ["c2"], etag: "e1" }]);
          if (!args.guid) {
            const guid = `L${lists.length + 1}`;
            lists.push({ guid, name: args.name ?? "Untitled List", members: args.add ?? [], etag: "e" });
            return guid;
          }
          const l = lists.find((x) => x.guid === args.guid);
          if (args.delete) { window.__lists = lists.filter((x) => x.guid !== args.guid); return args.guid; }
          if (args.name) l.name = args.name;
          l.members = l.members.filter((m) => !(args.remove ?? []).includes(m));
          for (const m of args.add ?? []) if (!l.members.includes(m)) l.members.push(m);
          return l.guid;
        }
        case "drive_thumbnails": return driveFixture ? { "FILE::d": "data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='80' height='60'><rect width='80' height='60' fill='%23eee'/></svg>" } : {};
        case "drive_duplicate": (window.__dups ??= []).push(args.drivewsid); return null;
        // Copy and Paste: each copy asked for, and — where a test keeps folders of its own in
        // __driveIn — the copy landing there under the name __driveCopyName gives it.
        case "drive_copy": {
          (window.__driveCopies ??= []).push(args);
          if (window.__driveCopyDelay) await new Promise((r) => setTimeout(r, window.__driveCopyDelay));
          if (window.__driveCopyFail) throw window.__driveCopyFail;
          const from = [...driveItems, ...Object.values(window.__driveIn ?? {}).flat()].find((i) => i.drivewsid === args.drivewsid);
          const into = window.__driveIn?.[args.destinationId];
          // Named as the backend's copy_name names it, unless the test names it.
          const full = (i) => (i.extension ? `${i.name}.${i.extension}` : i.name).toLowerCase();
          const free = (stem) => !into || !into.some((i) => full(i) === full({ ...from, name: stem }));
          const base = from?.name.match(/^(.+) copy(?: \d+)?$/)?.[1] ?? from?.name;
          let stem = from?.name;
          if (from && !free(stem)) { stem = `${base} copy`; for (let n = 2; !free(stem); n++) stem = `${base} copy ${n}`; }
          if (from && into) into.push({ ...from, drivewsid: `${from.drivewsid}-copy${window.__driveCopies.length}`, parentId: args.destinationId, name: window.__driveCopyName ?? stem });
          return null;
        }
        // The desktop's clipboard as the backend reads it: files copied in the file manager, or text.
        case "clipboard_read":
        case "clipboard_paste": return { files: window.__clipFiles ?? [], text: window.__clipText ?? null };
        case "drive_upload": (window.__driveUploads ??= []).push(args); return [];
        case "storage_info": return { usedBytes: 182196857, totalBytes: 5368709120 };
        // The local copy's room, as settings.rs measures it; clearing takes away only the cached pictures.
        case "local_storage": return (window.__local ??= { library: 13_800_000, photos: 4_812_000_000, notes: 456_000_000, drive: 24_000, other: 7_400_000, total: 5_289_224_000, clearable: 4_480_000_000 });
        case "local_storage_clear": {
          if (window.__clearFail) throw window.__clearFail;
          const l = (window.__local ??= { library: 13_800_000, photos: 4_812_000_000, notes: 456_000_000, drive: 24_000, other: 7_400_000, total: 5_289_224_000, clearable: 4_480_000_000 });
          window.__cleared = (window.__cleared ?? 0) + 1;
          return (window.__local = { ...l, photos: l.photos - (l.clearable - l.notes - 3_000), notes: 0, drive: l.drive - 3_000, total: l.total - l.clearable, clearable: 0 });
        }
        // Photos Cache: a cap set lets the longest unseen go in the background, reported as an event.
        case "photos_cache_status": case "photos_cache_set": {
          const GB = 1024 ** 3;
          const c = (window.__cache ??= { used: 1_900_000_000, cap: 2 * GB, limit: 2 * GB, thumbnails: 160_000_000,
            steps: [512 * 1024 ** 2, GB, 2 * GB, 5 * GB, 10 * GB, 20 * GB, 50 * GB] });
          if (cmd === "photos_cache_set") {
            (window.__capSets ??= []).push(args.bytes);
            c.cap = c.limit = args.bytes;
            if (c.used > c.cap) setTimeout(() => { c.used = Math.floor(c.cap * 0.9); window.__emit?.("quince://photo-cache", { ...c }); }, 100);
          }
          return { ...c };
        }
        case "alerts_shown": return window.__alertsShown ?? true;
        case "diagnostics_save": (window.__diagnostics = (window.__diagnostics ?? 0) + 1); return "/home/user/Downloads/Skykeep diagnostics 2026-10-18 20.00.txt";
        case "account_info": return { name: "Asha Rao", email: "asha@example.com", photoContact: window.__mePhoto ?? null, webAccessAllowed: window.__webAccess ?? true, dataProtection: false, termsUpdateNeeded: Boolean(window.__termsNeeded), repairNeeded: Boolean(window.__repairNeeded) };
        // Asking Apple for the account again: counted, and answering what the test set next.
        case "account_refresh":
          window.__accountRefreshes = (window.__accountRefreshes ?? 0) + 1;
          if (window.__termsAfterRefresh !== undefined) window.__termsNeeded = window.__termsAfterRefresh;
          return { name: "Asha Rao", email: "asha@example.com", photoContact: null, webAccessAllowed: true, dataProtection: false, termsUpdateNeeded: Boolean(window.__termsNeeded), repairNeeded: Boolean(window.__repairNeeded) };
        // Advanced Data Protection's approval: nothing waits on one unless a test says so
        // (`window.__dataAccess`); asking again counts, and answers what the test set next.
        case "data_access_status": return window.__dataAccess ?? { state: "open", services: [], problem: null };
        case "data_access_ask":
          window.__dataAccessAsks = (window.__dataAccessAsks ?? 0) + 1;
          window.__dataAccess = window.__dataAccessAfterAsk ?? window.__dataAccess;
          return window.__dataAccess ?? { state: "open", services: [], problem: null };
        case "settings_get": return { reminderAlerts: true, eventAlerts: true, ...(window.__settings ?? {}) };
        case "settings_set":
          if (window.__settingsFail) throw "Couldn't save that setting.";
          (window.__settings ??= {})[args.key] = args.on;
          return null;
        case "drive_children":
          // A slow read, when a test asks for one.
          if (window.__slowDrive) await new Promise((r) => setTimeout(r, window.__slowDrive));
          // A folder of a test's own, where it keeps one (__driveIn); every folder holds the fixture otherwise.
          if (window.__driveIn?.[args.folderId]) return [...window.__driveIn[args.folderId]];
          // A suite's own extra files (`window.__driveExtra`), for a long list.
          return !driveFixture ? [] : window.__driveExtra ? driveItems.concat(window.__driveExtra) : driveItems;
        case "drive_recents": {
          if (!driveFixture) return [];
          const now = Math.floor(Date.now() / 1000);
          const f = (id, name, ext, opened) => ({ drivewsid: `FILE::r${id}`, parentId: "FOLDER::x", name, extension: ext, kind: "FILE",
            size: 34145, childCount: null, createdAt: now - 9e6, modifiedAt: now - 9e6, etag: "e", openedAt: opened });
          // Today and yesterday by this computer's own midnight: "ten minutes ago" and "a day and
          // an hour ago" fell on yesterday and the day before in the first hour after midnight.
          const midnight = Math.floor(new Date().setHours(0, 0, 0, 0) / 1000);
          const today = midnight + Math.floor((now - midnight) / 2);
          return [f(1, "Invoice-0001", "pdf", today), f(2, "Boarding pass", "pdf", midnight - 12 * 3600), f(3, "Lease", "pdf", now - 40 * 86400)];
        }
        case "drive_trash": return !driveFixture ? [] : (window.__trash ??= [
          { drivewsid: "FILE::t1", parentId: null, name: "Old notes", extension: "pdf", kind: "FILE",
            size: 177000, childCount: null, createdAt: 1786000000, modifiedAt: 1786000000, etag: "t1",
            expiresAt: Math.floor(Date.now() / 1000) + 21 * 86400 - 60, restorePath: "Downloads/Old notes.pdf" },
          { drivewsid: "FILE::t2", parentId: null, name: "Wallpaper", extension: "jpg", kind: "FILE",
            size: 1500000, childCount: null, createdAt: 1786000000, modifiedAt: 1786000000, etag: "t2",
            expiresAt: Math.floor(Date.now() / 1000) + 2 * 86400 - 60, restorePath: "Wallpaper.jpg" },
        ]).slice();
        case "drive_trash_out": {
          // One request for all of them, as Recover All / Delete All send it.
          (window.__trashBatches ??= []).push({ recover: args.recover, ids: args.items.map((i) => i.drivewsid) });
          const ids = new Set(args.items.map((i) => i.drivewsid));
          window.__trash = (window.__trash ?? []).filter((i) => !ids.has(i.drivewsid));
          for (const i of args.items) (window.__trashCalls ??= []).push([args.recover ? "drive_put_back" : "drive_delete_forever", i.drivewsid]);
          return [];
        }
        case "drive_put_back": case "drive_delete_forever":
          window.__trash = (window.__trash ?? []).filter((i) => i.drivewsid !== args.item.drivewsid);
          (window.__trashCalls ??= []).push([cmd, args.item.drivewsid]);
          return null;
        case "drive_sync_folder":
          // A folder deleted on another device: the backend forgets it and says so.
          if (args.folderId === window.__driveGone) throw "This folder was deleted or moved on another device.";
          // Apple busy for the next `__driveBusy` asks (a test's): refused as the backend says it.
          if ((window.__driveBusy ?? 0) > 0) { window.__driveBusy--; (window.__busyAsks ??= []).push(args.folderId); throw "iCloud Drive is busy right now \u2014 try this one again in a minute."; }
          return driveItems.length;
        case "drive_download": (window.__singleDownloads ??= []).push(args.drivewsid); return "/home/user/Downloads/file.pdf";
        // A folder, or a selection, into Downloads (folder_download.rs): kept going until the
        // test says how it ended — `window.__downloadEnd[batch](report)` — playing its progress
        // in with __emit("quince://download", …) meanwhile. Cancel ends it as cancelled.
        case "drive_download_items": {
          (window.__downloadItems ??= []).push(args);
          return await new Promise((resolve) => { (window.__downloadEnd ??= {})[args.batch] = resolve; });
        }
        case "drive_download_cancel": {
          (window.__downloadCancels ??= []).push(args);
          window.__downloadEnd?.[args.batch]?.({ key: "k", saved: [], failed: [], moreFailed: 0, cancelled: true, problem: null, resumable: true });
          return null;
        }
        // Downloads left unfinished last time (`window.__unfinished`), and those let go of.
        case "drive_downloads_unfinished": return window.__unfinished ?? [];
        case "drive_download_forget": (window.__forgotten ??= []).push(args.key); return null;
        case "download_reveal": (window.__revealed ??= []).push(args.path); return null;
        case "drive_move": (window.__moves ??= []).push(args); await new Promise((r) => setTimeout(r, 600)); return null;
        case "drive_create_folder": case "drive_rename": {
          (window.__driveNames ??= []).push(args);
          // Renamed where a test keeps folders of its own (__driveIn).
          const it = cmd === "drive_rename" && Object.values(window.__driveIn ?? {}).flat().find((i) => i.drivewsid === args.drivewsid);
          if (it) it.name = it.extension && args.name.endsWith(`.${it.extension}`) ? args.name.slice(0, -(it.extension.length + 1)) : args.name;
          return null;
        }
        case "drive_delete": {
          // A folder changed on another device since it was shown (`__driveChanged[id]`, as
          // drive::FolderNow): not deleted, but answered as it now stands — unless the delete
          // names that version (`expect`), as "Delete Anyway" does.
          const now = window.__driveChanged?.[args.drivewsid];
          if (now && args.expect !== now.etag) { (window.__driveAsked ??= []).push(args.drivewsid); return { kind: "changed", ...now }; }
          (window.__driveDeleted ??= []).push(args.drivewsid);
          for (const [k, list] of Object.entries(window.__driveIn ?? {})) window.__driveIn[k] = list.filter((i) => i.drivewsid !== args.drivewsid);
          return { kind: "done" };
        }
        case "drive_item": return null;
        case "auth_two_factor_options": return twoFactor;
        case "auth_resend_code":    return null;
        // Counted, so a suite can tell a code was asked for only when the person was there.
        case "auth_request_code":
          window.__codeRequests = (window.__codeRequests ?? 0) + 1;
          return null;
        case "auth_send_sms":       return null;
        case "auth_submit_sms_code": window.__codeVia = `sms:${args?.phoneId}`; return null;
        case "auth_sign_out":    sessionStorage.setItem("__signedOut", JSON.stringify(args)); return null;
        case "search_everything": {
          // A search across every app, as search.rs answers it: every word has to begin a word
          // of the record (case and accents aside); per app the count and a few hits with where
          // the words fall; `only` + `skip` read one app further. `__searchDelay` holds an
          // answer back (ms, or a function of the query) to play answers arriving out of order.
          const fold = (t) => String(t ?? "").normalize("NFD").replace(/[\u0300-\u036f]/g, "").toLowerCase();
          const q = fold(args.query).split(/\s+/).filter(Boolean);
          const words = (t) => [...String(t ?? "").matchAll(/[\p{L}\p{N}\p{M}]+/gu)];
          const has = (t) => { const w = words(t).map((m) => fold(m[0])); return q.every((x) => w.some((y) => y.startsWith(x))); };
          const marks = (t) => words(t).filter((m) => q.some((x) => fold(m[0]).startsWith(x)))
            .map((m) => [m.index, m.index + Math.max(...q.filter((x) => fold(m[0]).startsWith(x)).map((x) => x.length))]);
          const hit = (id, title, o = {}) => ({ id, within: null, title, titleMarks: marks(title), detail: null, detailMarks: [],
            when: null, allDay: false, picture: false, locked: false, ...o, ...(o.detail ? { detailMarks: marks(o.detail) } : {}) });
          const people = contactList.concat(window.__moreContacts ?? []);
          const all = {
            notes: noteList.filter((n) => has(`${n.title} ${n.body ?? ""}`))
              .map((n) => hit(n.guid, n.title, { detail: (n.snippet ?? "").replace(/…$/, ""), when: n.modifiedAt ?? null })),
            calendar: events.filter((e) => has(`${e.title} ${e.location ?? ""}`))
              .map((e) => hit(e.guid, e.title, { within: e.calendarGuid, detail: e.location, when: e.startsAt, allDay: e.allDay })),
            contacts: people.filter((c) => has(`${c.firstName ?? ""} ${c.lastName ?? ""} ${c.company ?? ""} ${(c.methods ?? []).map((m) => m.value).join(" ")}`))
              .map((c) => hit(c.guid, [c.firstName, c.lastName].filter(Boolean).join(" ") || c.company || "No Name", { detail: c.methods?.[0]?.value ?? null })),
            drivews: (driveFixture ? driveItems : []).filter((f) => has(`${f.name}.${f.extension ?? ""}`))
              .map((f) => hit(f.drivewsid, f.extension ? `${f.name}.${f.extension}` : f.name, { within: f.parentId, when: f.modifiedAt, allDay: true })),
          };
          const each = args.each ?? 5, skip = args.only ? args.skip ?? 0 : 0;
          const groups = Object.entries(all).filter(([app, h]) => h.length && (!args.only || app === args.only))
            .map(([app, h]) => ({ app, count: h.length, hits: h.slice(skip, skip + each) }));
          (window.__searches ??= []).push({ query: args.query, seq: args.seq, only: args.only ?? null, skip });
          const delay = typeof window.__searchDelay === "function" ? window.__searchDelay(args.query) : window.__searchDelay ?? 0;
          const answer = { seq: args.seq ?? 0, groups };
          // Answers still on their way, so a suite can wait for the last of them to land.
          if (!delay) return answer;
          window.__searchesOut = (window.__searchesOut ?? 0) + 1;
          return new Promise((r) => setTimeout(() => { window.__searchesOut--; r(answer); }, delay));
        }
        case "drive_kept": return window.__driveFolder?.on ? [...(window.__kept ??= [])] : null;
        case "drive_keep":
          if (!window.__driveFolder?.on) throw "Turn on the Drive folder in Settings to keep files on this computer.";
          window.__kept = (window.__kept ??= []).filter((k) => k !== args.drivewsid).concat(args.on ? [args.drivewsid] : []);
          return [...window.__kept];
        case "login_item_status": return { on: false, command: "\"/usr/bin/quince-app\" --background", honoured: true, startInTray: true, ...(window.__login ?? {}) };
        case "login_item_set":
          if (window.__loginRefuse) throw window.__loginRefuse;
          return (window.__login = { startInTray: true, ...(window.__login ?? {}), on: args.on, command: "\"/usr/bin/quince-app\" --background", honoured: window.__login?.honoured ?? true });
        case "login_start_in_tray_set":
          if (window.__loginRefuse) throw window.__loginRefuse;
          return (window.__login = { on: false, command: "\"/usr/bin/quince-app\" --background", honoured: true, ...(window.__login ?? {}), startInTray: args.on });
        case "account_unsent":   return window.__unsent ?? { changes: 0, files: 0, photos: 0 };
        // Changes waiting for Apple, and ones it refused: a test sets `window.__outbox`.
        case "outbox_status":    return window.__outbox ?? { waiting: [], refused: [] };
        // The desktop's Increase Contrast, from the Settings portal (appearance.rs).
        case "system_contrast":  return window.__contrast ?? false;
        // LC_TIME, LANG (appearance.rs): null leaves the engine's own language.
        case "system_time_locale": return window.__timeLocale ?? null;
        case "network_reach": return { accounts: window.__reachAccounts ?? [], apple: ["apple.com", "icloud.com", "icloud-content.com"], subscribed: window.__reachSubscribed ?? [],
          updates: (window.__update?.install ?? "here") === "here" ? ["github.com", "release-assets.githubusercontent.com", "objects.githubusercontent.com"] : [] };
        // Skykeep's own updates (updates.rs). A test sets `window.__update` (an offer, an install
        // kind); Update and Restart plays the download, then the wait (`__updateHold`: "changes"
        // or "typing", lifted by setting it null), then installing. `__updateFail` fails the download.
        case "update_status": case "update_check": case "update_apply": case "update_later":
        case "update_auto_set": case "update_typing": {
          const u = (window.__update ??= {});
          const base = { install: "here", auto: true, current: "0.1.0", phase: "current", version: null, notes: null,
            received: 0, total: null, waitingFor: null, problem: null, checkedAt: Math.floor(Date.now() / 1000) - 3600, prompt: false };
          const st = (window.__updateState ??= { ...base, ...u, ...(u.version && !u.phase ? { phase: "available" } : {}) });
          const tell = () => window.__emit?.("quince://update", null);
          const shown = () => ({ ...st, prompt: ["available", "ready", "downloading", "waiting", "installing"].includes(st.phase) && !st.putOff
            || (st.phase === "failed" && st.inNotice) });
          (window.__updateCalls ??= []).push([cmd, args]);
          if (cmd === "update_typing") { window.__typing = args.typing; return null; }
          if (cmd === "update_auto_set") { st.auto = args.on; tell(); return null; }
          if (cmd === "update_later") { st.putOff = true; if (st.phase === "downloading" || st.phase === "failed") st.phase = "available"; st.problem = null; tell(); return null; }
          if (cmd === "update_check") {
            if (st.install !== "here") throw "Updates come through your package manager.";
            if (window.__updateCheckFail) { Object.assign(st, { phase: "failed", problem: window.__updateCheckFail, inNotice: false }); return shown(); }
            if (window.__updateOffer) Object.assign(st, { phase: "available", putOff: false, ...window.__updateOffer });
            else st.phase = "current";
            st.checkedAt = Math.floor(Date.now() / 1000);
            return shown();
          }
          if (cmd === "update_apply") {
            st.putOff = false;
            const step = (ms) => new Promise((r) => setTimeout(r, ms));
            (async () => {
              Object.assign(st, { phase: "downloading", received: 0, total: 1000, problem: null }); tell();
              for (const n of [250, 600, 1000]) { await step(window.__updateStepMs ?? 150); st.received = n; tell(); }
              if (window.__updateFail) { Object.assign(st, { phase: "failed", problem: window.__updateFail, inNotice: true }); tell(); return; }
              while (window.__updateHold && !st.putOff) { Object.assign(st, { phase: "waiting", waitingFor: window.__updateHold }); tell(); await step(150); }
              if (st.putOff) { st.phase = "ready"; tell(); return; }
              Object.assign(st, { phase: "installing", waitingFor: null }); tell();
              window.__restarted = true;
            })();
            return null;
          }
          return shown();
        }
        case "outbox_resend": case "outbox_discard": {
          (window.__outboxCalls ??= []).push([cmd, args.id]);
          if (window.__outbox) window.__outbox = { ...window.__outbox, refused: window.__outbox.refused.filter((r) => r.id !== args.id) };
          return null;
        }
        // One refresh covers every service, so there is one command to stub rather than
        // a pair per screen.
        case "sync_all":
        case "sync_status":
          return {
            services: [
              { service: "calendar", label: "Calendar", outcome: "ok", detail: null, lastSyncedAt: Math.floor(Date.now() / 1000) - 120 },
              { service: "reminders", label: "Reminders", outcome: "ok", detail: null, lastSyncedAt: Math.floor(Date.now() / 1000) - 60 },
              { service: "notes", label: "Notes", outcome: "ok", detail: null, lastSyncedAt: Math.floor(Date.now() / 1000) - 90 },
              { service: "contacts", label: "Contacts", outcome: "ok", detail: null, lastSyncedAt: Math.floor(Date.now() / 1000) - 300 },
              // A suite's own services as a pass reported them (`window.__syncExtra`); one named
              // here replaces the default for that service.
              ...(window.__syncExtra ?? []),
            ].filter((r, i, all) => all.findLastIndex((x) => x.service === r.service) === i),
            asOf: Math.floor(Date.now() / 1000) - 300,
            tookMs: 40,
          };
        // The event plugin is how a finished refresh reaches the screens. Nothing emits in
        // a plain browser, so listening is a no-op that hands back an unsubscribe id.
        case "plugin:event|listen": {
          // Kept, so a test can play a finished refresh in: `window.__emit(event, payload)`.
          const all = (window.__listeners ??= {});
          (all[args.event] ??= []).push(args.handler);
          window.__emit ??= (event, payload) => (window.__listeners[event] ?? []).forEach((h) => typeof h === "function" && h({ event, payload, id: 0 }));
          return 0;
        }
        case "plugin:event|unlisten":
          return null;
        // Contacts: a card's postal addresses, part by part (contacts_cmds.rs). Zoya (c1) has a
        // home in Germany and a work address in India; __addressFail turns the next write down.
        case "contact_addresses": {
          const book = (window.__addresses ??= !contactsFixture ? {} : { c1: [
            { label: "HOME", street: "Hauptstraße 5\nHinterhaus", subLocality: null, city: "Berlin", subAdministrativeArea: null,
              state: null, postalCode: "10115", country: "Germany", countryCode: "de" },
            { label: "WORK", street: "12 MG Road", subLocality: null, city: "Bengaluru", subAdministrativeArea: null,
              state: "Karnataka", postalCode: "560001", country: "India", countryCode: "in" },
          ] });
          return (book[args.guid] ?? []).map((a) => ({ ...a }));
        }
        case "contact_address_edit": {
          (window.__addressEdits ??= []).push(args);
          await new Promise((r) => setTimeout(r, window.__addressDelay ?? 400));
          if (window.__addressFail) { window.__addressFail = false; throw "Apple turned that change down."; }
          const list = (args.base ?? window.__addresses?.[args.guid] ?? []).map((a) => ({ ...a }));
          for (const e of args.edits) {
            if (e.op === "setField") list[e.index][e.field] = e.value;
            else if (e.op === "setLabel") list[e.index].label = e.label;
            else if (e.op === "add") list.push({ ...e.address });
            else if (e.op === "remove") list.splice(e.index, 1);
          }
          const kept = list.filter((a) => Object.entries(a).some(([k, v]) => k !== "label" && v));
          (window.__addresses ??= {})[args.guid] = kept;
          return kept.map((a) => ({ ...a }));
        }
        // Export Everything (src-tauri/src/export.rs): each service told along the way, as the
        // backend tells it; __exportStepMs slows it to catch it mid-way, __exportFail fails it.
        case "library_export_start": {
          window.__exportArgs = args;
          window.__exportCancel = false;
          const services = ["notes", "reminders", "contacts", "calendar", "drive", ...(args.photos ? ["photos"] : [])];
          const wait = (ms) => new Promise((r) => setTimeout(r, ms));
          for (const [i, service] of services.entries()) {
            for (const done of [0, 5, 10]) {
              window.__emit?.("quince://export", { service, step: i + 1, steps: services.length, done, total: 10 });
              await wait(window.__exportStepMs ?? 20);
              if (window.__exportCancel) throw "Export cancelled.";
            }
          }
          if (window.__exportFail) throw window.__exportFail;
          return { path: `${args.dest}/Skykeep Export 2026-10-21 10.30`, notes: 3, reminders: 2, contacts: 1, events: 2,
                   driveFiles: 1, photos: args.photos ? 2 : 0, photosMissing: args.photos ? 1 : 0 };
        }
        case "library_export_cancel": window.__exportCancel = true; return null;
        case "library_export_reveal": window.__exportRevealed = (window.__exportRevealed ?? 0) + 1; return null;
        // Backup (src-tauri/src/backup.rs): off until a folder is set; Hidden only while unlocked,
        // the same lock Photos keeps (`__hiddenOpen`).
        case "backup_status":
          window.__backup ??= { folder: null, hidden: false, drive: true, driveRunning: false, driveNote: null, copied: 0, total: 2140, running: false, problem: null, photosBytes: 0, photosBytesTotal: 7_340_000_000, driveFiles: 0, driveFilesTotal: 0, driveBytes: 0, driveBytesTotal: 0 };
          return { ...window.__backup, hiddenLocked: !window.__hiddenOpen };
        case "backup_reveal": window.__backupRevealed = (window.__backupRevealed ?? 0) + 1; return null;
        case "backup_set": {
          const was = (window.__backup ??= { folder: null, hidden: false, drive: true, driveRunning: false, driveNote: null, copied: 0, total: 2140, running: false, problem: null, photosBytes: 0, photosBytesTotal: 7_340_000_000, driveFiles: 0, driveFilesTotal: 0, driveBytes: 0, driveBytesTotal: 0 });
          if (args.hidden && !was.hidden && !window.__hiddenOpen) throw "Unlock the Hidden album in Photos first, then include it.";
          window.__backupArgs = args;
          window.__backup = { ...was, folder: args.folder, hidden: args.hidden, drive: args.drive, running: Boolean(args.folder), copied: args.folder ? 812 : 0,
            photosBytes: args.folder ? 2_610_000_000 : 0, driveRunning: Boolean(args.folder && args.drive),
            driveFiles: args.folder ? 1203 : 0, driveFilesTotal: args.folder ? 4120 : 0, driveBytes: args.folder ? 1_100_000_000 : 0, driveBytesTotal: args.folder ? 4_100_000_000 : 0 };
          return { ...window.__backup, hiddenLocked: !window.__hiddenOpen };
        }
        // Windows of their own (apps.rs): recorded, as the test can't see another window.
        // Skykeep's window: what each app holds, counted from the library (launcher.rs).
        case "launcher_summary":
          return window.__launcher ?? { notes: 7, remindersToday: 4, eventsToday: 2,
            nextEvent: { title: "Design review", startsAt: Math.floor(Date.now() / 1000) + 5400, allDay: false },
            contacts: 124, photos: 15712 };
        case "app_open": case "app_reveal": case "settings_open":
          (window.__windows ??= []).push({ cmd, ...(args ?? {}) });
          return null;
        default: throw new Error(`unstubbed command: ${cmd}`);
      }
    },
  };
}

/**
 * Open a screen from the sidebar the way a person would — in a narrow window the sidebar is
 * put away, so it is brought back first.
 */
export async function openScreen(page, name) {
  // Every app is a window of its own (docs/apps-as-windows.md): opened as its launcher opens
  // it, by its address. Settings are Skykeep's own window, the address with no app.
  const key = { Notes: "notes", Reminders: "reminders", Calendar: "calendar", Contacts: "contacts", Drive: "drivews", Photos: "photos", Settings: null }[name];
  if (key === undefined) throw new Error(`openScreen: no app named ${name}`);
  const url = new URL(page.url());
  url.search = "";
  if (key) url.searchParams.set("app", key);
  const already = new URL(page.url()).searchParams.get("app") === key;
  if (!already) await page.goto(url.toString(), { waitUntil: "networkidle" });
  // Each app's code arrives on its own; the screen is open once it is drawn.
  const root = { Calendar: ".cx", Notes: ".notes", Reminders: ".rx", Contacts: ".contacts", Drive: ".drive", Photos: ".px", Settings: ".settings" }[name];
  // Playwright's own 30 s: the gate runs six browsers at once, and a window loading two
  // thousand photos beside them took past 10 s there (alone it takes well under one).
  if (root) await page.locator(root).first().waitFor({ state: "attached", timeout: 30000 });
}

/* Settings: Skykeep's own window, the address with no app (docs/apps-as-windows.md). */
export async function openSettings(page) {
  const url = new URL(page.url());
  if (url.searchParams.get("app")) {
    url.search = "";
    await page.goto(url.toString(), { waitUntil: "networkidle" });
  }
  await page.locator(".settings").first().waitFor({ timeout: 10000 });
}

/* Answer every yes-or-no confirmation sheet (components/Confirm) by going ahead — a question
   with several answers is left to the suite — keeping each question's
   title in window.__asked — what page.on("dialog") did for the browser's own confirm(). */
export function autoConfirm() {
  const watch = () => new MutationObserver(() => {
    const sheet = document.querySelector(".confirm:not([data-answered]):not(:has(.is-stacked))");
    if (!sheet) return;
    sheet.setAttribute("data-answered", "");
    (window.__asked ??= []).push(sheet.querySelector("h2")?.textContent ?? "");
    sheet.querySelector(".confirm-answer")?.click();
  }).observe(document.documentElement, { childList: true, subtree: true });
  if (document.documentElement) watch();
  else document.addEventListener("DOMContentLoaded", watch);
}
