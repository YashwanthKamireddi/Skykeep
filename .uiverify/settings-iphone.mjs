// Settings' iPhone card: hidden where the build or system has no iPhone link; otherwise one
// switch, off until turned on (nothing runs for those who don't use it). On, it says the one
// step missing (plug it in, open Skykeep on it), or that it's in use, paused or microphone only,
// and it changes as the phone does, from the backend's event, with nothing polled.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openSettings, openPane } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const open = async (state, everyApp = "setUp") => {
  const ctx = await b.newContext({ viewport: { width: 1180, height: 820 } });
  if (state) await ctx.addInitScript(([s, e]) => { window.__iphone = { state: s, cameraMs: null, microphoneMs: null, everyApp: e }; }, [state, everyApp]);
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await openSettings(page);
  await page.waitForSelector(".settings");
  await page.locator(".prefs-side").waitFor();
  if (state) await openPane(page, "iPhone");
  return { ctx, page };
};
const card = (page) => page.locator('section[aria-labelledby="card-iphone"]');
const tell = (page, change) => page.evaluate((c) => { window.__iphone = { ...window.__iphone, ...c }; window.__emit("quince://iphone", null); }, change);

{
  const { ctx, page } = await open();
  await page.waitForTimeout(300);
  ok((await card(page).count()) === 0 && (await page.getByRole("tab", { name: "iPhone" }).count()) === 0, "no iPhone section where there's no iPhone link (Windows, the Flatpak)");
  await ctx.close();
}
{
  const { ctx, page } = await open("off");
  await card(page).waitFor();
  const sw = card(page).getByRole("switch");
  ok((await sw.getAttribute("aria-checked")) === "false", "off until turned on");
  ok((await card(page).getByText("Show it to every app").count()) === 0, "off: nothing else in the card");
  ok((await page.getByRole("tab", { name: "iPhone" }).getAttribute("aria-selected")) === "true", "a section of its own in the sidebar, not mixed into This Computer");
  await sw.click();
  await card(page).getByText("Plug in your iPhone with a cable and open Skykeep on it.").waitFor({ timeout: 2000 });
  ok(JSON.stringify(await page.evaluate(() => window.__iphoneSet)) === "[true]", "turned on, once");
  ok((await sw.getAttribute("aria-checked")) === "true", "the switch shows on");
  await tell(page, { state: "plugged" });
  await card(page).getByText("Open Skykeep on your iPhone.").waitFor({ timeout: 2000 });
  ok(true, "plugged in: only the step left, as soon as the backend says so");
  await tell(page, { state: "inUse" });
  await card(page).getByText("Your iPhone is this computer's camera and microphone.").waitFor({ timeout: 2000 });
  ok(true, "in use: says so");
  await tell(page, { state: "inUse", cameraMs: 62.4, microphoneMs: 48.6 });
  await card(page).getByText("Live · 62 ms").waitFor({ timeout: 3000 });
  ok(await card(page).getByText("Live · 49 ms").isVisible() && await card(page).getByText("Connected by cable").isVisible(), "in use: each part live, with how quick its picture and sound are");
  await tell(page, { state: "cameraPaused" });
  await card(page).getByText("The camera pauses while your iPhone is locked", { exact: false }).waitFor({ timeout: 2000 });
  ok(true, "iOS paused the camera: says why, and that the microphone stays on");
  await tell(page, { state: "micOnly" });
  await card(page).getByText("Your iPhone is this computer's microphone.", { exact: false }).waitFor({ timeout: 2000 });
  ok(true, "camera turned off on the phone: microphone only, said so");
  const every = card(page).locator(".settings-row", { hasText: "Show it to every app" });
  ok(await every.isVisible(), "on: the webcam for every app is offered while it isn't set up");
  ok(/(\.\.\.|…)/.test(await every.getByRole("button").innerText()) === false, "its button doesn't trail off");
  await every.getByRole("button", { name: "Set Up" }).click();
  await every.waitFor({ state: "detached", timeout: 2000 });
  ok((await page.evaluate(() => window.__everyAppAsked)) === 1, "Set Up asks once, and the row goes once it's ready");
  await sw.click();
  await card(page).getByText("Use your iPhone as this computer's camera and microphone", { exact: false }).waitFor({ timeout: 2000 });
  ok(JSON.stringify(await page.evaluate(() => window.__iphoneSet)) === "[true,false]", "turned off again, once");
  await ctx.close();
}
{
  // On, no v4l2loopback on this system: says what to install, with nothing to press.
  const { ctx, page } = await open("none", "install");
  const every = card(page).locator(".settings-row", { hasText: "Show it to every app" });
  await every.getByText("install v4l2loopback", { exact: false }).waitFor({ timeout: 2000 });
  ok((await every.getByRole("button").count()) === 0, "no module to set up: the step to take instead, no button");
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
