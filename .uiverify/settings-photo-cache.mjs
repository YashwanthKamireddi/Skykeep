// Settings' photo cache, inline on This Computer: how much room photos opened here may keep,
// one choice of the backend's sizes, sent once as it is picked; the line under it says what it
// holds now, and a disk short of space says the cap is held lower for now. Settings' one Clear
// lets go of these too, and the line follows the backend's report.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openSettings } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const open = async (init) => {
  const ctx = await b.newContext({ viewport: { width: 1180, height: 820 } });
  if (init) await ctx.addInitScript(init);
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(process.env.URL ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await openSettings(page);
  await page.waitForSelector(".settings");
  await page.locator(".settings-row", { hasText: "Keep photos up to" }).waitFor();
  return { ctx, page };
};
const row = (page) => page.locator(".settings-row", { hasText: "Keep photos up to" });

{
  const { ctx, page } = await open();
  ok(/1\.8 GB of recently viewed photos/.test(await row(page).innerText()), `what the cache holds is said under the choice  ${JSON.stringify(await row(page).innerText())}`);
  ok(await page.locator(".dash-card", { hasText: "This Computer" }).getByText("Keep photos up to").first().isVisible(), "the choice sits on This Computer's card, no sheet to open");
  ok((await page.locator(".details-sheet, [role='dialog']").count()) === 0, "nothing opens over the page to reach it");
  const picker = row(page).getByRole("button", { name: /Keep photos up to/ }).or(row(page).getByRole("combobox")).first();
  ok(/2 GB/.test(await picker.innerText()), "it rests on the cap chosen, in words");
  await picker.click();
  await page.getByRole("option", { name: "512 MB" }).or(page.getByRole("menuitemradio", { name: "512 MB" })).first().click();
  await page.waitForTimeout(400);
  ok(JSON.stringify(await page.evaluate(() => window.__capSets)) === JSON.stringify([512 * 1024 ** 2]), "a size picked is sent, once");
  ok(/461 MB of recently viewed photos/.test(await row(page).innerText()), `the trim a lower cap starts reports back  ${JSON.stringify(await row(page).innerText())}`);
  const labels = await page.locator(".settings button").allInnerTexts();
  ok(!labels.some((l) => /…|\.\.\./.test(l.replace("Clearing…", ""))), "no button label trails off");
  await ctx.close();
}
{
  // A disk short of space: said plainly, and true.
  const { ctx, page } = await open(() => {
    window.__cache = { used: 900_000_000, cap: 5 * 1024 ** 3, limit: 1_200_000_000, thumbnails: 1, steps: [512 * 1024 ** 2, 1024 ** 3, 2 * 1024 ** 3, 5 * 1024 ** 3] };
  });
  ok((await row(page).innerText()).includes("Using up to 1.1 GB for now, to leave room on this disk."), "a disk short of space says the cap is held lower for now");
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
