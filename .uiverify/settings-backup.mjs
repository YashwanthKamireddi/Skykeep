// Settings › Back Up Photos: off until a folder is chosen; once on, it says how far it is and
// where, offers Change Folder and the switch to turn it off; the Hidden album is included only
// while unlocked, and a refusal is said.
import { engine } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const shots = process.env.SHOTS;
for (const scheme of ["light", "dark"]) {
  const ctx = await b.newContext({ viewport: { width: 1180, height: 820 }, colorScheme: scheme });
  await ctx.addInitScript(() => { window.__pick = "/media/yash/Backup Drive"; window.__hiddenLocked = true; });
  await ctx.addInitScript(() => { window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} }; });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(process.env.URL ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await page.locator(".sidebar-account").click();
  await page.waitForSelector(".settings");
  const row = page.getByRole("group", { name: "Backup" });
  await row.scrollIntoViewIfNeeded();
  await page.waitForTimeout(200);
  ok((await row.innerText()).includes("Every photo and video, as the original, in a folder you choose"), `${scheme}: off, it says what it does`);
  ok((await row.getByRole("switch").count()) === 0, `${scheme}: no switch until a folder is chosen`);
  if (shots) await row.screenshot({ path: `${shots}/backup-off-${scheme}.png` });
  await row.getByRole("button", { name: "Choose Folder" }).click();
  await page.waitForTimeout(150);
  const args = await page.evaluate(() => window.__backupArgs);
  ok(args?.folder === "/media/yash/Backup Drive" && args.hidden === false, `${scheme}: the chosen folder is set, Hidden left out`);
  const text = await row.innerText();
  ok(text.includes("Copying · 812 of 2,140"), `${scheme}: it says how far it is  [${text.split("\n")[1]}]`);
  ok(await row.getByRole("progressbar", { name: "Backup progress" }).isVisible(), `${scheme}: a bar shows while it copies`);
  ok(await row.getByRole("button", { name: "Change Folder" }).isVisible(), `${scheme}: Change Folder once on`);
  await row.getByRole("switch", { name: "Include the Hidden album" }).click();
  await page.waitForTimeout(150);
  ok((await row.getByRole("switch", { name: "Include the Hidden album" }).getAttribute("aria-checked")) === "false", `${scheme}: Hidden stays out while locked`);
  ok(await page.locator(".toast", { hasText: "Unlock the Hidden album" }).isVisible(), `${scheme}: and it says how to include it`);
  if (shots) await row.screenshot({ path: `${shots}/backup-on-${scheme}.png` });
  await row.getByRole("switch", { name: "Back Up Photos" }).click();
  await page.waitForTimeout(150);
  ok((await page.evaluate(() => window.__backupArgs.folder)) === null, `${scheme}: the switch turns it off`);
  await ctx.close();
}
await b.close();
console.log(failures ? `${failures} FAILED` : "all passed");
process.exit(failures ? 1 : 0);
