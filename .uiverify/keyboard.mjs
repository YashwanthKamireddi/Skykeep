import { openScreen } from "./stub.mjs";
// Keyboard-only operability. axe cannot judge focus flow, and this is what a keyboard or
// screen-reader user actually experiences.
import { engine as chromium } from "./engine.mjs";

const SERVICES = ["account","archivews","calendar","ckdatabasews","ckdeviceservice",
                  "cksharews","contacts","contactsweb","developerapi","docws","drivews",
                  "findme","geows","iwmb","iworkexportws","iworkthumbnailws","keyvalue",
                  "mail","mcc","mccgateway","mpp","notes","photos","photosupload",
                  "premiummailsettings","push","reminders","settings","sharedlibrary",
                  "sharedstreams","ubiquity","uploadimagews"];

function installStub({ screen, services, twoFactor }) {
  // Enough reminders to exercise the controls on a row. The keyboard driver is about
  // behaviour, so the data only has to be shaped right, not large.
  const lists = [
    { guid: "L1", title: "Daily", colour: "#c8912a", ctag: null, sortOrder: 0, enabled: true },
    { guid: "L2", title: "Study", colour: "#1badf8", ctag: null, sortOrder: 1, enabled: true },
  ];
  const items = [
    { guid: "R1", listGuid: "L1", title: "Water the plants", notes: null, dueAt: 4102444800,
      completedAt: null, priority: 0, flagged: false, parentGuid: null, recurFrequency: null,
      recurInterval: null, etag: "t1" },
    { guid: "R2", listGuid: "L1", title: "Evening run", notes: null, dueAt: 4102448400,
      completedAt: null, priority: 1, flagged: false, parentGuid: null, recurFrequency: null,
      recurInterval: null, etag: "t2" },
  ];
  window.__TAURI_INTERNALS__ = {
    transformCallback: (cb) => cb,
    invoke: async (cmd) => {
      switch (cmd) {
        case "auth_status":
          return { hasStoredSession: screen === "library", signedIn: screen === "library" };
        case "auth_resume": return screen === "library" ? "library" : "signin";
        case "auth_sign_in": return screen === "code" ? "needs_two_factor" : "complete";
        case "auth_two_factor_options": return twoFactor;
        case "account_services": return services;
        case "calendar_events": return [];
        case "calendar_list": return [];
        case "calendar_last_synced": return Math.floor(Date.now() / 1000);
        case "event_save": return "E-new";
        case "event_delete": return null;
        case "calendar_sync": return { calendars: 0, events: 0 };
        case "reminder_counts": return { today: 4, scheduled: 7, flagged: 2, all: 12, lists: [["L1", 5], ["L2", 3], ["lists", 4]] };
        case "reminder_lists": return lists;
        case "reminders_in": return items;
        case "reminders_due_between": return [];
        case "reminder_create": return null;
        case "reminder_delete": return null;
        case "reminder_set_done": return null;
        case "reminder_set_repeat": return null;
        case "reminder_edit": return null;
        case "reminder_list_create": case "reminder_list_rename": case "reminder_list_delete":
          return null;
        case "reminders_smart": return items;
        case "note_folders": return [];
        case "note_save": case "note_delete": case "note_move": return null;
        case "note_create": return "Note/new";
        case "notes_in": return [];
        case "contact_save": return "C-new";
        case "contact_delete": return null;
        case "contacts_list": return [];
        case "contacts_last_synced": return Math.floor(Date.now() / 1000);
        case "contacts_sync": return 0;
        case "drive_children": return [
          { drivewsid: "FOLDER::a", parentId: null, name: "Downloads", extension: null,
            kind: "FOLDER", size: null, childCount: 3, createdAt: 1785000000,
            modifiedAt: 1788000000, etag: null },
        ];
        case "drive_sync_folder": return 0;
        case "drive_download": return "/home/user/Downloads/file.pdf";
        case "drive_create_folder": case "drive_rename": case "drive_delete": case "drive_move": return null;
        case "drive_item": return null;
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
            ],
            asOf: Math.floor(Date.now() / 1000) - 300,
            tookMs: 40,
          };
        // The event plugin is how a finished refresh reaches the screens. Nothing emits in
        // a plain browser, so listening is a no-op that hands back an unsubscribe id.
        case "plugin:event|listen":
          return 0;
        case "plugin:event|unlisten":
          return null;
        default: return null;
      }
    },
  };
}
const TWO_FACTOR = { trustedDeviceCount: 2, phones: [], tooManyCodesSent: false };

