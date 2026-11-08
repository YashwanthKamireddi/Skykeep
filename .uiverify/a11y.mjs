// Accessibility audit with axe-core — the same engine Lighthouse and Deque use — at WCAG 2.2 AA.
// Every screen and every sheet, in light, dark, Increase Contrast and Windows' High Contrast
// (forced colours),
// with the sample library the other suites draw from. Passes with no serious or critical
// violation; moderate and minor ones are listed, not failed.
//
//   node .uiverify/a11y.mjs                 Chromium (WebView2's engine)
//   ENGINE=webkit node .uiverify/a11y.mjs   WebKit (WebKitGTK's engine) — no forced colours:
//                                           forced colours is a Windows mode, and Playwright
//                                           emulates it in Chromium alone
//   BASE=http://localhost:4232 ONLY=drive   another server, or only the scenarios matching
import { engine as browserEngine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen, openSettings } from "./stub.mjs";
import AxeBuilder from "@axe-core/playwright";

const BASE = process.env.BASE ?? "http://localhost:4173";
const ONLY = process.env.ONLY ? new RegExp(process.env.ONLY, "i") : null;
const TWO_FACTOR = { trustedDeviceCount: 0, phones: [{ id: 1, maskedNumber: "+1 ••• ••• ••42" }], tooManyCodesSent: false };
const FIXTURES = { calendarFixture: true, notesFixture: true, contactsFixture: true, driveFixture: true, remindersFixture: true };

const settle = (p, ms = 300) => p.waitForTimeout(ms);
const open = (name) => async (p) => { await openScreen(p, name); await settle(p, 500); };
const then = (...steps) => async (p) => { for (const s of steps) await s(p); };

/** Each: what it is, where it starts, how to get there, and what must be on screen when it is. */
const scenarios = [
  { name: "sign-in", screen: "signin", ready: ".signin, form" },
  { name: "code", screen: "code", ready: ".code", act: async (p) => {
      await p.fill('input[aria-label="Apple Account"]', "you@icloud.com");
      await p.fill('input[aria-label="Password"]', "secret");
      await p.click('button[type="submit"]');
    } },
  { name: "calendar month", act: open("Calendar"), ready: ".cx" },
  { name: "calendar event", ready: ".cx-popover", act: then(open("Calendar"), async (p) => {
      await p.keyboard.press("Control+2");
      await settle(p, 400);
      await p.locator(".cx-block", { hasText: "Dentist" }).first().click();
    }) },
  { name: "notes", act: open("Notes"), ready: ".notes" },
  { name: "notes format", ready: ".format-panel", act: then(open("Notes"), async (p) => {
      await p.locator(".note-row").first().click();
      await settle(p);
      await p.getByRole("button", { name: "Format", exact: true }).click();
    }) },
  { name: "reminders", act: open("Reminders"), ready: ".rx" },
  { name: "reminders list info", ready: ".rx-info", act: then(open("Reminders"), async (p) => {
      await p.locator(".rx-list", { hasText: "Daily" }).hover();
      await p.getByRole("button", { name: "Actions for Daily" }).click();
      await p.getByRole("menuitem", { name: "Show List Info" }).click();
    }) },
  { name: "contacts", act: open("Contacts"), ready: ".contacts" },
  { name: "contacts menu", ready: '[role="menu"]', act: then(open("Contacts"), async (p) => {
      await p.locator(".contact-row").first().click({ button: "right" });
    }) },
  { name: "drive", act: open("Drive"), ready: ".drive-row" },
  { name: "drive info", ready: ".info-sheet", act: then(open("Drive"), async (p) => {
      await p.locator(".drive-row", { hasText: "Semester Plan" }).hover();
      await p.getByRole("button", { name: "Actions for Semester Plan.pdf" }).click();
      await p.getByRole("menuitem", { name: "Get Info" }).click();
    }) },
  { name: "drive move", ready: ".modal-sheet", act: then(open("Drive"), async (p) => {
      await p.locator(".drive-row", { hasText: "Semester Plan" }).hover();
      await p.getByRole("button", { name: "Actions for Semester Plan.pdf" }).click();
      await p.getByRole("menuitem", { name: "Move to Folder" }).click();
    }) },
  { name: "photos", act: open("Photos"), ready: ".px-tile" },
  { name: "photos viewer", ready: ".px-viewer", act: then(open("Photos"), async (p) => {
      await p.locator(".px-tile").nth(1).dblclick();
    }) },
  { name: "photos download", ready: '[role="dialog"]', act: then(open("Photos"), async (p) => {
      await p.locator(".px-tile").nth(1).click({ button: "right" });
      await p.locator(".row-menu").getByText("More Download Options").click();
    }) },
  { name: "photos jump", ready: ".px-jump", act: then(open("Photos"), async (p) => {
      await p.locator('[aria-label="Go to a Month"]').click();
    }) },
  { name: "settings", ready: ".settings, [data-screen='settings']", act: async (p) => {
      await openSettings(p);
      await settle(p, 500);
    } },
  { name: "sign-out confirm", ready: '[role="alertdialog"]', act: async (p) => {
      await openSettings(p);
      await settle(p, 500);
      await p.getByRole("button", { name: "Sign Out" }).click();
    } },
  // The other calendar views, each a grid of its own.
  { name: "calendar week", ready: ".cx", act: then(open("Calendar"), async (p) => { await p.keyboard.press("Control+2"); await settle(p, 400); }) },
  { name: "calendar day", ready: ".cx", act: then(open("Calendar"), async (p) => { await p.keyboard.press("Control+1"); await settle(p, 400); }) },
  { name: "calendar year", ready: ".cal-year", act: then(open("Calendar"), async (p) => { await p.keyboard.press("Control+4"); await settle(p, 400); }) },
  // A list with the keys in it draws its selection in the accent: text on it must still read.
  { name: "notes list focused", ready: ".note-rows:focus-within", act: then(open("Notes"), async (p) => {
      await p.locator(".note-row").nth(1).click();
      await p.locator(".note-row").nth(1).focus();
    }) },
  { name: "contacts list focused", ready: ".contact-rows.is-focused", act: then(open("Contacts"), async (p) => {
      await p.locator(".contact-row").nth(1).click();
      await p.locator(".contact-row").nth(1).focus();
    }) },
  { name: "drive selected", ready: ".drive-row.is-selected", act: then(open("Drive"), async (p) => {
      await p.locator(".drive-row", { hasText: "Semester Plan" }).click();
    }) },
  { name: "photos selected", ready: '.px-tile[aria-selected="true"]', act: then(open("Photos"), async (p) => {
      await p.locator(".px-tile").first().click();
    }) },
  // A new version offered (components/UpdateNotice), with its notes open.
  { name: "update notice", ready: ".update-notice .update-notes[open]",
    init: () => { window.__update = { version: "0.2.0", notes: "Faster Photos.\nCalendar fixes." }; },
    act: async (p) => {
      await p.locator(".update-notice summary").click();
      await settle(p, 300);
    } },
  // Each app's own window: its account card at the foot of its sidebar, and the window narrow,
  // where an app folds its own sidebar into a picker.
  { name: "app foot", ready: ".app-foot .sidebar-account", act: open("Notes") },
  { name: "narrow notes", ready: ".notes-folder-picker", viewport: { width: 800, height: 700 }, act: open("Notes") },
];

