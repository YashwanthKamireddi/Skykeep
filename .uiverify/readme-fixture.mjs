// The README's demo library: an invented person's notes, reminders, week, contacts, files and
// photos, so the screenshots show a lived-in app without anyone's real data. Every name, place
// and address here is made up; phone numbers use the 555-01xx range kept for fiction, emails
// and links use example.com/.example, and every picture is drawn by .uiverify/readme-pictures.mjs.
//
// Used only when a suite passes `demoFixture: true` to installStub (stub.mjs) and adds this
// script first: `ctx.addInitScript(installDemo, { picBase, tz, library: photoLibrary() })`.
//
// addInitScript SERIALIZES installDemo: everything it needs is inside it or in its argument.

/**
 * The photo library, planned in one place so the pictures drawn (readme-shots.mjs) and the
 * rows the app reads (installDemo) agree: outings over four months, each a handful of shots
 * of one kind of scene, with a few videos, Live Photos and favourites among them.
 */
export function photoLibrary() {
  const outings = [
    // [days ago, scenes in order]
    [0, ["city", "sea"]],
    [2, ["forest", "forest", "landscape", "forest", "lake"]],
    [4, ["city", "sea", "city", "bloom"]],
    [6, ["tulips", "bloom", "tulips"]],
    [9, ["bloom", "lake"]],
    [12, ["lake", "landscape", "lake", "night", "fire", "night"]],
    [16, ["sea", "beach", "sea"]],
    [19, ["city", "coffee", "city"]],
    [24, ["landscape", "landscape", "forest", "lake"]],
    [29, ["city", "city", "sea", "coffee", "city", "sea"]],
    [34, ["tulips", "bloom", "tulips", "bloom"]],
    [38, ["beach", "beach", "sea", "beach", "sea"]],
    [43, ["coffee", "city"]],
    [47, ["dunes", "dunes", "landscape", "dunes", "night"]],
    [53, ["forest", "lake", "forest"]],
    [58, ["beach", "sea", "beach", "beach", "sea", "beach"]],
    [63, ["aurora", "night", "aurora", "lake"]],
    [69, ["city", "coffee", "city", "city"]],
    [74, ["landscape", "lake", "landscape", "lake", "fire"]],
    [80, ["tulips", "bloom", "tulips"]],
    [86, ["sea", "beach", "dunes", "sea"]],
    [92, ["forest", "forest", "landscape"]],
    [99, ["city", "sea", "city", "coffee"]],
    [104, ["lake", "landscape", "night", "fire", "lake"]],
  ];
  const shapes = [[800, 600], [600, 800], [800, 600], [800, 450], [800, 600], [600, 800], [800, 600]];
  const videos = { 3: 14.2, 17: 41.6, 33: 8.4, 52: 22.9 };
  const live = new Set([1, 6, 11, 20, 27, 39, 46, 61]);
  const favourite = new Set([2, 8, 14, 19, 25, 31, 44, 50, 57, 66, 72, 80]);
  const out = [];
  for (const [daysAgo, scenes] of outings)
    scenes.forEach((scene, k) => {
      const i = out.length;
      const [w, h] = shapes[(i * 5 + k) % shapes.length];
      out.push({ id: `g${i}`, scene, w, h, seed: i + 3, daysAgo, minutes: daysAgo === 0 ? 7 * 60 + 40 + k * 35 : 10 * 60 + (i % 7) * 47 - k * 9,
        kind: videos[i] ? "video" : live.has(i) ? "live" : "image", duration: videos[i] ?? (live.has(i) ? 2.8 : null), favorite: favourite.has(i) });
    });
  return out;
}

