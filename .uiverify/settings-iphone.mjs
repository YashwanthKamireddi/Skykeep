// Settings' iPhone row, on This Computer: hidden where the build or system has no iPhone link,
// otherwise the one step that's missing (plug it in, open Skykeep on it), or that it's in use,
// and it changes as the phone does, from the backend's event, with nothing polled.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openSettings } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const open = async (state) => {
  const ctx = await b.newContext({ viewport: { width: 1180, height: 820 } });
  if (state) await ctx.addInitScript((s) => { window.__iphone = { state: s, cameraMs: null, microphoneMs: null, everyApp: "setUp" }; }, state);
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(process.env.URL ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await openSettings(page);
  await page.waitForSelector(".settings");
  await page.locator(".dash-card", { hasText: "This Computer" }).waitFor();
  return { ctx, page };
};
const row = (page) => page.locator(".settings-row", { hasText: "iPhone as camera and microphone" });

{
  const { ctx, page } = await open();
  await page.waitForTimeout(300);
  ok((await row(page).count()) === 0, "no row where there's no iPhone link (Windows, the Flatpak)");
  await ctx.close();
}
{
  // No v4l2loopback on this system: says what to install, with nothing to press.
  const { ctx, page } = await open("none");
  await page.evaluate(() => { window.__iphone.everyApp = "install"; window.__emit("quince://iphone", null); });
  const every = page.locator(".settings-row", { hasText: "Show it to every app" });
  await every.getByText("install v4l2loopback", { exact: false }).waitFor({ timeout: 2000 });
  ok((await every.getByRole("button").count()) === 0, "no module to set up: the step to take instead, no button");
  await ctx.close();
}
{
  const { ctx, page } = await open("none");
  await row(page).waitFor();
  ok((await row(page).innerText()).includes("Plug in your iPhone with a cable and open Skykeep on it."), "no phone: both steps, in one line");
  ok(await page.locator(".dash-card", { hasText: "This Computer" }).locator(".settings-row", { hasText: "iPhone" }).isVisible(), "the row sits on This Computer's card");
  await page.evaluate(() => {
    window.__iphone = { ...window.__iphone, state: "plugged" };
    window.__emit("quince://iphone", null);
  });
  await row(page).getByText("Open Skykeep on your iPhone.").waitFor({ timeout: 2000 });
  ok(true, "plugged in: only the step left, as soon as the backend says so");
  await page.evaluate(() => {
    window.__iphone = { ...window.__iphone, state: "inUse", cameraMs: 31, microphoneMs: 26 };
    window.__emit("quince://iphone", null);
  });
  await row(page).getByText("Your iPhone is this computer's camera and microphone.").waitFor({ timeout: 2000 });
  ok(true, "in use: says so");
  ok((await row(page).locator("button").count()) === 0, "nothing to press: it works by itself");
  const every = page.locator(".settings-row", { hasText: "Show it to every app" });
  ok(await every.isVisible(), "the webcam for every app is offered while it isn't set up");
  ok(/(\.\.\.|…)/.test(await every.getByRole("button").innerText()) === false, "its button doesn't trail off");
  await every.getByRole("button", { name: "Set Up" }).click();
  await every.waitFor({ state: "detached", timeout: 2000 });
  ok((await page.evaluate(() => window.__everyAppAsked)) === 1, "Set Up asks once, and the row goes once it's ready");
  await page.evaluate(() => {
    window.__iphone = { ...window.__iphone, state: "cameraPaused", cameraMs: null };
    window.__emit("quince://iphone", null);
  });
  await row(page).getByText("The camera pauses while your iPhone is locked or another app is open on it.", { exact: false }).waitFor({ timeout: 2000 });
  ok(true, "iOS paused the camera: says why, and that the microphone stays on");
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