// "more" is Increase Contrast (prefers-contrast: more), where a11y.css firms the design up.
const modes = engineName === "chromium" || engineName === "msedge"
  ? ["light", "dark", "more-light", "more-dark", "forced-light", "forced-dark"]
  : ["light", "dark", "more-light", "more-dark"];

const browser = await browserEngine.launch();
let serious = 0;
let minor = 0;
let broken = 0;

for (const mode of modes) {
  for (const s of scenarios) {
    if (ONLY && !ONLY.test(s.name)) continue;
    const forced = mode.startsWith("forced");
    const ctx = await browser.newContext({
      viewport: s.viewport ?? { width: 1280, height: 800 },
      colorScheme: mode.endsWith("dark") ? "dark" : "light",
      ...(forced ? { forcedColors: "active" } : {}),
      ...(mode.startsWith("more") ? { contrast: "more" } : {}),
    });
    await ctx.addInitScript(installStub, { screen: s.screen ?? "library", services: SERVICES, twoFactor: TWO_FACTOR, ...FIXTURES });
    if (s.init) await ctx.addInitScript(s.init);
    const page = await ctx.newPage();
    const label = `${s.name} (${mode})`.padEnd(32);
    try {
      await page.goto(BASE, { waitUntil: "networkidle" });
      if ((s.screen ?? "library") === "library") await page.waitForSelector("main.content[data-screen]", { state: "attached" });
      if (s.act) await s.act(page);
      await page.locator(s.ready).first().waitFor({ state: "attached", timeout: 5000 });
      await settle(page, 400);
    } catch (e) {
      broken++;
      console.log(`${label} FAIL could not reach it: ${String(e.message).split("\n")[0]}`);
      await ctx.close();
      continue;
    }

    const { violations } = await new AxeBuilder({ page })
      .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
      // Under forced colours the system draws every colour, so the author's contrast is moot.
      .disableRules(forced ? ["color-contrast"] : [])
      .analyze();

    const bad = violations.filter((v) => v.impact === "serious" || v.impact === "critical");
    serious += bad.length;
    minor += violations.length - bad.length;
    console.log(violations.length === 0 ? `${label} clean`
      : `${label} ${bad.length} serious/critical, ${violations.length - bad.length} other`);
    for (const v of violations) {
      console.log(`   [${v.impact}] ${v.id} — ${v.help}`);
      for (const node of v.nodes.slice(0, 4)) {
        console.log(`      ${node.target.join(" ")}  ${node.html.slice(0, 100)}`);
        if (v.id === "color-contrast") console.log(`        ${node.any[0]?.message?.slice(0, 140)}`);
      }
    }
    await ctx.close();
  }
}

await browser.close();
console.log(serious + broken === 0
  ? `\nPASS no serious or critical WCAG 2.2 AA violations (${engineName}; ${minor} lesser)`
  : `\nFAIL ${serious} serious/critical violation(s), ${broken} scenario(s) unreachable (${engineName})`);
process.exitCode = serious + broken === 0 ? 0 : 1;
