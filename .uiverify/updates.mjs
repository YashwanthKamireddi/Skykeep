// Skykeep's own updates (src-tauri/src/updates.rs, components/UpdateNotice): a new version is
// offered once, calmly, with its notes; Update and Restart shows the download's progress, says
// what a restart waits for (changes not yet in iCloud, someone typing) and goes ahead once
// clear; Later puts it away; a download that fails its signature says so and offers no retry.
// Settings has Check for Updates Automatically and Check for Updates, and a copy installed by a
// package manager says who updates it instead. Played against the stub (.uiverify/stub.mjs).
//
//   BASE=http://localhost:4173 node .uiverify/updates.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openSettings, openScreen } from "./stub.mjs";
const BASE = process.env.BASE ?? process.env.URL ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };

async function open(update, extra = {}) {
  const ctx = await b.newContext({ viewport: { width: 1180, height: 820 }, colorScheme: extra.scheme ?? "light" });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
  await ctx.addInitScript((u) => { window.__update = u; }, update);
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(BASE, { waitUntil: "networkidle" });
  await page.waitForTimeout(300);
  return { ctx, page };
}
const notice = (page) => page.locator(".update-notice");

// ── An update is offered ──
{
  const { ctx, page } = await open({ version: "0.2.0", notes: "Faster Photos.\nCalendar fixes." });
  const n = notice(page);
  await n.waitFor({ timeout: 4000 });
  const text = await n.innerText();
  ok(/Skykeep 0\.2\.0 is available/.test(text), `the notice names the version  ${JSON.stringify(text.slice(0, 60))}`);
  ok((await n.count()) === 1, "one notice, not several");
  const buttons = await n.getByRole("button").allInnerTexts();
  ok(buttons.includes("Update and Restart") && buttons.includes("Later"), `it offers Update and Restart and Later  ${JSON.stringify(buttons)}`);
  ok(buttons.every((t) => !t.includes("…")), "no button ends in an ellipsis");
  await n.getByText("What’s New").click();
  ok((await n.locator(".update-notes-text").innerText()).includes("Calendar fixes."), "its release notes are a click away");

  // Later: gone, and the backend is told.
  await n.getByRole("button", { name: "Later" }).click();
  await page.waitForTimeout(200);
  ok((await notice(page).count()) === 0, "Later puts the notice away");
  ok(await page.evaluate(() => window.__updateCalls.some(([c]) => c === "update_later")), "and the backend is told");
  await ctx.close();
}

// ── Update and Restart: progress, a wait for unsent changes, then installing ──
{
  const { ctx, page } = await open({ version: "0.2.0" });
  await page.evaluate(() => { window.__updateHold = "changes"; window.__updateStepMs = 300; });
  const n = notice(page);
  await n.waitFor();
  await n.getByRole("button", { name: "Update and Restart" }).click();
  await page.waitForFunction(() => document.querySelector(".update-notice [role=progressbar]"));
  ok(/Downloading Skykeep 0\.2\.0/.test(await n.innerText()), "the download is said, with its progress");
  await page.waitForFunction(() => /\d+%/.test(document.querySelector(".update-percent")?.textContent ?? ""));
  ok(true, "the share downloaded is shown");
  await page.waitForFunction(() => /reach iCloud/.test(document.querySelector(".update-notice")?.textContent ?? ""), null, { timeout: 5000 });
  ok(true, "with changes not yet in iCloud, it says it waits for them");
  ok(!(await page.evaluate(() => window.__restarted ?? false)), "and does not restart");
  await page.evaluate(() => { window.__updateHold = "typing"; });
  await page.waitForFunction(() => /finish typing/.test(document.querySelector(".update-notice")?.textContent ?? ""));
  ok(true, "someone typing: it waits for them to finish");
  await page.evaluate(() => { window.__updateHold = null; });
  await page.waitForFunction(() => /Installing Skykeep 0\.2\.0/.test(document.querySelector(".update-notice")?.textContent ?? ""));
  ok((await n.getByRole("button").count()) === 0, "once clear it installs, with nothing left to press");
  ok(await page.evaluate(() => window.__restarted === true), "and restarts");
  await ctx.close();
}

// ── Typing in this window is reported while an update is near ──
{
  const { ctx, page } = await open({ version: "0.2.0" });
  await notice(page).waitFor();
  await openScreen(page, "Notes");
  await page.locator(".notes").first().waitFor({ state: "attached" });
  await page.waitForTimeout(300);
  const field = page.locator("input, textarea, [contenteditable='true']").first();
  await field.click();
  await page.keyboard.type("abc");
  await page.waitForTimeout(200);
  ok(await page.evaluate(() => window.__typing === true), "typing in this window is told to the backend");
  await ctx.close();
}

