// Settings' storage on this computer: what the copy takes, read from the backend; Clear lets go
// of the cached pictures only and the figures are read again; a desktop with no notification
// service says so under the alert switches; a folder that cannot be opened says why on its row.
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
  await page.waitForTimeout(300);
  return { ctx, page };
};

{
  const { ctx, page } = await open();
  const text = await page.locator(".settings").innerText();
  ok(/4\.9 GB kept on this computer/.test(text), "the local copy's whole size is on This Computer's card");
  // Apple's storage in Apple's own units, the same figure Drive shows under a folder.
  ok(/182\.2 MB of 5\.37 GB used/.test(text) && /5\.19 GB available/.test(text), "iCloud storage is said as Apple says it");
  ok((await page.locator(".launch-tile", { hasText: "Drive" }).innerText()).includes("5.19 GB available"), "and Drive's tile says what Drive itself says");
  ok(/Cached pictures[\s\S]*4\.2 GB/.test(text), "what can be cleared is said before clearing");
  const clear = page.getByRole("button", { name: "Clear", exact: true });
  await clear.click();
  await page.waitForTimeout(200);
  const after = await page.locator(".settings").innerText();
  ok((await page.evaluate(() => window.__cleared)) === 1, "Clear asks the backend once");
  ok(/4\.2 GB cleared/.test(after) && /772 MB kept on this computer/.test(after), "and the figures are the backend's answer after");
  ok(await clear.isDisabled(), "nothing left to clear, Clear rests");
  ok(!(await page.getByText("no notification service").isVisible()), "a desktop that shows alerts is not warned");
  await ctx.close();
}
{
  const { ctx, page } = await open(() => {
    window.__alertsShown = false;
    window.__driveFolder = { on: true, path: "/home/test/iCloud Drive", problem: null };
    window.__clearFail = "The data folder could not be read.";
    window.__driveOpenFail = "That folder is off.";
  });
  await page.getByRole("button", { name: "Open", exact: true }).click();
  await page.waitForTimeout(150);
  const row = page.locator(".settings-row", { hasText: "Drive folder" });
  ok((await row.innerText()).includes("That folder is off.") && !(await page.locator(".toast.is-error").isVisible()), "a folder that cannot be opened says why on its own row");
  ok(await page.getByText("Alerts can't appear: this desktop has no notification service running.").isVisible(), "a desktop with no notification service is said under the switches");
  await page.getByRole("button", { name: "Clear", exact: true }).click();
  await page.waitForTimeout(200);
  const banner = page.locator(".toast.is-error");
  ok((await banner.innerText()).includes("could not be read"), "a failed clear says why");
  await page.locator(".content").evaluate((el) => { el.scrollTop = el.scrollHeight; });
  await page.waitForTimeout(100);
  const box = await banner.boundingBox();
  ok(box && box.y >= 0 && box.y + box.height <= (page.viewportSize()?.height ?? 900), `the reason stays in sight as the page scrolls  [y=${box?.y}]`);
  await banner.getByRole("button", { name: "Dismiss" }).click();
  await page.waitForTimeout(250);
  ok(!(await banner.isVisible()), "and is dismissed");
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
