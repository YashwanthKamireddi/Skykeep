// Settings › Backup and Export › Back Up Photos: off until a folder is chosen; once on, it says
// how far it is and where, with Change Folder and the switch to turn it off. "Include the Hidden
// album" shows always, and switching it on while locked asks for the unlock right there.
import { engine } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
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
  await page.locator(".sidebar-account").click();
  await page.waitForSelector(".settings");
  const section = page.locator(".settings-group", { has: page.getByRole("group", { name: "Export a Copy" }) });
  const row = page.getByRole("group", { name: "Backup" });
  await row.scrollIntoViewIfNeeded();
  await page.waitForTimeout(200);
  ok(await section.getByRole("group", { name: "Backup" }).isVisible(), `${scheme}: Backup and Export share their own section`);
  ok((await row.innerText()).includes("kept up to date in a folder you choose"), `${scheme}: off, it says what it does`);
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
  ok(args?.folder === "/media/yash/Backup Drive" && args.hidden === true, `${scheme}: the chosen folder is set, with Hidden`);
  const text = await row.innerText();
  ok(text.includes("Copying · 812 of 2,140"), `${scheme}: it says how far it is  [${text.split("\n")[1]}]`);
  ok(await row.getByRole("progressbar", { name: "Backup progress" }).isVisible(), `${scheme}: a bar shows while it copies`);
  if (shots) await section.screenshot({ path: `${shots}/backup-on-${scheme}.png` });
  await row.getByRole("switch", { name: "Back Up Photos" }).click();
  await page.waitForTimeout(150);
  ok((await page.evaluate(() => window.__backupArgs.folder)) === null, `${scheme}: the switch turns it off`);
  await ctx.close();
}
await b.close();
console.log(failures ? `${failures} FAILED` : "all passed");
process.exit(failures ? 1 : 0);