export function installDemo({ picBase, tz, library }) {
  const HOUR = 3600, DAY = 86400, MIN = 60;
  const today = new Date(); today.setHours(0, 0, 0, 0);
  const at = (days, h = 0, m = 0) => { const d = new Date(today); d.setDate(d.getDate() + days); d.setHours(h, m, 0, 0); return d.getTime() / 1000; };
  const now = Math.floor(Date.now() / 1000);
  window.__picBase = picBase;

  // ---- Calendar: a busy week, four calendars, a birthday, video calls and reminders on the schedule.
  const calendars = [
    { guid: "work", title: "Work", colour: "#0088ff", ctag: null, sortOrder: 0, enabled: true, writable: true, isDefault: true },
    { guid: "home", title: "Home", colour: "#34c759", ctag: null, sortOrder: 1, enabled: true, writable: true, isDefault: false },
    { guid: "family", title: "Family", colour: "#ff9500", ctag: null, sortOrder: 2, enabled: true, writable: true, isDefault: false },
    { guid: "fitness", title: "Fitness", colour: "#af52de", ctag: null, sortOrder: 3, enabled: true, writable: true, isDefault: false },
    { guid: "quince:birthdays", title: "Birthdays", colour: "#8e8e93", ctag: null, sortOrder: 900, enabled: true, writable: false, isDefault: false },
  ];
  let n = 0;
  const ev = (cal, title, day, h, m, minutes, location = null, extra = {}) => ({
    guid: extra.guid ?? `d${++n}`, calendarGuid: cal, title, location, startsAt: at(day, h, m), endsAt: at(day, h, m) + minutes * MIN,
    allDay: false, timezone: tz, url: extra.url ?? null, notes: extra.notes ?? null, etag: null });
  const allDay = (cal, title, day, days = 1, guid = `a${++n}`) => ({ guid, calendarGuid: cal, title, location: null,
    startsAt: at(day), endsAt: at(day + days), allDay: true, timezone: null, url: null, notes: null, etag: null });
  const standups = [-2, -1, 0, 1, 2].map((d) => ev("work", "Standup", d, 9, 0, 30, null, { guid: `SERIES__standup${d + 2}` }));
  const events = [
    ...standups,
    ev("family", "Brunch", -3, 11, 0, 90, "Juniper Kitchen"),
    ev("fitness", "Long run", -3, 8, 0, 60, "Riverside loop"),
    ev("work", "Design crit", -2, 11, 0, 60, "Studio B"),
    ev("fitness", "Yoga", -2, 18, 0, 60, "Northside Studio"),
    ev("work", "Roadmap", -1, 10, 0, 90, "Room 4"),
    ev("home", "Lunch: Sam", -1, 12, 30, 60, "Corner Café"),
    ev("home", "Dentist", -1, 16, 0, 60, "Brookfield Dental"),
    ev("work", "Q4 review", 0, 10, 30, 60, "Room 4"),
    ev("family", "Lunch: Mum", 0, 12, 30, 60, "Olive & Fig"),
    ev("work", "1:1 Jordan", 0, 15, 0, 45, null, { url: "https://zoom.us/j/5550100142" }),
    ev("fitness", "Climbing", 0, 17, 30, 90, "Boulder Hall"),
    ev("work", "Interviews", 1, 11, 0, 90, null, { guid: "d-meet", url: "https://meet.google.com/xqv-mbtr-kpe",
      notes: "Three customers, thirty minutes each. Ask about the first week." }),
    ev("home", "Bike service", 1, 14, 0, 60, "Spoke & Chain"),
    ev("family", "Movie night", 1, 19, 0, 120),
    ev("work", "Sprint demo", 2, 10, 0, 60, "Room 4"),
    ev("work", "Focus time", 2, 13, 0, 120),
    ev("family", "Swim lessons", 3, 10, 0, 60, "Leisure Centre"),
    ev("family", "Market", 3, 11, 30, 90, "Market Square"),
    ev("fitness", "Yoga", 1, 7, 30, 60, "Northside Studio"),
    allDay("family", "Cabin weekend", 2, 3),
    allDay("quince:birthdays", "Maya Chen’s Birthday", 1, 1, "quince:birthdays:c3:2026"),
  ];
  const dueReminders = [
    { guid: "r2", listGuid: "trip", title: "Book the train tickets", notes: null, dueAt: at(0, 14, 0), completedAt: null, priority: 0, etag: null },
    { guid: "r7", listGuid: "errands", title: "Pick up the dry cleaning", notes: null, dueAt: at(1, 17, 0), completedAt: null, priority: 0, etag: null },
  ];

  // ---- Reminders: colourful lists, tags, sections, subtasks, a link, a place and an early alert.
  const reminderLists = [
    { guid: "home", title: "Home", colour: "#007aff", ctag: null, sortOrder: 0, enabled: true, emblem: "house" },
    { guid: "work", title: "Work", colour: "#af52de", ctag: null, sortOrder: 1, enabled: true, emblem: null },
    { guid: "trip", title: "Cabin Weekend", colour: "#34c759", ctag: null, sortOrder: 2, enabled: true, emblem: "nature" },
    { guid: "errands", title: "Errands", colour: "#ff9500", ctag: null, sortOrder: 3, enabled: true, emblem: null },
    { guid: "groceries", title: "Groceries", colour: "#ff3b30", ctag: null, sortOrder: 4, enabled: true, emblem: null },
    { guid: "reading", title: "Reading", colour: "#a2845e", ctag: null, sortOrder: 5, enabled: true, emblem: "book" },
    { guid: "training", title: "Training", colour: "#ff2d55", ctag: null, sortOrder: 6, enabled: true, emblem: "fitness" },
  ];
  const R = (guid, listGuid, title, dueAt, extra = {}) => ({ guid, listGuid, title, notes: null, dueAt, completedAt: null,
    priority: 0, flagged: false, parentGuid: null, recurFrequency: null, recurInterval: null, sectionGuid: null, allDay: false, etag: null, ...extra });
  // Apple's priorities: 1 high, 5 medium, 9 low.
  const reminderItems = [
    R("r1", "home", "Call the plumber about the sink", at(0, 8, 30), { priority: 1 }),
    R("r2", "trip", "Book the train tickets", at(0, 14, 0), { flagged: true, priority: 5, notes: "Window seats if we can", sectionGuid: "s-before" }),
    R("r2a", "trip", "Compare the 4:10 and the 5:40", null, { parentGuid: "r2", sectionGuid: "s-before" }),
    R("r13", "trip", "Print the trail map", at(1, 19, 0), { sectionGuid: "s-before" }),
    R("r6", "trip", "Pack the camping gear", at(2, 8, 0), { sectionGuid: "s-pack", flagged: true }),
    R("r6a", "trip", "Headlamps and spare batteries", null, { parentGuid: "r6", sectionGuid: "s-pack" }),
    R("r6b", "trip", "Sleeping bags", null, { parentGuid: "r6", sectionGuid: "s-pack" }),
    R("r6c", "trip", "Stove and fuel", null, { parentGuid: "r6", sectionGuid: "s-pack" }),
    R("r16", "trip", "Turn the heating down", at(2, 15, 30), { sectionGuid: "s-before" }),
    R("r3", "work", "Send the Q4 slides to Jordan", at(0, 11, 30), { priority: 1 }),
    R("r4", "errands", "Return the library books", at(0), { allDay: true }),
    R("r5", "home", "Water the herbs", at(0, 18, 0), { recurFrequency: 0, recurInterval: 1 }),
    R("r7", "errands", "Pick up the dry cleaning", at(1, 17, 0)),
    R("r17", "errands", "Post the birthday card", at(0, 16, 0), { flagged: true }),
    R("r8", "groceries", "Oat milk", null), R("r9", "groceries", "Basil", null), R("r10", "groceries", "Lemons", null),
    R("r18", "groceries", "Sourdough loaf", null), R("r19", "groceries", "Coffee beans", null),
    R("r11", "work", "Review the onboarding draft", at(2, 9, 30)),
    R("r20", "work", "Book a room for the offsite", at(5, 10, 0)),
    R("r12", "home", "Renew the car insurance", at(9, 9, 0), { flagged: true }),
    R("r21", "home", "Book the boiler service", at(12, 9, 0)),
    R("r22", "reading", "Piranesi", null), R("r23", "reading", "The Overstory", null), R("r24", "reading", "A Psalm for the Wild-Built", null),
    R("r25", "training", "Easy 5 km", at(1, 7, 0), { recurFrequency: 1, recurInterval: 1 }),
    R("r26", "training", "Stretch after climbing", at(0, 19, 30)),
  ];
  window.__tagMap = { r2: ["trip"], r3: ["work"], r4: ["errands"], r5: ["home"], r6: ["trip", "weekend"], r7: ["errands"], r13: ["trip"],
    r17: ["errands"] };
  window.__sections = [
    { guid: "s-before", listGuid: "trip", title: "Before we go", createdAt: null },
    { guid: "s-pack", listGuid: "trip", title: "Packing", createdAt: null },
  ];
  window.__smartLists = [
    { guid: "SL-today", kind: "today", title: null, colour: null, emblem: null, pinned: true, sorting: null, filter: null },
    { guid: "SL-1", kind: "custom", title: "This Weekend", colour: "#5856d6", emblem: null, pinned: false, sorting: null, filter: '{"hashtags":["weekend"]}' },
  ];
  window.__details = { r2: { url: "https://rail.example/tickets/fern-hollow", earlyAlerts: [{ id: "a1", count: 1, unit: 1 }] } };
  window.__marks = {
    r2: { url: "https://rail.example/tickets/fern-hollow" },
    r16: { location: { title: "Home", proximity: "leaving", kind: "custom" } },
  };

  // ---- Notes: folders, pinned notes, and a note with a link, a tag, a picture, a checklist and a table.
  const noteFolders = [
    { guid: "f1", title: "Notes", sortOrder: 0 },
    { guid: "f3", title: "Travel", sortOrder: 1 },
    { guid: "f4", title: "Recipes", sortOrder: 2 },
    { guid: "f5", title: "Work", sortOrder: 3 },
    { guid: "f6", title: "Home", sortOrder: 4 },
    { guid: "f7", title: "Journal", sortOrder: 5 },
    { guid: "f2", title: "Recently Deleted", sortOrder: 9 },
  ];
  // A line is [text, style, checked, attachment]; text may be a list of [piece, marks] for a
  // line that mixes plain words with a link or an inline #tag. Styles as Apple numbers them:
  // 0 title, 1 heading, 2 subheading, 100 dashed, 101 bulleted, 102 numbered, 103 checklist.
  const cabin = [
    ["Cabin weekend", 0],
    [[["Friday to Sunday at Fern Hollow. Leave by four to miss the traffic. "], ["￼", { attachment: "H1" }]], null],
    [[["Cabin 7, booked on "], ["fernhollow.example", { link: "https://fernhollow.example/cabin-7" }]], null],
    ["￼", null, null, "M1"],
    ["Packing", 1],
    ["Hiking boots", 103, true], ["Rain jackets", 103, true], ["Camera and a spare battery", 103, true],
    ["Board games", 103, false], ["Coffee beans and the grinder", 103, false],
    ["Who brings what", 1],
    ["￼", null, null, "T1"],
    ["", null],
  ];
  const notes = {
    nC: { lines: cabin, folder: "f3", pinned: true, modified: now - 25 * MIN },
    nS: { lines: [["Sourdough starter", 0], ["Feed it every morning: 50 g flour, 50 g water.", null], ["Bake when it doubles in four hours", 103, false], ["Rye works too, it just smells sharper", 101]], folder: "f4", pinned: true, modified: now - 2 * DAY },
    nW: { lines: [["Wi-Fi and house codes", 0], ["Guest network: Rivera-Guest", null], ["Bins go out on Tuesday night", 101]], folder: "f6", pinned: true, modified: now - 40 * DAY },
    nB: { lines: [["Book club picks", 0], ["November: The Overstory. December: Piranesi.", null], ["Ask Lena to host", 103, false]], folder: "f1", modified: now - 3 * HOUR },
    nJ: { lines: [["Climbing day", 0], ["Climbed the green route clean for the first time. Sore hands, happy.", null]], folder: "f7", modified: now - 19 * HOUR },
    nQ: { lines: [["Q4 planning", 0], ["Ship the new onboarding by mid-November.", null], ["Hire a second designer", 103, false]], folder: "f5", modified: now - 26 * HOUR },
    nG: { lines: [["Gift ideas", 0], ["Sam: a good pour-over kettle", 101], ["Mum: the blue scarf from the market", 101], ["Maya: climbing chalk bag", 101]], folder: "f1", modified: now - 3 * DAY },
    nL: { lines: [["Lisbon in spring", 0], ["Tram 28 early, before the crowds. Custard tarts in Belém.", null]], folder: "f3", modified: now - 5 * DAY },
    nP: { lines: [["Garden plan", 0], ["Tulip bulbs in before the first frost. Move the rosemary to the sun.", null]], folder: "f6", modified: now - 6 * DAY },
    nM: { lines: [["Design sync", 0], ["Tighter spacing in the sidebar, and calmer empty states.", null]], folder: "f5", modified: now - 8 * DAY },
    nK: { lines: [["Bike maintenance", 0], ["Chain lube every 300 km. Brake pads at 2 mm.", null]], folder: "f1", modified: now - 12 * DAY },
    nT: { lines: [["Kyoto, someday", 0], ["Philosopher’s Path in April. Stay near Gion.", null]], folder: "f3", modified: now - 16 * DAY },
    nR: { lines: [["Tomato soup", 0], ["Roast the tomatoes first, with garlic and thyme.", null]], folder: "f4", modified: now - 20 * DAY },
    nN: { lines: [["Pancakes", 0], ["Buttermilk, a pinch of salt, rest the batter ten minutes.", null]], folder: "f4", modified: now - 33 * DAY },
    nO: { lines: [["Offsite ideas", 0], ["A day at the lake, with one long walk and no laptops.", null]], folder: "f5", modified: now - 45 * DAY },
  };
  const textOf = (l) => (Array.isArray(l[0]) ? l[0].map((p) => p[0]).join("") : l[0]);
  const toNote = (guid, n) => {
    const text = n.lines.map(textOf).join("\n");
    const rest = n.lines.slice(1).map(textOf).map((t) => t.replace(/￼/g, "").trim()).filter(Boolean).join(" ");
    return { guid, folderGuid: n.folder, title: textOf(n.lines[0]), snippet: rest, body: text, locked: false, editable: true,
      pinned: !!n.pinned, createdAt: n.modified - 30 * DAY, modifiedAt: n.modified, etag: null };
  };
  const noteList = Object.entries(notes).map(([g, n]) => toNote(g, n));
  // The note that arrives from "the phone" in the hero animation (readme-shots.mjs adds it).
  const arriving = { lines: [["Farmers market", 0], ["Saturday morning, before swim lessons", null], ["Sourdough loaf", 103, false], ["Honey", 103, false], ["Apples for the pie", 103, false], ["Flowers for Maya", 103, false]], folder: "f1", modified: now };
  window.__demoArrive = () => { const list = window.__demo.noteList; if (!list.some((x) => x.guid === "nF")) { notes.nF = arriving; list.unshift(toNote("nF", arriving)); } };
  window.__table = [["Who", "Brings"], ["Sam", "Firewood"], ["Alex", "Saturday breakfast"], ["Maya", "Games and snacks"]];
  const plain = { indent: 0, bold: false, italic: false, underline: false, strike: false, link: null };
  const runsOf = (n) => n.lines.flatMap((l, i) => {
    const nl = i < n.lines.length - 1 ? 1 : 0;
    const line = { style: l[1], checked: l[1] === 103 ? !!l[2] : null };
    if (!Array.isArray(l[0]))
      return [{ ...plain, ...line, length: [...l[0]].length + nl, ...(l[3] ? { attachment: l[3] } : {}) }];
    return l[0].map(([piece, marks = {}], k) => ({ ...plain, ...line, ...marks, length: [...piece].length + (k === l[0].length - 1 ? nl : 0) }));
  });

  // ---- Contacts: faces on most cards, lists, and one card with everything on it.
  const C = (guid, firstName, lastName, methods = [], extra = {}) => ({ guid, firstName, middleName: null, lastName,
    photoUrl: extra.face ? `demo:${guid}` : null, photoSignature: extra.face ? guid : null,
    company: null, isCompany: false, birthday: null, notes: null, etag: null, methods, ...extra });
  const phone = (label, last) => ({ kind: "phone", label, value: `+1 (555) 01${last}` });
  const contactList = [
    C("c1", "Alex", "Brennan", [phone("mobile", "0-2231")], { face: true }),
    C("c2", "Priya", "Desai", [{ kind: "email", label: "home", value: "priya.desai@example.com" }], { face: true }),
    C("c3", "Maya", "Chen", [
      phone("mobile", "0-4417"),
      phone("work", "0-9020"),
      { kind: "email", label: "home", value: "maya.chen@example.com" },
      { kind: "email", label: "work", value: "maya@studionorth.example" },
      { kind: "url", label: "homepage", value: "mayachen.example" },
      { kind: "related", label: "brother", value: "Leo Chen" },
      { kind: "related", label: "partner", value: "Sam Garcia" },
    ], { face: true, company: "Studio North", jobTitle: "Product Designer", birthday: "1991-10-22", notes: "Met at the climbing gym. Loves the sourdough." }),
    C("c4", "Jordan", "Ellis", [phone("work", "0-7788")], { company: "Studio North", face: true }),
    C("c5", "Noah", "Fischer", [phone("mobile", "0-3304")]),
    C("c6", "Sam", "Garcia", [phone("mobile", "0-6612")], { face: true }),
    C("c7", "Lena", "Hart", [{ kind: "email", label: "home", value: "lena.hart@example.com" }], { face: true }),
    C("c8", "Theo", "Ivanov", [phone("mobile", "0-1180")]),
    C("c9", "Ada", "Kim", [phone("mobile", "0-5521")], { face: true }),
    C("c10", "Ruth", "Lindqvist", [phone("home", "0-0045")], { face: true }),
    C("c11", "Ben", "Morales", [{ kind: "email", label: "home", value: "ben.morales@example.com" }]),
    C("c12", "Chidi", "Okafor", [phone("mobile", "0-8890")], { face: true }),
    C("c13", "Jamie", "Rivera", [{ kind: "email", label: "home", value: "jamie.rivera@example.com" }], { face: true }),
    C("c14", "Hana", "Sato", [phone("mobile", "0-2675")], { face: true }),
    C("c15", null, null, [phone("work", "0-0100")], { company: "Spoke & Chain Bikes", isCompany: true }),
    C("c16", "Leo", "Chen", [phone("mobile", "0-4410")]),
    C("c17", "Grace", "Adeyemi", [phone("mobile", "0-3391")], { face: true }),
    C("c18", "Oscar", "Bell", [phone("mobile", "0-7720")]),
    C("c19", "Elena", "Costa", [phone("mobile", "0-6604")], { face: true }),
    C("c20", "Marcus", "Dlamini", [phone("mobile", "0-2219")]),
    C("c21", "Isla", "Fraser", [phone("mobile", "0-5583")], { face: true }),
    C("c22", "Kai", "Nakamura", [phone("mobile", "0-1147")]),
    C("c23", "Zara", "Patel", [phone("mobile", "0-9035")], { face: true }),
    C("c24", "Owen", "Walsh", [phone("mobile", "0-6458")]),
    C("c25", null, null, [phone("work", "0-0177")], { company: "Brookfield Dental", isCompany: true }),
  ];
  window.__lists = [
    { guid: "L1", name: "Family", members: ["c10", "c13", "c16"], etag: "e1" },
    { guid: "L2", name: "Climbing club", members: ["c3", "c6", "c9", "c21"], etag: "e2" },
    { guid: "L3", name: "Studio North", members: ["c3", "c4", "c19"], etag: "e3" },
    { guid: "L4", name: "Book club", members: ["c7", "c2", "c17", "c23"], etag: "e4" },
  ];
  window.__me = "c13";
  window.__addresses = { c3: [{ label: "HOME", street: "48 Willow Lane", subLocality: null, city: "Brookfield", subAdministrativeArea: null,
    state: "OR", postalCode: "97000", country: "United States", countryCode: "us" }] };

  // ---- Drive: a folder of a summer trip, with pictures, documents and their thumbnails.
  // Ids carry iCloud Drive's own zone, as Apple's do, so the folder takes uploads.
  const Z = "com.apple.CloudDocs";
  const F = (key, parent, name, extension, kind, size, modified, childCount = null) => ({ drivewsid: `${kind === "FILE" ? "FILE" : kind === "APP_LIBRARY" ? "APPLIB" : "FOLDER"}::${Z}::${key}`,
    parentId: parent ? `FOLDER::${Z}::${parent}` : null, name, extension, kind, size, childCount, createdAt: modified - 40 * DAY, modifiedAt: modified, etag: "e" });
  const drive = {
    root: [
      F("lake", null, "Summer at the Lake", null, "FOLDER", null, now - 2 * HOUR, 17),
      F("docs", null, "Documents", null, "FOLDER", null, now - 3 * DAY, 24),
      F("desk", null, "Desktop", null, "FOLDER", null, now - DAY, 6),
      F("recipes", null, "Recipes", null, "FOLDER", null, now - 9 * DAY, 12),
      F("budget", null, "Household budget", "numbers", "FILE", 412300, now - 4 * DAY),
      F("portfolio", null, "Portfolio", "pdf", "FILE", 2840000, now - 15 * DAY),
    ],
    lake: [
      F("receipts", "lake", "Receipts", null, "FOLDER", null, now - 2 * DAY, 4),
      F("fromsam", "lake", "From Sam", null, "FOLDER", null, now - 3 * DAY, 23),
      F("lakedawn", "lake", "Lake at dawn", "jpg", "FILE", 3412000, now - 2 * HOUR),
      F("ridge", "lake", "Ridge trail", "jpg", "FILE", 2980000, now - 3 * HOUR),
      F("fire", "lake", "Campfire", "jpg", "FILE", 2210000, now - 5 * HOUR),
      F("aurora", "lake", "Northern lights", "jpg", "FILE", 3540000, now - 6 * HOUR),
      F("stars", "lake", "Stars over the lake", "jpg", "FILE", 2650000, now - 7 * HOUR),
      F("mist", "lake", "Morning mist", "jpg", "FILE", 2870000, now - 8 * HOUR),
      F("pines", "lake", "Pines", "jpg", "FILE", 2760000, now - 9 * HOUR),
      F("porch", "lake", "Coffee on the porch", "jpg", "FILE", 1980000, now - 10 * HOUR),
      F("shore", "lake", "The shore", "jpg", "FILE", 3010000, now - 11 * HOUR),
      F("map", "lake", "Trail map", "pdf", "FILE", 1840000, now - DAY),
      F("plan", "lake", "Weekend plan", "pdf", "FILE", 220000, now - DAY),
      F("packing", "lake", "Packing list", "pages", "FILE", 310000, now - DAY),
      F("costs", "lake", "Shared costs", "numbers", "FILE", 98000, now - 2 * DAY),
      F("slides", "lake", "Trip slideshow", "key", "FILE", 8200000, now - 2 * DAY),
      F("songs", "lake", "Road trip songs", "txt", "FILE", 2100, now - 3 * DAY),
    ],
  };
  const children = (folderId) => drive[String(folderId ?? "").split("::").pop()] ?? drive.root;
  const driveById = new Map(Object.values(drive).flat().map((i) => [i.drivewsid, i]));
  const thumbs = Object.fromEntries(["lakedawn", "ridge", "fire", "aurora", "stars", "mist", "pines", "porch", "shore", "map", "plan", "packing", "costs", "slides", "budget", "portfolio"]
    .map((k) => [`FILE::${Z}::${k}`, `${picBase}/drive/${k}`]));

  // ---- Photos: the planned library (photoLibrary), favourites, videos, Live Photos and albums.
  window.__photos = library.map((p) => {
    const takenAt = at(-p.daysAgo) + p.minutes * MIN;
    const video = p.kind === "video";
    return { id: p.id, master: `m${p.id}`, takenAt, addedAt: takenAt, kind: p.kind, width: p.w * 5, height: p.h * 5, duration: p.duration,
      filename: `IMG_${4100 + Number(p.id.slice(1))}.${video ? "MOV" : "HEIC"}`, size: video ? 38_000_000 : 2_400_000,
      uti: video ? "com.apple.quicktime-movie" : "public.heic", favorite: p.favorite, hidden: false, deleted: false, deletedAt: null, subtype: 0, changeTag: null };
  }).filter((p) => p.takenAt <= now);
  const ofScene = (...scenes) => library.filter((p) => scenes.includes(p.scene)).map((p) => p.id);
  window.__albums = [
    { id: "AL1", name: "Summer at the Lake", position: 0, changeTag: null, items: ofScene("lake", "fire", "night").slice(0, 14) },
    { id: "AL2", name: "Sunsets", position: 1, changeTag: null, items: ofScene("sea", "dunes").slice(0, 12) },
    { id: "AL3", name: "Lisbon", position: 2, changeTag: null, items: ofScene("city").slice(0, 16) },
    { id: "AL4", name: "Garden", position: 3, changeTag: null, items: ofScene("tulips", "bloom").slice(0, 9) },
    { id: "AL5", name: "Coffee", position: 4, changeTag: null, items: ofScene("coffee") },
  ];

  // ---- Settings and freshness.
  const services = ["notes", "reminders", "calendar", "contacts", "drive", "photos"];
  const labels = { notes: "Notes", reminders: "Reminders", calendar: "Calendar", contacts: "Contacts", drive: "Drive", photos: "Photos" };
  const report = () => ({ services: services.map((s, i) => ({ service: s, label: labels[s], outcome: "ok", detail: null,
    lastSyncedAt: Math.floor(Date.now() / 1000) - 6 - i * 3 })), asOf: Math.floor(Date.now() / 1000) - 6, tookMs: 40 });
  window.__local = { library: 48_200_000, photos: 1_912_000_000, notes: 86_000_000, drive: 640_000_000, other: 7_400_000, total: 2_693_600_000, clearable: 2_100_000_000 };

  window.__demo = {
    events, calendars, dueReminders, reminderLists, reminderItems, noteFolders, noteList, contactList,
    // Drive items for the stub's own search; the folders are answered below.
    driveItems: drive.root,
    answer(cmd, args) {
      // The stub's own arrays, which it shares back after taking these rows in.
      const reminderItems = window.__demo.reminderItems;
      switch (cmd) {
        case "note_runs": return notes[args.guid] ? runsOf(notes[args.guid]) : undefined;
        case "note_body": return notes[args.guid] ? notes[args.guid].lines.map(textOf).join("\n") : undefined;
        case "note_attachment_info":
          if (args.id === "T1") return { id: "T1", uti: "com.apple.notes.table", kind: "table", filename: null, title: null, width: null, height: null, size: 1200, hasPreview: false };
          if (args.id === "M1") return { id: "M1", uti: "public.jpeg", kind: "image", filename: "Porch.jpeg", title: null, width: 1600, height: 560, size: 412000, hasPreview: true };
          if (args.id === "H1") return { id: "H1", uti: "com.apple.notes.inlinetextattachment.hashtag", kind: "inline", filename: null, title: "#trips", width: null, height: null, size: null, hasPreview: false };
          return undefined;
        // The launcher's tiles, read off the same demo library the apps show.
        case "launcher_summary": {
          const start = at(0), end = at(1), now = Math.floor(Date.now() / 1000);
          const today = events.filter((e) => e.startsAt < end && e.endsAt > start);
          const next = today.filter((e) => !e.allDay && e.startsAt >= now).sort((a, b) => a.startsAt - b.startsAt)[0] ?? null;
          return {
            notes: window.__demo.noteList.length,
            remindersToday: reminderItems.filter((r) => !r.completedAt && !r.parentGuid && r.dueAt !== null && r.dueAt < end).length,
            eventsToday: today.length,
            nextEvent: next && { title: next.title, startsAt: next.startsAt, allDay: false },
            contacts: window.__demo.contactList.length,
            photos: (window.__photos ?? []).length,
          };
        }
        case "reminder_counts": {
          const open = reminderItems.filter((r) => !r.completedAt && !r.parentGuid);
          const end = at(1);
          return { today: open.filter((r) => r.dueAt !== null && r.dueAt < end).length, scheduled: open.filter((r) => r.dueAt !== null).length,
            flagged: open.filter((r) => r.flagged).length, all: open.length,
            lists: reminderLists.map((l) => [l.guid, open.filter((r) => r.listGuid === l.guid).length]) };
        }
        case "reminder_groups": return [];
        case "reminders_smart": {
          const open = reminderItems.filter((r) => !r.completedAt);
          const pick = { today: (r) => r.dueAt !== null && r.dueAt < args.until, scheduled: (r) => r.dueAt !== null, flagged: (r) => r.flagged, all: () => true }[args.kind] ?? (() => true);
          // As Today shows them on a phone: the reminders due, without their undated subtasks.
          return open.filter((r) => !r.parentGuid && pick(r));
        }
        case "reminders_in": {
          const all = reminderItems.filter((r) => r.listGuid === args.listGuid);
          return args.includeDone ? all : all.filter((r) => !r.completedAt);
        }
        case "reminder_set_done": {
          const r = reminderItems.find((x) => x.guid === args.guid);
          if (r) r.completedAt = args.done ? Math.floor(Date.now() / 1000) : null;
          return undefined;
        }
        case "reminders_in_smart_list":
          return { reminders: reminderItems.filter((r) => !r.completedAt && (window.__tagMap[r.guid] ?? []).includes("weekend")), unknown: [] };
        case "drive_children": return children(args.folderId).map((i) => ({ ...i }));
        case "drive_item": return driveById.get(args.drivewsid) ?? null;
        case "drive_thumbnails": return thumbs;
        case "drive_sync_folder": return children(args.folderId).length;
        case "drive_recents": return drive.lake.filter((i) => i.kind === "FILE").map((i, k) => ({ ...i, openedAt: now - (k + 1) * 3 * HOUR }));
        case "drive_trash": return [];
        case "account_info": return { name: "Jamie Rivera", email: "jamie.rivera@example.com", photoContact: "c13", webAccessAllowed: true, dataProtection: false };
        case "storage_info": return { usedBytes: 61_400_000_000, totalBytes: 200_000_000_000 };
        case "sync_status": case "sync_all": return report();
        case "photos_albums": return window.__albums.map(({ items, ...a }) => [a, items.length, items[0] ?? null]);
        default: return undefined;
      }
    },
  };
}
