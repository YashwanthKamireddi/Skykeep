// The footer speaks for the app in front: its own freshness, and a small control that updates
// that app alone — spinning while it works. Ctrl+R and F5 do the same and never reload the
// window. The window's title follows the screen. In Settings the footer is the whole account.
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
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await page.waitForTimeout(500);
reloaded = 0;
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

await openScreen(page, "Reminders");
await page.waitForTimeout(150);
// The freshness line is itself the control (no separate glyph): its tooltip says the whole
// sentence, and "Click to update now" while it is not already updating.
const status = page.locator(".sidebar-sync");
const said = async () => ((await status.getAttribute("title")) ?? "").replace(/\. Click to update now$/, "");
ok(/^Reminders updated/.test(await said()) && /^Updated/.test(await status.locator(".sidebar-sync-text").innerText()), `the footer speaks for Reminders  [${await said()}]`);
const fits = await status.locator(".sidebar-sync-text").evaluate((el) => el.scrollWidth <= el.clientWidth);
ok(fits, "and its words fit, not cut off");
ok((await page.evaluate(() => window.__titles.at(-1))) === "Reminders", "the window is titled Reminders");

const btn = status;
ok(await btn.isVisible() && /Update now$/.test((await btn.getAttribute("aria-label")) ?? ""), "the line itself is the control to update Reminders");
await btn.click();
await page.waitForTimeout(100);
ok((await said()) === "Updating Reminders…", "pressing it says Reminders is updating");
const anim = await page.evaluate(() => getComputedStyle(document.querySelector(".sidebar-sync.is-working .sync-dot")).animationName);
ok(anim === "sync-spin", `and its dot turns in place  [${anim}]`);
ok(JSON.stringify(await page.evaluate(() => window.__some)) === '[["reminders"]]', "only Reminders is asked for");
await page.waitForTimeout(700);
ok((await said()) === "Reminders updated just now", `and settles  [${await said()}]`);

await openScreen(page, "Drive");
await page.waitForTimeout(150);
ok((await page.evaluate(() => window.__titles.at(-1))) === "Drive", "the title follows to iCloud Drive");
await page.keyboard.press("Control+r");
await page.waitForTimeout(100);
ok(JSON.stringify(await page.evaluate(() => window.__some.at(-1))) === '["drive"]', "Ctrl+R updates Drive's own service (drive, not drivews)");
await page.waitForTimeout(700);
await page.keyboard.press("F5");
await page.waitForTimeout(100);
ok((await page.evaluate(() => window.__some.length)) === 3, "F5 does the same");
await page.waitForTimeout(700);
ok(reloaded === 0, "neither key reloads the window");

await page.evaluate(() => { window.__someFail = "Apple didn’t answer."; });
await status.click();
await page.waitForTimeout(800);
ok((await said()) === "Drive didn’t update: Apple didn’t answer." && (await status.getAttribute("class")).includes("is-trouble"), `a failure says why  [${await said()}]`);

await openSettings(page);
await page.waitForSelector(".settings");
ok((await page.evaluate(() => window.__titles.at(-1))) === "Settings", "in Settings the window is titled Settings");
ok(/^(Updated|Syncing|Some)/.test(await status.locator(".sidebar-sync-text").innerText()) && !/iCloud Drive|Reminders/.test(await said()), `and the footer is the whole account  [${await said()}]`);

await b.close();
console.log(failures ? `${failures} FAILED` : "all passed");
process.exit(failures ? 1 : 0);