const browser = await chromium.launch();
const checks = [];
const check = (name, ok, detail = "") => checks.push({ name, ok, detail });

async function open(screen) {
  const ctx = await browser.newContext({ viewport: { width: 1040, height: 800 } });
  await ctx.addInitScript(installStub, { screen, services: SERVICES, twoFactor: TWO_FACTOR });
  const page = await ctx.newPage();
  await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
  await page.waitForTimeout(300);
  return { ctx, page };
}

const focused = (page) =>
  page.evaluate(() => {
    const el = document.activeElement;
    if (!el) return "none";
    return el.getAttribute("aria-label") || el.textContent?.trim().slice(0, 24) || el.tagName;
  });

// --- sign-in: every control reachable, in a sensible order ---
{
  const { ctx, page } = await open("signin");
  const seen = [];
  for (let i = 0; i < 4; i++) {
    await page.keyboard.press("Tab");
    seen.push(await focused(page));
  }
  // Sign in is disabled — out of the Tab order — until both fields are filled, so it is
  // reached by Tab from the password once they are. (Matching the sheet's heading, which
  // once read "Sign in with…", let this pass without ever reaching the button.)
  await page.fill('input[aria-label="Apple Account"]', "you@icloud.com");
  await page.fill('input[aria-label="Password"]', "secret");
  await page.focus('input[aria-label="Password"]');
  await page.keyboard.press("Tab");
  const button = await page.evaluate(() => document.activeElement?.matches('button[type="submit"]') ? document.activeElement.textContent.trim() : "");
  check("sign-in reaches Apple Account, Password, Sign in by Tab",
    seen.includes("Apple Account") && seen.includes("Password") && button === "Sign in",
    `${seen.join(" → ")}; then ${button || "not the button"}`);

  // A visible focus ring is the whole point of keyboard focus.
  await page.focus('input[aria-label="Apple Account"]');
  const ring = await page.evaluate(() => {
    const g = document.querySelector(".field-group");
    return g ? getComputedStyle(g).boxShadow : "";
  });
  check("focused field shows a visible ring", ring.includes("rgb"), ring.slice(0, 48));
  await ctx.close();
}

// --- code: one tab stop, not six ---
{
  const { ctx, page } = await open("code");
  await page.fill('input[aria-label="Apple Account"]', "you@icloud.com");
  await page.fill('input[aria-label="Password"]', "secret");
  await page.click('button[type="submit"]');
  await page.waitForSelector(".code");

  check("code field takes focus automatically", (await focused(page)) === "Digit 1");

  const stops = await page.evaluate(
    () => document.querySelectorAll('.code input[tabindex="0"]').length
  );
  check("six boxes are ONE tab stop, not six", stops === 1, `${stops} tabbable`);

  // Typing advances; the whole code submits without touching a mouse.
  await page.keyboard.type("4812");
  check("typing advances between boxes", (await focused(page)) === "Digit 5");
  await ctx.close();
}

// --- calendar: the month is a grid, so one tab stop and arrows move within it ---
{
  const { ctx, page } = await open("library");
  await openScreen(page, "Calendar");
  await page.waitForSelector(".cal-grid");

  // Calendar opens on the month, which is the view the grid checks below belong to.
  const scope = await page.locator('.segment[aria-pressed="true"]').innerText();
  check("Calendar opens on the month", scope === "Month", scope);

  const stops = await page.evaluate(
    () => document.querySelectorAll('.cal-grid button[tabindex="0"]').length
  );
  check("month grid is ONE tab stop, not forty-two", stops === 1, `${stops} tabbable`);

  // The selected day is the grid's one tab stop, and its label names the day — what a
  // screen reader announces as the arrows move.
  const day = () => page.locator('.cal-grid button[tabindex="0"]').getAttribute("aria-label");
  const before = await day();
  await page.locator('.cal-grid button[tabindex="0"]').focus();
  await page.keyboard.press("ArrowDown");
  const down = await day();
  check("ArrowDown moves a week down the month", before !== down, `${before} → ${down}`);

  await page.keyboard.press("ArrowRight");
  const right = await day();
  check("ArrowRight moves a day along", down !== right, `${down} → ${right}`);
  await ctx.close();
}

