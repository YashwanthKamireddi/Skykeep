// The shell around the apps does no more than it must, and draws nothing half-made.
//
// - The window is shown (window_ready) only once the app in front is drawn with its code in.
// - "Apple signed you out" is a notice over the foot of the content, never a bar pushing the
//   window down; Sign In is on it.
// - In a narrow window the sidebar is there at once when asked for: no slide of a large layer.
// - A sheet still fades in, as the page draws it.
// - A refresh that changed notes is read by Notes when it is in front, not behind another app.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const BASE = process.env.BASE ?? "http://localhost:4173";
const logCalls = () => {
  // Leaving the library lets its listeners go, which the stub has no plugin for.
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ ??= { unregisterListener() {} };
  const wrap = () => {
    const t = window.__TAURI_INTERNALS__;
    if (!t || t.__logged) return;
    const inner = t.invoke;
    t.invoke = async (cmd, args) => {
      (window.__calls ??= []).push(cmd);
      if (cmd === "window_ready") {
        // What is on the page at the moment the window is shown.
        const pane = document.querySelector(".content .screen-slot:not([hidden])");
        window.__readyAt = {
          needsYou: args?.needsYou,
          rows: document.querySelectorAll(".source-row").length,
          drawn: Boolean(pane && pane.querySelector(".notes, .rx, .cx, .contacts, .drive, .px-scroll")),
        };
        return null;
      }
      return inner(cmd, args);
    };
    t.__logged = true;
  };
  wrap(); document.addEventListener("DOMContentLoaded", wrap);
};

{
  const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
  await ctx.addInitScript(logCalls);
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
  await page.goto(`${BASE}/?app=notes`, { waitUntil: "networkidle" });
  await page.waitForTimeout(500);
  const at = await page.evaluate(() => window.__readyAt);
  ok(at && at.needsYou === false && at.rows > 0 && at.drawn,
    `the window is shown once its app is drawn, its own sidebar with it  ${JSON.stringify(at)}`);
  ok((await page.evaluate(() => window.__calls.filter((c) => c === "window_ready").length)) === 1, "and said once");

  // Contacts' own window holds no Notes: a pass that changed notes reads nothing there.
  await openScreen(page, "Contacts");
  await page.waitForTimeout(300);
  await page.evaluate(() => { window.__calls = []; });
  const report = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("sync_status"));
  for (let i = 0; i < 3; i++) {
    await page.evaluate((r) => window.__emit("quince://synced", { ...r, changed: ["notes"] }), report);
    await page.waitForTimeout(80);
  }
  await page.waitForTimeout(200);
  const hidden = await page.evaluate(() => window.__calls.filter((c) => c === "notes_in").length);
  ok(hidden === 0, `three passes that changed notes, read by nothing in Contacts' window  (${hidden} reads)`);

  // Signed out by Apple: one notice over the foot, the window not moved.
  const top = await page.locator(".library").evaluate((e) => e.getBoundingClientRect().top);
  await page.evaluate(() => window.__emit("quince://needs-signin", null));
  await page.waitForTimeout(300);
  const notice = page.locator(".toast-host .toast", { hasText: "Apple signed you out" });
  ok((await notice.count()) === 1, "Apple signing the session out is said in a notice");
  ok((await page.locator(".library").evaluate((e) => e.getBoundingClientRect().top)) === top, "and the window is not pushed down by it");
  const foot = await page.locator(".app-foot").boundingBox();
  ok(foot && foot.y + foot.height <= 800, `the sidebar's foot is still in the window  ${JSON.stringify(foot)}`);
  await notice.getByRole("button", { name: "Sign In" }).click();
  await page.waitForTimeout(300);
  ok((await page.locator(".shell form.sheet").count()) === 1 && (await page.locator(".library").count()) === 0,
    "Sign In on it opens the sign-in");

  await ctx.close();
}

{
  // A narrow app window.
  const ctx = await b.newContext({ viewport: { width: 760, height: 700 } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
  const page = await ctx.newPage();
  await page.goto(`${BASE}/?app=notes`, { waitUntil: "networkidle" });
  await page.waitForTimeout(400);

  // A sheet still fades in, drawn by the page.
  await page.keyboard.press("Control+/");
  const first = await page.locator(".modal-sheet").first().evaluate((e) => e.style.opacity);
  await page.waitForTimeout(400);
  const settled = await page.locator(".modal-sheet").first().evaluate((e) => e.style.opacity);
  ok(first !== "" && Number(first) < 1 && settled === "", `a sheet fades in  (${first} → "${settled}")`);
  await ctx.close();
}

await b.close();
process.exitCode = failures ? 1 : 0;
