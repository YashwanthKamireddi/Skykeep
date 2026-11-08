// Settings › Backup and Export › Back Up iCloud (photos, and iCloud Drive unless left out): off until a folder is chosen; once on, it says
// how far it is and where, with Change Folder and the switch to turn it off. "Include the Hidden
// album" shows always, and switching it on while locked asks for the unlock right there.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openSettings } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const shots = process.env.SHOTS;
for (const scheme of ["light", "dark"]) {
  const ctx = await b.newContext({ viewport: { width: 1180, height: 820 }, colorScheme: scheme });
  await ctx.addInitScript(() => { window.__pick = "/media/yash/Backup Drive"; });
  await ctx.addInitScript(() => { window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} }; });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(process.env.URL ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await openSettings(page);
  await page.waitForSelector(".settings");
  const section = page.locator(".dash-card", { has: page.getByRole("heading", { name: "Backup", exact: true }) });
  const row = page.getByRole("group", { name: "Backup", exact: true });
  await row.scrollIntoViewIfNeeded();
  await page.waitForTimeout(200);
  ok(await section.getByRole("group", { name: "Backup", exact: true }).isVisible() && await section.getByRole("group", { name: "Export a Copy" }).getByRole("button", { name: "Export" }).isVisible(), `${scheme}: Backup has its own card, Export a Copy in it`);
  ok((await row.innerText()).includes("Originals, kept up to date in a folder or drive you choose"), `${scheme}: off, it says what it does`);
  const drive = row.getByRole("switch", { name: "Include iCloud Drive" });
  ok((await drive.getAttribute("aria-checked")) === "true", `${scheme}: iCloud Drive is in by default`);
  const hidden = row.getByRole("switch", { name: "Include the Hidden album" });
  ok(await hidden.isVisible(), `${scheme}: the Hidden option shows before a folder is chosen`);
  if (shots) await section.screenshot({ path: `${shots}/backup-off-${scheme}.png` });

  // Hidden while locked: the unlock sheet, then included.
  await hidden.click();
  const sheet = page.locator(".px-unlock");
  ok(await sheet.isVisible(), `${scheme}: switching Hidden on while locked asks to unlock it`);
  await sheet.locator("input").fill("right");
  await sheet.locator("input").press("Enter");
  await page.waitForTimeout(250);
  ok((await hidden.getAttribute("aria-checked")) === "true", `${scheme}: unlocked, Hidden is included`);

  await row.getByRole("button", { name: "Choose Folder" }).click();
  await page.waitForTimeout(150);
  const args = await page.evaluate(() => window.__backupArgs);
  ok(args?.folder === "/media/yash/Backup Drive" && args.hidden === true && args.drive === true, `${scheme}: the chosen folder is set, with Hidden and iCloud Drive`);
  const text = await row.innerText();
  ok(text.includes("812 of 2,140"), `${scheme}: Photos says how far it is`);
  ok(text.includes("1,203 of 4,120 files"), `${scheme}: iCloud Drive says how far it is`);
  ok(await row.getByRole("progressbar", { name: "Photos backup progress" }).isVisible(), `${scheme}: a bar for Photos while it copies`);
  ok(await row.getByRole("progressbar", { name: "iCloud Drive backup progress" }).isVisible(), `${scheme}: and one for iCloud Drive`);
  ok(text.includes("In Backup Drive"), `${scheme}: it says where`);
  await row.getByRole("button", { name: "Show in Folder" }).click();
  await page.waitForTimeout(100);
  ok((await page.evaluate(() => window.__backupRevealed)) === 1, `${scheme}: Show in Folder opens it`);
  if (shots) await section.screenshot({ path: `${shots}/backup-on-${scheme}.png` });
  await drive.click();
  await page.waitForTimeout(150);
  ok((await page.evaluate(() => window.__backupArgs.drive)) === false && (await drive.getAttribute("aria-checked")) === "false", `${scheme}: iCloud Drive can be left out`);
  await row.getByRole("switch", { name: "Back Up iCloud" }).click();
  await page.waitForTimeout(150);
  ok((await page.evaluate(() => window.__backupArgs.folder)) === null, `${scheme}: the switch turns it off`);
  await ctx.close();
}
await b.close();
console.log(failures ? `${failures} FAILED` : "all passed");
process.exit(failures ? 1 : 0);