// --- the pop-up button: one tab stop, arrows move, Escape leaves it alone ---
{
  const { ctx, page } = await open("library");
  await openScreen(page, "Reminders");
  await page.waitForSelector(".rx-rows li");

  // Open a reminder's details, where the pickers live.
  await page.locator(".rx-trailing .rx-icon-button[aria-expanded]").first().click({ force: true });
  await page.waitForSelector(".rx-detail .picker-button");

  const picker = page.locator(".rx-detail .picker-button").first();
  const before = await picker.innerText();
  await picker.click();
  await page.waitForSelector(".picker-menu");

  const options = await page.locator(".picker-menu li").count();
  check("the menu is ours, not the operating system's", options > 1, `${options} options`);

  const tabbable = await page.evaluate(
    () => document.querySelectorAll('.picker-menu li[tabindex="0"]').length
  );
  check("menu is ONE tab stop, not one per option", tabbable === 1, `${tabbable} tabbable`);

  await page.keyboard.press("Escape");
  const closed = (await page.locator(".picker-menu").count()) === 0;
  const after = await picker.innerText();
  check("Escape closes it without changing the value", closed && before === after, `${before} → ${after}`);

  // Arrows move the highlight, Enter takes it. The value on screen still comes from the
  // library, so this asserts the interaction rather than a stub echoing a choice back.
  await page.keyboard.press("ArrowDown");
  await page.waitForSelector(".picker-menu");
  const first = await page.locator(".picker-menu li.is-active").innerText();
  await page.keyboard.press("ArrowDown");
  const second = await page.locator(".picker-menu li.is-active").innerText();
  check("ArrowDown moves the highlight", first !== second, `${first.trim()} → ${second.trim()}`);

  // Two presses in one go, before focus can have moved into the menu they open: the second
  // is still the menu's. It used to reopen the menu and be lost.
  await page.keyboard.press("Escape");
  await picker.focus();
  const quick = await page.evaluate(() => {
    const button = document.querySelector(".rx-detail .picker-button");
    const press = () => button.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true }));
    press();
    press();
    return new Promise((r) => setTimeout(() => {
      const items = [...document.querySelectorAll(".picker-menu li")];
      r({ active: items.findIndex((li) => li.classList.contains("is-active")), chosen: items.findIndex((li) => li.getAttribute("aria-selected") === "true"), dump: items.map((li) => li.className + "/" + li.getAttribute("aria-selected")).join(" ") });
    }, 200));
  });
  check("a second key pressed at once still moves the highlight", quick.active === quick.chosen + 1, JSON.stringify(quick));
  // The pointer, once it moves, still takes the highlight to the row it is over.
  const third = page.locator(".picker-menu li").nth(2);
  const box = await third.boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2, { steps: 4 });
  check("a moving pointer highlights the row under it", (await third.getAttribute("class"))?.includes("is-active"), await third.innerText());
  await page.keyboard.press("Escape");
  await picker.focus();
  await page.keyboard.press("ArrowDown");
  await page.waitForSelector(".picker-menu");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowUp");

  await page.keyboard.press("Enter");
  await page.waitForTimeout(150);
  const dismissed = (await page.locator(".picker-menu").count()) === 0;
  const focusBack = await page.evaluate(
    () => document.activeElement?.classList.contains("picker-button") ?? false
  );
  check("Enter takes the choice and returns focus", dismissed && focusBack);
  await ctx.close();
}

// --- Drive's Move dialog: Escape closes it, focus starts inside it ---
{
  const { ctx, page } = await open("library");
  await openScreen(page, "Drive");
  await page.waitForSelector(".drive-row-hit");
  await page.hover(".drive-row.is-folder");
  await page.click('.drive-row.is-folder .drive-action[aria-label^="Actions for"]');
      await page.getByRole("menuitem", { name: "Move to Folder" }).click();
  await page.waitForSelector(".modal-sheet");

  // The focus move is deliberately a tick after mount (it has to win a race against the
  // browser's own "focus the button that was clicked" behavior), so the test waits for the
  // eventual state rather than asserting the instant after the dialog appears.
  const startsInside = await page
    .waitForFunction(() => document.activeElement?.closest(".modal-sheet") !== null, {
      timeout: 1000,
    })
    .then(() => true)
    .catch(() => false);
  check("Move dialog starts with focus inside it", startsInside);

  await page.keyboard.press("Escape");
  const closed = (await page.locator(".modal-sheet").count()) === 0;
  check("Escape closes the Move dialog", closed);
  await ctx.close();
}

await browser.close();
let failed = 0;
for (const c of checks) {
  if (!c.ok) failed++;
  console.log(`${c.ok ? "PASS" : "FAIL"} ${c.name}${c.detail ? `  [${c.detail}]` : ""}`);
}
console.log(failed === 0 ? "\nPASS keyboard-only operation works" : `\nFAIL ${failed} check(s)`);
process.exitCode = failed === 0 ? 0 : 1;
