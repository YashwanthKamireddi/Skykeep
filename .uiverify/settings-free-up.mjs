// Settings › Home › Free up iCloud space (components/FreeUp): the line under the storage bar only
// with a backup and something that can leave; the sheet's bar as the preview (striped, now and
// after), the age choice, every photo by month and untickable, what stays and why, one button
// that says what it does; the run, Stop, and the result with what was kept and why. Light and
// dark, roomy and 800×600, where what stays folds to one line and the sheet fits the window.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openSettings } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const shots = process.env.SHOTS;
const GB = 1_000_000_000;
const storage = {
  usedBytes: 47.6 * GB, totalBytes: 50 * GB, full: false, almostFull: true,
  kinds: [
    { key: "photos", label: "Photos", color: "#FFC800", bytes: 38 * GB },
    { key: "backup", label: "Backups", color: "#7B61FF", bytes: 6 * GB },
  ],
};
const DAY = 86400;
const now = Math.floor(Date.UTC(2026, 10, 27) / 1000);
const planFor = (months) => {
  const n = months === 24 ? 6 : 29;
  const items = Array.from({ length: n }, (_, i) => ({ id: `p${i}`, takenAt: now - (400 + i * 9) * DAY, bytes: 0.5 * GB, video: i % 5 === 0 }));
  return { items, stays: { favourites: 12, hidden: 3, sharedLibrary: 0, notBackedUp: 2, undated: 0 } };
};
const idle = { running: false, checked: 0, total: 0, freed: 0, freedBytes: 0, edited: 0, notSame: 0, gone: 0 };

async function open(scheme, width, height, { backup = true, plan = true } = {}) {
  const ctx = await b.newContext({ viewport: { width, height }, colorScheme: scheme });
  await ctx.addInitScript(() => { window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} }; });
  await ctx.addInitScript(({ storage, backup, plan, idle, src }) => {
    window.__storage = storage;
    if (backup) window.__backup = { folder: "/media/yash/Backup Drive", hidden: false, drive: true, driveRunning: false, driveNote: null, albumsNote: null, shared: false, sharedCopied: 0, sharedTotal: 0, freeUp: idle, copied: 2140, total: 2140, running: false, problem: null, photosBytes: 0, photosBytesTotal: 0, driveFiles: 0, driveFilesTotal: 0, driveBytes: 0, driveBytesTotal: 0 };
    window.__freeUpPlan = plan ? new Function("months", src) : undefined;
  }, { storage, backup, plan, idle, src: `const planFor = ${planFor.toString()}; const now = ${now}, DAY = ${DAY}, GB = ${GB}; return planFor(months);` });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await openSettings(page);
  await page.waitForSelector(".prefs-storage");
  await page.waitForTimeout(300);
  return { ctx, page };
}

// No backup, or nothing that can leave: no line at all.
for (const opts of [{ backup: false }, { plan: false }]) {
  const { ctx, page } = await open("light", 1180, 820, opts);
  ok((await page.locator(".free-up-entry").count()) === 0, `no way in ${opts.backup === false ? "without a backup" : "with nothing to free"}`);
  await ctx.close();
}