// ── A download that fails its signature ──
{
  const { ctx, page } = await open({ version: "0.2.0" });
  await page.evaluate(() => {
    window.__updateFail = "The download didn't match Skykeep's signature, so it wasn't installed. Your current version is unchanged.";
  });
  const n = notice(page);
  await n.waitFor();
  await n.getByRole("button", { name: "Update and Restart" }).click();
  await page.waitForFunction(() => /signature/.test(document.querySelector(".update-notice")?.textContent ?? ""));
  ok((await n.getAttribute("role")) === "alert", "a signature failure is said at once, as an alert");
  const buttons = await n.getByRole("button").allInnerTexts();
  ok(!buttons.includes("Try Again") && !buttons.includes("Update and Restart"), `and nothing offers to install it  ${JSON.stringify(buttons)}`);
  ok(!(await page.evaluate(() => window.__restarted ?? false)), "nothing was installed");
  await ctx.close();
}

// ── Settings: the switch, Check for Updates, and what it finds ──
for (const scheme of ["light", "dark"]) {
  const { ctx, page } = await open({}, { scheme });
  ok((await notice(page).count()) === 0, `${scheme}: up to date, no notice`);
  await openSettings(page);
  await page.locator(".settings").waitFor();
  const group = page.getByRole("region", { name: "Software Update" });
  await group.waitFor();
  ok(/Skykeep 0\.1\.0 is up to date/.test(await group.innerText()), `${scheme}: Settings says it is up to date`);
  const sw = group.getByRole("switch", { name: "Check for Updates Automatically" });
  ok((await sw.getAttribute("aria-checked")) === "true", `${scheme}: checking automatically is on by default`);
  await sw.click();
  ok((await sw.getAttribute("aria-checked")) === "false", `${scheme}: and turns off at once`);
  ok(await page.evaluate(() => window.__updateCalls.some(([c, a]) => c === "update_auto_set" && a.on === false)), `${scheme}: the backend keeps it`);

  // A check that cannot reach GitHub says so plainly.
  await page.evaluate(() => { window.__updateCheckFail = "Couldn't reach GitHub to check for updates. Check your connection and try again."; });
  await group.getByRole("button", { name: "Check for Updates" }).click();
  await page.waitForFunction(() => /Couldn't reach GitHub/.test(document.querySelector('[aria-label="Software Update"]')?.textContent ?? ""));
  ok(true, `${scheme}: a failed check is said plainly`);

  // One that finds a version offers it, here and in the notice.
  await page.evaluate(() => { window.__updateCheckFail = null; window.__updateOffer = { version: "0.3.0" }; });
  await group.getByRole("button", { name: "Check for Updates" }).click();
  await page.waitForFunction(() => /0\.3\.0 is available/.test(document.querySelector('[aria-label="Software Update"]')?.textContent ?? ""));
  ok(await group.getByRole("button", { name: "Update and Restart" }).isVisible(), `${scheme}: a found version can be installed from Settings`);
  await page.evaluate(() => window.__emit?.("quince://update", null));
  await notice(page).waitFor();
  ok((await notice(page).count()) === 1, `${scheme}: and the notice offers it once`);

  // What Skykeep talks to names GitHub's release servers, for updates.
  const reach = await page.locator(".settings-reach").innerText();
  ok(/Updates/.test(reach) && reach.includes("github.com") && reach.includes("release-assets.githubusercontent.com"), `${scheme}: GitHub's release servers are listed for updates`);
  const note = await page.locator(".settings-note").innerText();
  ok(/GitHub for its own updates/.test(note) && !/only to Apple/.test(note), `${scheme}: the footer says what Skykeep connects to  ${JSON.stringify(note)}`);
  await ctx.close();
}

// ── A copy a package manager installed: never offered a self-update ──
{
  const { ctx, page } = await open({ install: "packageManager" });
  await openSettings(page);
  const group = page.getByRole("region", { name: "Software Update" });
  await group.waitFor();
  const text = await group.innerText();
  ok(/Updates come through your package manager/.test(text), "a packaged copy says its package manager updates it");
  ok((await group.getByRole("button").count()) === 0 && (await group.getByRole("switch").count()) === 0, "with nothing to check or switch");
  const reach = await page.locator(".settings-reach").innerText();
  ok(!reach.includes("github.com"), "and lists no GitHub host, since it never contacts one");
  ok((await notice(page).count()) === 0, "and shows no notice");
  await ctx.close();
}

await b.close();
process.exitCode = failures ? 1 : 0;
