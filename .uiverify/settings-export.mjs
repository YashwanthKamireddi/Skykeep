// Settings › Backup and Export › Export a Copy: a folder is chosen, the bar moves service by
// service, it can be cancelled, it ends with Show in Folder, and a refusal is said. Photos are
// Back Up Photos' own, never part of a copy.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const open = async (init) => {
  const ctx = await b.newContext({ viewport: { width: 1180, height: 820 } });
  if (init) await ctx.addInitScript(init);
  // The event plugin's own half, which the shared stub leaves out (as crash.mjs does).
  await ctx.addInitScript(() => { window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} }; });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(process.env.URL ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await page.locator(".sidebar-account").click();
  await page.waitForSelector(".settings");
  await page.waitForTimeout(300);
  return { ctx, page, row: page.getByRole("group", { name: "Export a Copy" }) };
};

{
  const { ctx, page, row } = await open(() => { window.__pick = "/home/test/Backups"; window.__exportStepMs = 60; });
  ok(!(await row.getByRole("switch").count()), "no photos switch: photos are Back Up Photos' own");
  await row.getByRole("button", { name: "Export" }).click();
  await page.waitForTimeout(250);
  const bar = row.getByRole("progressbar", { name: "Export progress" });
  ok(await bar.isVisible(), "a bar shows while it runs");
  const mid = await row.innerText();
  ok(/(Notes|Reminders) · \d+ of 10/.test(mid), `each service is said with its count  [${mid.split("\n")[1]}]`);
  const now = Number(await bar.getAttribute("aria-valuenow"));
  ok(now > 0 && now < 100, `the bar is part way  [${now}%]`);
  ok(await row.getByRole("button", { name: "Cancel" }).isVisible(), "Cancel is offered while it runs");
  await page.waitForFunction(() => document.body.innerText.includes("Exported to"), null, { timeout: 5000 });
  const args = await page.evaluate(() => window.__exportArgs);
  ok(args.dest === "/home/test/Backups" && args.photos === false, "the chosen folder is sent, photos left out");
  ok((await row.innerText()).includes("Exported to Skykeep Export 2026-10-21 10.30"), "the row says where it went");
  ok(await page.locator(".toast", { hasText: "Exported to" }).getByRole("button", { name: "Show in Folder" }).isVisible(), "a toast says so, with Show in Folder");
  await row.getByRole("button", { name: "Show in Folder" }).click();
  await page.waitForTimeout(100);
  ok((await page.evaluate(() => window.__exportRevealed)) === 1, "Show in Folder asks the backend to show it");

  // An export under way, kept across leaving Settings and coming back.
  await page.evaluate(() => { window.__exportStepMs = 120; });
  await row.getByRole("button", { name: "Export" }).click();
  await page.waitForTimeout(200);
  await openScreen(page, "Reminders");
  await page.waitForTimeout(200);
  await page.locator(".sidebar-account").click();
  await page.waitForSelector(".settings");
  const back = page.getByRole("group", { name: "Export a Copy" });
  ok(await back.getByRole("progressbar").isVisible(), "an export under way is still shown on coming back");
  await page.waitForFunction(() => document.body.innerText.includes("Exported to"), null, { timeout: 8000 });
  await ctx.close();
}
{
  const { ctx, page, row } = await open(() => { window.__pick = "/home/test/Backups"; window.__exportStepMs = 200; });
  await row.getByRole("button", { name: "Export" }).click();
  await page.waitForTimeout(300);
  await row.getByRole("button", { name: "Cancel" }).click();
  await page.waitForTimeout(500);
  ok(await page.locator(".toast", { hasText: "Export cancelled. Nothing was left behind." }).isVisible(), "a cancel says nothing was left");
  ok(!(await page.locator(".toast.is-error").isVisible()), "and is not shown as a failure");
  ok(await row.getByRole("button", { name: "Export" }).isVisible() && !(await row.getByRole("progressbar").isVisible()), "the row is ready again");
  await ctx.close();
}
{
  const { ctx, page, row } = await open(() => {
    window.__pick = "/etc";
    window.__exportStepMs = 1;
    window.__exportFail = "That’s a system folder. Choose one in your home folder or on a drive.";
  });
  await row.getByRole("button", { name: "Export" }).click();
  await page.waitForTimeout(400);
  ok((await page.locator(".toast.is-error").innerText()).includes("system folder"), "a refused folder says why");
  await ctx.close();
}
{
  const { ctx, page, row } = await open(() => { window.__pick = null; });
  await row.getByRole("button", { name: "Export" }).click();
  await page.waitForTimeout(200);
  ok((await page.evaluate(() => window.__exportArgs)) === undefined, "closing the folder picker starts nothing");
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