for (const [scheme, width, height] of [["light", 1180, 820], ["dark", 1180, 820], ["light", 800, 600]]) {
  const at = `${scheme} ${width}px`;
  const { ctx, page } = await open(scheme, width, height);
  const entry = page.locator(".free-up-entry");
  ok((await entry.innerText()).includes("14.5 GB of photos and videos are in your backup and can leave iCloud"), `${at}: the line says how much can leave`);
  await entry.getByRole("button", { name: "Free Up Space" }).click();
  const sheet = page.getByRole("dialog", { name: "Free up iCloud space" });
  await sheet.waitFor();
  await page.waitForTimeout(200);
  const text = await sheet.innerText();
  ok(text.includes("They leave iCloud and every device signed in to it") && text.includes("Your backup on Backup Drive keeps every one"), `${at}: it says plainly where they go and where they're kept`);
  ok(/Recently Deleted keeps them until \S/.test(text), `${at}: and until when they can come back`);
  ok((await sheet.locator(".storage-part.is-leaving").count()) === 1, `${at}: the leaving part is drawn on the bar`);
  ok(text.includes("47.6 GB used now, 33.1 GB after") && text.includes("14.5 GB freed of 50 GB"), `${at}: now, after and freed`);
  ok((await sheet.locator(".segment[aria-pressed=true]").innerText()) === "1 year", `${at}: a year by default`);
  const tiles = sheet.locator(".free-up-tile");
  await page.waitForTimeout(300);
  ok((await tiles.count()) > 0, `${at}: the photos show (${await tiles.count()} drawn)`);
  const remove = sheet.getByRole("button", { name: /^Remove \d+ from iCloud$/ });
  ok((await remove.innerText()) === "Remove 29 from iCloud", `${at}: one button says what it does`);
  await tiles.first().click();
  ok((await remove.innerText()) === "Remove 28 from iCloud" && (await tiles.first().getAttribute("aria-checked")) === "false", `${at}: a photo unticked stays`);
  ok((await sheet.innerText()).includes("14 GB freed of 50 GB"), `${at}: and the bar follows`);
  if (width > 820) {
    const stays = await sheet.locator(".free-up-stays").innerText();
    ok(stays.includes("Favourites") && stays.includes("12") && stays.includes("Not in the backup yet") && !stays.includes("Shared Library"), `${at}: what stays and why, none that don't apply`);
  } else {
    ok(!(await sheet.locator(".free-up-stays ul").isVisible()) && (await sheet.locator(".free-up-stays-sum").innerText()) === "17", `${at}: what stays folds to one line`);
    const box = await sheet.boundingBox();
    ok(box && box.y >= 0 && box.y + box.height <= height && box.x >= 0 && box.x + box.width <= width, `${at}: the sheet fits the window`);
  }
  if (shots) await page.screenshot({ path: `${shots}/free-up-${scheme}-${width}.png` });

  // Two years: fewer, and the choice made fresh.
  await sheet.getByRole("button", { name: "2 years" }).click();
  await page.waitForTimeout(250);
  ok((await remove.innerText()) === "Remove 6 from iCloud", `${at}: another age, another plan`);

  // The run: started with exactly the ticked photos, its progress, Stop, then the result.
  await tiles.nth(1).click();
  await remove.click();
  const started = await page.evaluate(() => window.__freeUpStarted);
  ok(Array.isArray(started) && started.length === 5 && !started.includes("p1"), `${at}: only the ticked photos are sent`);
  await page.evaluate(() => window.__emit("quince://backup", { ...window.__backup, freeUp: { running: true, checked: 2, total: 5, freed: 2, freedBytes: 1e9, edited: 0, notSame: 0, gone: 0 } }));
  await page.waitForTimeout(100);
  ok((await sheet.innerText()).includes("Checking against Apple's copies: 2 of 5"), `${at}: how far, while it runs`);
  await sheet.getByRole("button", { name: "Stop" }).click();
  ok(await page.evaluate(() => window.__freeUpStopped === true), `${at}: Stop stops it`);
  await page.evaluate(() => window.__emit("quince://backup", { ...window.__backup, freeUp: { running: false, checked: 5, total: 5, freed: 3, freedBytes: 1.5e9, edited: 1, notSame: 1, gone: 0 } }));
  await page.waitForTimeout(100);
  const result = await sheet.innerText();
  ok(result.includes("1.5 GB freed. 3 photos and videos went to Recently Deleted."), `${at}: what was done`);
  ok(result.includes("47.6 GB used now, 46.1 GB after") && result.includes("1.5 GB freed of 50 GB"), `${at}: the bar shows what was freed, not what was ticked`);
  ok(result.includes("Kept in iCloud, edited on a device: 1") && result.includes("Kept in iCloud, not the same as the backup: 1"), `${at}: what was kept, and why`);
  if (shots) await page.screenshot({ path: `${shots}/free-up-done-${scheme}-${width}.png` });
  await sheet.getByRole("button", { name: "Done" }).click();
  await page.waitForTimeout(150);
  ok((await page.getByRole("dialog", { name: "Free up iCloud space" }).count()) === 0, `${at}: Done closes it`);
  await ctx.close();
}

// A problem before anything was checked (drive gone, iCloud preparing): said, and the sheet stays.
{
  const { ctx, page } = await open("light", 1180, 820);
  await page.locator(".free-up-entry button").click();
  const sheet = page.getByRole("dialog", { name: "Free up iCloud space" });
  await sheet.waitFor();
  await sheet.getByRole("button", { name: /^Remove/ }).click();
  await page.evaluate(() => window.__emit("quince://free-up-failed", "The backup folder isn't there. Connect its drive, then try again."));
  await page.waitForTimeout(100);
  ok((await sheet.getByRole("alert").innerText()).includes("Connect its drive"), "a problem is said in the sheet");
  ok(await sheet.getByRole("button", { name: /^Remove/ }).isVisible(), "and it can be tried again");
  await ctx.close();
}

await b.close();
if (failures) { console.log(`${failures} failed`); process.exit(1); }
console.log("all passed");
