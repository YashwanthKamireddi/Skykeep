// Settings: Skykeep's own window, opened from the account at the foot of an app's sidebar;
// every switch is the backend's and moves at once, going back with the reason if it could not
// be kept; the appearance chosen applies at once and survives a restart.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openSettings } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1180, height: 820 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
const page = await ctx.newPage();
const BASE = process.env.URL ?? "http://localhost:4173";
await page.goto(`${BASE}/?app=reminders`, { waitUntil: "networkidle" });
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

const card = page.locator(".app-foot .sidebar-account");
ok((await card.innerText()).includes("Asha") && !(await card.innerText()).includes("…"), "the app's sidebar names who is signed in, whole");
await card.click();
await page.waitForTimeout(150);
ok((await page.evaluate(() => window.__windows ?? [])).some((w) => w.cmd === "settings_open"), "the account opens Settings, in Skykeep's own window");
await openSettings(page);
ok(await page.getByRole("heading", { name: "Skykeep", level: 1 }).isVisible(), "which is Skykeep: the account, sync and settings");

const sw = page.getByRole("switch", { name: "Calendar alerts" });
await page.waitForFunction(() => !document.querySelector('[role=switch]')?.disabled);
await sw.click();
ok((await sw.getAttribute("aria-checked")) === "false", "a switch moves the moment it is pressed");
ok((await page.evaluate(() => window.__settings?.eventAlerts)) === false, "and the backend is told");
await page.evaluate(() => { window.__settingsFail = true; });
await sw.click();
await page.waitForTimeout(100);
ok((await sw.getAttribute("aria-checked")) === "false" && (await page.locator(".toast.is-error").isVisible()), "a setting that could not be kept goes back, and says why");
await page.evaluate(() => { window.__settingsFail = false; });

await page.getByRole("button", { name: "Dark", exact: true }).click();
ok((await page.evaluate(() => document.documentElement.dataset.theme)) === "dark", "Dark applies at once");
const bg = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
ok(bg !== "rgb(245, 245, 247)", `the window turns dark whatever the system says  [${bg}]`);
await page.reload({ waitUntil: "networkidle" });
ok((await page.evaluate(() => document.documentElement.dataset.theme)) === "dark", "and is still chosen after a restart");
await page.getByRole("button", { name: "Automatic", exact: true }).click();
ok((await page.evaluate(() => document.documentElement.dataset.theme)) === undefined, "Automatic follows the system again");

// A diagnostic report is written for the person to read and send, and says where.
await openSettings(page);
await page.waitForSelector(".settings");
await page.getByRole("button", { name: "Save Diagnostic Report" }).click();
await page.waitForTimeout(150);
ok((await page.evaluate(() => window.__diagnostics)) === 1 && /Saved to Downloads/.test(await page.locator(".toast").last().innerText()), "a diagnostic report is saved, and the screen says where");

// The account's picture is the photo on the person's own card, where it has one.
// (A plain browser cannot load quince:// — the picture falls back to initials, as it should —
// so what is checked is the picture asked for.)
await ctx.addInitScript(() => {
  window.__mePhoto = "me-card";
  window.__imgs = [];
  new MutationObserver((m) => {
    for (const r of m) for (const n of r.addedNodes) {
      if (n.nodeType !== 1) continue;
      for (const img of [n, ...n.querySelectorAll?.("img") ?? []]) if (img.tagName === "IMG") window.__imgs.push(img.getAttribute("src"));
    }
  }).observe(document, { childList: true, subtree: true });
});
await page.goto(`${BASE}/?app=reminders`, { waitUntil: "networkidle" });
await page.waitForTimeout(300);
const asked = await page.evaluate(() => window.__imgs);
ok(asked.some((s) => s?.includes("me-card")), `the account shows the photo on the person's own card  [${asked.join(", ")}]`);
ok((await page.locator(".app-foot .sidebar-account .account-avatar").innerText()).trim() === "AR", "and its initials hold the place where the photo cannot be had");
// An account that keeps its data from the web gets one plain answer, with Apple's switch.
{
  const c2 = await b.newContext({ viewport: { width: 1180, height: 820 } });
  await c2.addInitScript(() => { window.__webAccess = false; });
  await c2.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
  const p2 = await c2.newPage();
  // In an app's window, where every read would fail; Settings stay usable to sign out.
  await p2.goto(`${BASE}/?app=reminders`, { waitUntil: "networkidle" });
  await p2.waitForTimeout(500);
  ok(await p2.getByText("Access iCloud Data on the Web").first().isVisible(), "web access off says how to turn it on, instead of failing screen by screen");
  await c2.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
