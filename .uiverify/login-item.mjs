// Settings → Open at Login: off until turned on; on a desktop that runs login entries it says
// what it does; on one that does not (Hyprland without uwsm, Sway), it says so and gives the
// exact line, selectable whole; a refusal puts the switch back with the reason.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openSettings } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
for (const scheme of ["light", "dark"]) {
  const ctx = await b.newContext({ viewport: { width: 1200, height: 900 }, colorScheme: scheme });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
  await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
  await openSettings(page);
  await page.waitForTimeout(400);
  const sw = page.getByRole("switch", { name: "Open at Login" });
  const row = page.locator(".settings-row", { has: sw });
  ok((await sw.getAttribute("aria-checked")) === "false", `${scheme}: off until turned on`);
  await sw.click();
  await page.waitForTimeout(200);
  ok((await sw.getAttribute("aria-checked")) === "true" && /Starts in the background/.test(await row.innerText()), `${scheme}: on, and says what it does`);
  ok((await row.locator(".settings-command").count()) === 0, `${scheme}: no command where the desktop runs it`);
  await sw.click();
  await page.waitForTimeout(200);
  // A desktop that runs no login entries: said plainly, with the line to add.
  await page.evaluate(() => { window.__login = { honoured: false }; });
  await sw.click();
  await page.waitForTimeout(200);
  const cmd = row.locator(".settings-command");
  ok(/doesn't open apps at login by itself/.test(await row.innerText()) && (await cmd.innerText()) === '"/usr/bin/quince-app" --background', `${scheme}: where it is not run, the exact line to add`);
  ok((await cmd.evaluate((e) => getComputedStyle(e).userSelect)) === "all", `${scheme}: selectable whole`);
  const [fg, bg] = await cmd.evaluate((e) => [getComputedStyle(e).color, getComputedStyle(document.body).backgroundColor]);
  ok(fg !== bg, `${scheme}: readable (${fg} on ${bg})`);
  await page.screenshot({ path: `${process.env.TMPDIR ?? "/tmp"}/login-item-${scheme}-${process.env.ENGINE}.png` });
  // Refused (no config folder): back as it was, and why.
  await page.evaluate(() => { window.__loginRefuse = "There is no configuration folder to keep it in."; });
  await sw.click();
  await page.waitForTimeout(200);
  ok((await sw.getAttribute("aria-checked")) === "true" && /no configuration folder/.test(await page.locator(".toast.is-error").innerText()), `${scheme}: refused, back as it was with the reason`);
  await ctx.close();
}
await b.close();
if (failures) process.exitCode = 1;
