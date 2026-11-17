// The library is ready and the saved session can't be taken up. When Apple simply declines it,
// the notice says "Apple signed you out." When the resume failed with a reason (a locked
// account, say), the notice says that reason instead, and the copy on the computer stays.
import { engine } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const LOCKED =
  "Apple has locked this Apple Account for security. Unlock it at iforgot.apple.com, then sign in here again.";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

async function notice(resumeFails) {
  const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
  await ctx.addInitScript(({ resumeFails, LOCKED }) => {
    const inner = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = async (cmd, args) => {
      if (cmd === "auth_status") return { hasStoredSession: true, signedIn: false, libraryReady: true, notKept: false };
      if (cmd === "auth_resume") {
        if (resumeFails) throw LOCKED;
        return "signin";
      }
      return inner(cmd, args);
    };
  }, { resumeFails, LOCKED });
  const page = await ctx.newPage();
  await page.goto(BASE, { waitUntil: "networkidle" });
  await page.waitForFunction(() => /still here/.test(document.body.innerText), null, { timeout: 8000 }).catch(() => {});
  const text = await page.evaluate(() => document.body.innerText);
  await ctx.close();
  return text;
}

const declined = await notice(false);
ok(/Apple signed you out\. Your copy on this computer is still here\./.test(declined), "a declined session says Apple signed you out");
const locked = await notice(true);
ok(locked.includes("Apple has locked this Apple Account for security."), "a locked account says it is locked");
ok(locked.includes("iforgot.apple.com"), "and where to unlock it");
ok(!locked.includes("Apple signed you out"), "and not that Apple signed you out");
ok(locked.includes("Your copy on this computer is still here."), "the copy on this computer is still said to be there");

await b.close();
if (failures) { console.log(`${failures} failed`); process.exit(1); }
console.log("all passed");
