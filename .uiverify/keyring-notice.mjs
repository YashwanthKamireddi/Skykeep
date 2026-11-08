// No keyring to keep the session (a window manager without one, a session over SSH): signed in
// all the same, and said once, calmly — a quiet line under the account in the sidebar, the
// whole sentence with what fixes it in Settings. With a keyring, nothing is said.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openSettings } from "./stub.mjs";
const BASE = process.env.BASE ?? process.env.URL ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

async function open(notKept) {
  const ctx = await b.newContext({ viewport: { width: 1180, height: 820 } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
  await ctx.addInitScript((notKept) => {
    const inner = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = async (cmd, args) => {
      if (cmd === "auth_status") return { hasStoredSession: true, signedIn: true, libraryReady: true, notKept };
      return inner(cmd, args);
    };
  }, notKept);
  const page = await ctx.newPage();
  // An app's own window: its account card says it, at the foot of the app's sidebar.
  await page.goto(`${BASE}/?app=reminders`, { waitUntil: "networkidle" });
  await page.waitForTimeout(400);
  return { ctx, page };
}

{
  const { ctx, page } = await open(true);
  const line = page.locator(".app-foot .account-status");
  ok(await line.isVisible(), "the sidebar says the sign-in is not saved");
  ok((await line.innerText()).trim() === "Sign-in not saved", `in a few words  [${await line.innerText()}]`);
  ok(/keyring/.test((await page.locator(".app-foot .sidebar-account").getAttribute("title")) ?? ""), "and the whole sentence is under the pointer");
  const fits = await line.evaluate((el) => el.scrollWidth <= el.clientWidth);
  ok(fits, "its words fit the sidebar");
  await openSettings(page);
  await page.waitForSelector(".settings");
  const note = page.locator(".settings-account-note");
  const said = (await note.innerText()).trim();
  ok(await note.isVisible() && /until Skykeep closes/.test(said) && /GNOME Keyring or KWallet/.test(said), `Settings says what it means and what fixes it  [${said}]`);
  ok(!(await page.locator(".settings [role=alert]").count()), "calmly: no alert");
  await ctx.close();
}
{
  const { ctx, page } = await open(false);
  ok(!/not saved/.test((await page.locator(".app-foot .account-status").innerText().catch(() => "")) ?? ""), "with a keyring, the sidebar says nothing");
  await openSettings(page);
  await page.waitForSelector(".settings");
  ok(!(await page.locator(".settings-account-note").count()), "nor does Settings");
  await ctx.close();
}
await b.close();
console.log(failures ? `\n${failures} FAILED` : "\nall passed");
process.exit(failures ? 1 : 0);
