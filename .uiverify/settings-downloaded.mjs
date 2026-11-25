// Settings' "Downloaded from iCloud" on This Computer (quince-record): today and the last seven
// days, each reason with its bytes, largest first, and the programs that read the folders by
// name. A day older than seven is not counted; a reason under a megabyte is left out; an empty
// week says so plainly.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openSettings, openPane } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const open = async (init) => {
  const ctx = await b.newContext({ viewport: { width: 1180, height: 820 } });
  if (init) await ctx.addInitScript(init);
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await openSettings(page);
  await page.waitForSelector(".settings");
  await openPane(page, "This Computer");
  await page.locator(".settings-row", { hasText: "Downloaded from iCloud" }).waitFor();
  return { ctx, page };
};
const group = (page) => page.locator(".settings-group", { hasText: "Downloaded from iCloud" });

{
  const { ctx, page } = await open();
  const text = await group(page).innerText();
  ok(/Today 1\.4 GB, last 7 days 1\.5 GB/.test(text), `today and the week, the old day left out  ${JSON.stringify(text.split("\n").slice(0, 3))}`);
  const rows = await group(page).locator(".settings-row").allInnerTexts();
  const labels = rows.slice(1).map((r) => r.split("\n")[0]);
  ok(JSON.stringify(labels) === JSON.stringify(["Programs reading your folders", "Newest photos kept ready", "Photos and files you opened", "Skykeep updates", "Photo grid", "Staying up to date"]), `reasons largest first, none under a megabyte  ${JSON.stringify(labels)}`);
  ok(!labels.includes("Backup"), "a day older than seven is not counted");
  ok(rows[1].includes("nautilus 700 MB, tracker-extract 200 MB"), `the programs reading the folders, by name  ${JSON.stringify(rows[1])}`);
  await ctx.close();
}
{
  const { ctx, page } = await open(() => { window.__record = []; });
  const text = await group(page).innerText();
  ok(text.includes("Nothing in the last 7 days."), "an empty week says so");
  ok((await group(page).locator(".settings-row").count()) === 1, "and lists nothing under it");
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
