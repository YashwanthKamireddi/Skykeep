// A saved session the keyring would not give back (locked at login, or not started yet): the
// sign-in screen says so and what to do, rather than a bare form that reads as signed out.
// A session Apple simply ended says nothing extra.
import { engine } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const KEYRING =
  "Your system keyring is locked or not running. Unlock or start GNOME Keyring or KWallet, then try again.";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

async function open(resumeFails) {
  const ctx = await b.newContext({ viewport: { width: 1040, height: 720 } });
  await ctx.addInitScript(installStub, { screen: "signin", services: SERVICES, twoFactor: null });
  await ctx.addInitScript(({ resumeFails, KEYRING }) => {
    const inner = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = async (cmd, args) => {
      if (cmd === "auth_status") return { hasStoredSession: true, signedIn: false, libraryReady: false, notKept: false };
      if (cmd === "auth_resume") {
        if (resumeFails) throw KEYRING;
        return "signin";
      }
      return inner(cmd, args);
    };
  }, { resumeFails, KEYRING });
  const page = await ctx.newPage();
  await page.goto(BASE, { waitUntil: "networkidle" });
  await page.waitForSelector("input[type='password']");
  await page.waitForTimeout(200);
  return { ctx, page };
}

{
  const { ctx, page } = await open(true);
  const line = page.locator("#signin-error");
  ok(await line.isVisible(), "the sign-in screen says why the saved session wasn't opened");
  const said = (await line.innerText()).trim();
  ok(said === KEYRING, `in one line that says what to do  [${said}]`);
  const focused = await page.evaluate(() => document.activeElement?.getAttribute("type"));
  ok(focused !== "password", "the cursor is not put in the password before an address");
  await page.locator("input[type='email'], input[autocomplete='username']").first().fill("a");
  ok(!(await line.count()), "and it goes once the person starts typing");
  await ctx.close();
}
{
  const { ctx, page } = await open(false);
  ok(!(await page.locator("#signin-error").count()), "a session Apple ended shows no error");
  ok(await page.locator(".assurance").isVisible(), "just the usual assurance");
  await ctx.close();
}
await b.close();
console.log(failures ? `\n${failures} FAILED` : "\nall passed");
process.exit(failures ? 1 : 0);
