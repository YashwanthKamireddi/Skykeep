// Advanced Data Protection (issue #4): Notes, Reminders, Photos and iCloud Drive sync only for a
// while after the person approves Skykeep on one of their devices, and every approval puts a
// notice on those devices — so nothing but the person asks for one. A screen with nothing on
// this computer yet is replaced by one calm page saying so, with Sync Now; a screen that has
// its copy here keeps it, readable, with one card at the foot saying how it syncs and when it
// last did. Calendar and Contacts are never held up. Once approved, it all goes by itself;
// with web access off, the existing page shows. The window's own passes never ask for an
// approval; Sync Now does.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.BASE ?? process.env.URL ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const ALL = ["notes", "reminders", "photos", "drive"];
const HEADING = "Approve on your iPhone, iPad or Mac";
const EVERY = "Notes, Reminders, iCloud Drive and Photos";

async function open(access, afterAsk) {
  const ctx = await b.newContext({ viewport: { width: 1180, height: 820 } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
  await ctx.addInitScript(({ access, afterAsk }) => {
    window.__dataAccess = access;
    window.__dataAccessAfterAsk = afterAsk;
    // Every pass the window asks for, and whether the person asked for it.
    const inner = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = async (cmd, args) => {
      if (cmd === "sync_all" || cmd === "sync_some") (window.__passes ??= []).push(Boolean(args?.asked));
      return inner(cmd, args);
    };
  }, { access, afterAsk });
  const page = await ctx.newPage();
  await page.goto(BASE, { waitUntil: "networkidle" });
  await page.waitForTimeout(300);
  return { ctx, page };
}
const say = (page, access) => page.evaluate((a) => window.__emit?.("quince://data-access", a), access);
const heading = (page) => page.getByRole("heading", { name: HEADING });

// iCloud Drive has nothing on this computer yet: the page stands in for it.
{
  const { ctx, page } = await open({ state: "waiting", services: ALL, problem: null });
  ok((await page.evaluate(() => window.__passes ?? [])).every((asked) => !asked), "the window's own passes never ask for an approval");
  await openScreen(page, "Drive");
  await page.waitForTimeout(200);
  ok(await heading(page).isVisible(), "Drive, with nothing here yet, shows the approval page");
  const text = await page.locator(".device-approval").innerText();
  ok(text.includes(`Advanced Data Protection is on, so Skykeep syncs ${EVERY} when you approve it on one of your devices.`), "it says how syncing works under Advanced Data Protection");
  ok(text.includes("Waiting for your iPhone, iPad or Mac…"), "while it is on its way, it says it is waiting");
  ok(await page.locator(".device-approval .spinner").isVisible(), "with a spinner");
  ok(await page.getByRole("button", { name: "Sync Now" }).isVisible(), "and Sync Now");
  ok(!(await page.locator(".drive").first().isVisible()), "the empty Drive screen is not drawn beneath it");

  // Calendar and Contacts need no approval under Advanced Data Protection.
  await openScreen(page, "Calendar");
  await page.waitForTimeout(200);
  ok(!(await heading(page).isVisible()), "Calendar is never held up");
  ok(await page.locator(".cx").first().isVisible(), "and is drawn as usual");

  // Approved on the phone: the backend says so, and Drive opens by itself.
  await openScreen(page, "Drive");
  await say(page, { state: "open", services: [], problem: null });
  await page.waitForTimeout(250);
  ok(!(await heading(page).isVisible()), "once approved, the page goes away by itself");
  ok(await page.locator(".drive").first().isVisible(), "and Drive is drawn");
  await ctx.close();
}

// Notes already has its copy here: kept on screen, with one card at the foot.
{
  const { ctx, page } = await open({ state: "needed", services: ALL, problem: null }, { state: "waiting", services: ALL, problem: null });
  await openScreen(page, "Notes");
  await page.waitForTimeout(250);
  ok(!(await heading(page).isVisible()), "Notes with its copy here is not replaced");
  ok(await page.locator(".notes").first().isVisible(), "its notes stay readable");
  const card = page.locator(".device-approval-notice");
  ok(await card.isVisible(), "a card at the foot says how it syncs");
  const said = await card.innerText();
  ok(said.includes(`Advanced Data Protection is on, so Skykeep syncs ${EVERY} when you approve on your iPhone, iPad or Mac. Your copy here stays available in between.`), "in plain words");
  ok(/Updated (just now|\d+ min ago)/.test(said), "and when it last synced");
  ok((await page.evaluate(() => window.__dataAccessAsks ?? 0)) === 0, "nothing is asked until the person asks");
  await card.getByRole("button", { name: "Sync Now" }).click();
  await page.waitForTimeout(200);
  ok((await page.evaluate(() => window.__dataAccessAsks)) === 1, "Sync Now asks Apple once");
  ok((await card.innerText()).includes(`Approve on your iPhone, iPad or Mac to sync ${EVERY}.`), "and the card then says to approve");
  ok(!(await card.getByRole("button", { name: "Sync Now" }).isVisible()), "without a second button to press meanwhile");
  await ctx.close();
}

// Apple couldn't reach any of the person's devices; then, Sync Now on the page.
{
  const { ctx, page } = await open({ state: "notSent", services: ["photos"], problem: null }, { state: "waiting", services: ["photos"], problem: null });
  await openScreen(page, "Photos");
  await page.waitForTimeout(400);
  ok(await heading(page).isVisible(), "Photos shows the approval page");
  ok((await page.locator(".device-approval").innerText()).includes("Apple couldn’t reach your devices."), "and says Apple couldn't reach the devices");
  await page.getByRole("button", { name: "Sync Now" }).click();
  await page.waitForTimeout(200);
  ok((await page.locator(".device-approval").innerText()).includes("Waiting for your iPhone, iPad or Mac…"), "Sync Now puts it on its way");
  // A service not waiting on it opens as usual.
  await openScreen(page, "Notes");
  await page.waitForTimeout(200);
  ok(!(await page.locator(".device-approval-notice").isVisible()) && !(await heading(page).isVisible()), "Notes, not waiting on it, shows nothing extra");
  await ctx.close();
}

// The person's own refresh is the one pass that may ask for an approval.
{
  const { ctx, page } = await open({ state: "needed", services: ALL, problem: null });
  await page.evaluate(() => { window.__passes = []; });
  await page.getByRole("button", { name: "Sync Now" }).click();
  await page.waitForTimeout(300);
  ok(JSON.stringify(await page.evaluate(() => window.__passes)) === "[true]", "the sync button's pass is the person asking");
  await ctx.close();
}

// "Access iCloud Data on the Web" turned off: the page that says how to turn it on.
{
  const { ctx, page } = await open({ state: "open", services: [], problem: null });
  await openScreen(page, "Notes");
  await say(page, { state: "webAccessOff", services: [], problem: null });
  await page.waitForTimeout(250);
  ok(await page.getByRole("heading", { name: "Your iCloud data is kept off the web" }).isVisible(), "web access off shows the page with Apple's switch");
  ok(!(await heading(page).isVisible()), "not the approval page");
  await ctx.close();
}

// The page is reachable by keyboard and reads as a status for a screen reader.
{
  const { ctx, page } = await open({ state: "needed", services: ALL, problem: null });
  await openScreen(page, "Drive");
  await page.waitForTimeout(300);
  ok((await page.locator(".device-approval [role=status]").count()) === 1, "where it stands is announced politely");
  await page.getByRole("button", { name: "Sync Now" }).focus();
  ok(await page.evaluate(() => document.activeElement?.textContent === "Sync Now"), "Sync Now takes focus");
  await ctx.close();
}

await b.close();
if (failures) { console.log(`${failures} failed`); process.exit(1); }
console.log("all passed");
