// Keys that work from any app: Ctrl+, opens Settings, Ctrl+/ the Keyboard Shortcuts sheet
// (the app in front listed first; Escape or Done closes it, and the keys go back where they
// were). Settings sets Calendar's first day of the week and lists what Skykeep talks to: Apple,
// subscribed calendars, and GitHub's release servers for updates.
//
//   BASE=http://localhost:4320 ENGINE=webkit node .uiverify/app-keys.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen, openSettings } from "./stub.mjs";
const BASE = process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };

for (const scheme of ["light", "dark"]) {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 820 }, colorScheme: scheme });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true, notesFixture: true });
  await ctx.addInitScript(() => { window.__reachSubscribed = ["calendar.example.org"]; });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(BASE, { waitUntil: "networkidle" });
  await openScreen(page, "Calendar");
  await page.waitForTimeout(300);

  // ── Ctrl+/ : every app's keys, the one in front first ──
  await page.keyboard.press("Control+/");
  const sheet = page.getByRole("dialog", { name: "Keyboard Shortcuts" });
  await sheet.waitFor();
  const groups = await sheet.locator("h3").allInnerTexts();
  ok(groups[0] === "Calendar" && groups.includes("Everywhere") && groups.length === 7, `${scheme}: the sheet lists every app, Calendar first  ${JSON.stringify(groups)}`);
  ok((await sheet.locator(".shortcuts-row", { hasText: "Settings" }).locator("kbd").allInnerTexts()).join("+") === "Ctrl+,", `${scheme}: it names Ctrl+, for Settings`);
  const box = await sheet.boundingBox();
  ok(box && box.width <= 760 && box.height <= 820 * 0.88 + 1, `${scheme}: it fits the window  ${JSON.stringify(box)}`);
  ok(await page.evaluate(() => document.activeElement?.textContent === "Done"), `${scheme}: Done has the focus`);
  await page.keyboard.press("Tab");
  ok(await page.evaluate(() => Boolean(document.activeElement?.closest(".shortcuts-sheet"))), `${scheme}: the focus stays in the sheet`);
  await page.keyboard.press("Escape");
  ok((await sheet.count()) === 0, `${scheme}: Escape closes it`);
  await page.keyboard.press("Control+/");
  await sheet.waitFor();
  await sheet.getByRole("button", { name: "Done" }).click();
  ok((await sheet.count()) === 0, `${scheme}: Done closes it`);

  // ── Ctrl+, : Settings ──
  // From an app's own window: Skykeep's window, on Settings (apps.rs settings_open).
  await page.evaluate(() => { window.__windows = []; });
  await page.keyboard.press("Control+,");
  await page.waitForTimeout(200);
  ok((await page.evaluate(() => window.__windows ?? [])).some((w) => w.cmd === "settings_open"), `${scheme}: Ctrl+, opens Settings`);
  await openSettings(page);
  await page.locator(".settings-title").waitFor();

  // ── What Skykeep talks to, from the network lock ──
  const reach = page.getByRole("region", { name: "What Skykeep Talks To" }).or(page.locator(".settings-reach"));
  const text = await reach.first().innerText();
  ok(/apple\.com/.test(text) && /icloud-content\.com/.test(text) && !/\.cn\b/.test(text) && !/China/.test(text), `${scheme}: Apple's domains are listed  ${JSON.stringify(text.slice(0, 160))}`);
  ok(text.includes("calendar.example.org"), `${scheme}: and each subscribed calendar's host`);
  ok(/Updates/.test(text) && text.includes("github.com") && text.includes("release-assets.githubusercontent.com") && !text.includes("api.github.com"),
    `${scheme}: and GitHub's release servers, for updates`);

  // ── First Day of Week ──
  const picker = page.getByRole("button", { name: /First Day of Week/ });
  ok((await picker.innerText()).startsWith("Automatic ("), `${scheme}: the week starts as the locale says  ${await picker.innerText()}`);
  await picker.click();
  await page.getByRole("option", { name: "Monday" }).click();
  ok(await page.evaluate(() => localStorage.getItem("quince.weekStart")) === "1", `${scheme}: Monday is kept`);
  await openScreen(page, "Calendar");
  await page.getByRole("button", { name: "Month", exact: true }).click();
  await page.waitForTimeout(300);
  const firstHead = await page.evaluate(() => [document.querySelector(".cx-weekdays span")?.textContent, document.querySelector(".cx-day-hit")?.getAttribute("aria-label")]);
  ok(firstHead[0] === "Mon" && /^Monday/.test(firstHead[1] ?? ""), `${scheme}: Month starts its rows on Monday, at once  ${JSON.stringify(firstHead)}`);
  await openSettings(page);
  await page.getByRole("button", { name: /First Day of Week/ }).click();
  await page.getByRole("option", { name: /^Automatic/ }).click();
  ok(await page.evaluate(() => localStorage.getItem("quince.weekStart")) === null, `${scheme}: Automatic goes back to the locale's`);
  await ctx.close();
}

// The system's LC_TIME, where the engine reads only LANG: a British clock starts on Monday.
{
  const ctx = await b.newContext({ viewport: { width: 1280, height: 820 }, locale: "en-US" });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
  await ctx.addInitScript(() => { window.__timeLocale = "en-GB"; });
  const page = await ctx.newPage();
  await page.goto(BASE, { waitUntil: "networkidle" });
  await openScreen(page, "Calendar");
  await page.getByRole("button", { name: "Month", exact: true }).click();
  await page.waitForTimeout(400);
  const first = await page.evaluate(() => document.querySelector(".cx-day-hit")?.getAttribute("aria-label") ?? "");
  ok(/^Monday/.test(first), `LC_TIME en_GB under an American LANG starts the week on Monday  ${JSON.stringify(first)}`);
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
