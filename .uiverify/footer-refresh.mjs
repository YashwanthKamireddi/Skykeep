// An app window's account card speaks for that app: its own freshness, and the round button
// beside it updates that app alone — the ring round the picture turning while it works. Ctrl+R
// and F5 do the same and never reload the window. The window is titled for its app. In
// Skykeep's own window (Settings) the keys update the whole account.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openSettings, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1180, height: 820 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
// sync_some answers after a beat, so "working" can be seen; set_title is recorded.
await ctx.addInitScript(() => {
  const inner = window.__TAURI_INTERNALS__.invoke;
  window.__some = [];
  window.__titles = [];
  // As Tauri describes the window a page is in, so getCurrentWindow() resolves.
  window.__TAURI_INTERNALS__.metadata = { currentWindow: { label: "app-notes" }, currentWebview: { windowLabel: "app-notes", label: "app-notes" } };
  window.__TAURI_INTERNALS__.invoke = async (cmd, args) => {
    if (cmd === "plugin:window|set_title") { window.__titles.push(args.value); return null; }
    if (cmd === "sync_all") { window.__all = (window.__all ?? 0) + 1; return inner(cmd, args); }
    if (cmd === "sync_some") {
      window.__some.push(args.services);
      await new Promise((r) => setTimeout(r, window.__someDelay ?? 600));
      if (window.__someFail) throw window.__someFail;
      const now = Math.floor(Date.now() / 1000);
      return { services: args.services.map((s) => ({ service: s, label: s, outcome: "ok", detail: null, lastSyncedAt: now })), asOf: now, tookMs: 5 };
    }
    return inner(cmd, args);
  };
});
const page = await ctx.newPage();
let reloaded = 0;
page.on("framenavigated", (f) => { if (f === page.mainFrame()) reloaded++; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

await openScreen(page, "Reminders");
await page.waitForTimeout(150);
const card = page.locator(".account-card");
const update = card.locator(".account-update");
// The whole sentence, naming the app, is the card's tooltip; the sidebar line is its short form.
const said = async () => ((await card.locator(".sidebar-account").getAttribute("title")) ?? "").replace(/ · Skykeep$/, "");
ok(/^Reminders updated/.test(await said()) && /^Updated/.test(await card.locator(".account-status").innerText()), `the card speaks for Reminders  [${await said()}]`);
const fits = await card.locator(".account-status").evaluate((el) => el.scrollWidth <= el.clientWidth);
ok(fits, "and its words fit, not cut off");
ok((await page.evaluate(() => window.__titles.at(-1))) === "Reminders", "the window is titled Reminders");
// Opening an app's window is itself a request for its data: its own service, and only it.
ok(JSON.stringify(await page.evaluate(() => window.__some)) === '[["reminders"]]', "opening Reminders' window updates Reminders alone");
ok(await update.isVisible() && /^Update Now\. Reminders updated/.test((await update.getAttribute("aria-label")) ?? ""), "the round button updates Reminders, and says so");

reloaded = 0;
await update.click();
await page.waitForTimeout(100);
ok((await update.getAttribute("aria-label")) === "Updating Reminders…" && await update.isDisabled(), "pressing it says Reminders is updating");
const anim = await page.evaluate(() => getComputedStyle(document.querySelector(".account-card.is-working .account-ring"), "::after").animationName);
ok(anim === "sync-spin", `and the ring round the picture turns  [${anim}]`);
const some = await page.evaluate(() => window.__some);
ok(JSON.stringify(some) === `[["reminders"],["reminders"]]`, `only Reminders is asked for  ${JSON.stringify(some)}`);
await page.waitForTimeout(700);
ok((await said()) === "Reminders updated just now", `and settles  [${await said()}]`);
ok(((await page.locator("[role=status]").filter({ hasText: "Reminders" }).first().textContent()) ?? "") === "Reminders updated just now", "and how it ended is said aloud");

await openScreen(page, "Drive");
await page.waitForTimeout(150);
reloaded = 0;
ok((await page.evaluate(() => window.__titles.at(-1))) === "Drive", "Drive's window is titled Drive");
// Asked by its screen's name ("drivews"), the backend matched no service: Drive's window never
// updated on opening or coming back.
await page.waitForTimeout(700);
ok(JSON.stringify(await page.evaluate(() => window.__some)) === '[["drive"]]', `opening Drive's window updates Drive's own service  ${JSON.stringify(await page.evaluate(() => window.__some))}`);
await page.keyboard.press("Control+r");
await page.waitForTimeout(100);
ok(JSON.stringify(await page.evaluate(() => window.__some.at(-1))) === '["drive"]', "Ctrl+R updates Drive's own service (drive, not drivews)");
await page.waitForTimeout(700);
await page.keyboard.press("F5");
await page.waitForTimeout(100);
ok((await page.evaluate(() => window.__some.length)) === 3, `F5 does the same  ${JSON.stringify(await page.evaluate(() => window.__some))}`);
await page.waitForTimeout(700);
ok(reloaded === 0, "neither key reloads the window");

await page.evaluate(() => { window.__someFail = "Apple didn’t answer."; });
await update.click();
await page.waitForTimeout(800);
ok((await said()) === "Drive didn’t update: Apple didn’t answer." && (await card.getAttribute("class")).includes("is-trouble"), `a failure says why  [${await said()}]`);

await openSettings(page);
await page.waitForSelector(".settings");
reloaded = 0;
ok((await page.evaluate(() => window.__titles.at(-1))) === "Skykeep", "Skykeep's own window is titled Skykeep");
const all = await page.evaluate(() => window.__all ?? 0);
await page.keyboard.press("Control+r");
await page.waitForTimeout(200);
ok((await page.evaluate(() => window.__all ?? 0)) === all + 1 && (await page.evaluate(() => window.__some.length)) === 0, "there, Ctrl+R updates the whole account");
ok(reloaded === 0, "and never reloads the window");

await b.close();
console.log(failures ? `${failures} FAILED` : "all passed");
process.exit(failures ? 1 : 0);
