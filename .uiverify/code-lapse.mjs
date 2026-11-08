// Skykeep signed itself back in and Apple wants a code before it trusts this computer again.
// Nothing reaches the person's phone until they ask: the library stays, readable, with one calm
// line and Sign In; the code is asked for once they press it — never by the launch, the timer
// or a sync behind their back. The same when it lapses while the window is open.
import { engine } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
const BASE = process.env.BASE ?? process.env.URL ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const LINE = "Sign in again to keep syncing. Your copy on this computer is still here.";

async function open(resume) {
  const ctx = await b.newContext({ viewport: { width: 1180, height: 800 } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: { trustedDeviceCount: 1, phones: [], tooManyCodesSent: false } });
  await ctx.addInitScript((resume) => {
    const inner = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = async (cmd, args) => {
      if (cmd === "auth_status") return { hasStoredSession: true, signedIn: false, libraryReady: true, notKept: false };
      if (cmd === "auth_resume") return resume;
      return inner(cmd, args);
    };
  }, resume);
  const page = await ctx.newPage();
  await page.goto(BASE, { waitUntil: "networkidle" });
  await page.waitForTimeout(500);
  return { ctx, page };
}
const requests = (page) => page.evaluate(() => window.__codeRequests ?? 0);

// At launch.
{
  const { ctx, page } = await open("code");
  ok(await page.locator(".library").isVisible(), "the library opens on what it holds");
  ok(!(await page.locator(".code").count()), "not the code screen by itself");
  ok((await page.locator("body").innerText()).includes(LINE), "one calm line says to sign in again");
  ok((await requests(page)) === 0, "and no code is asked for");
  await page.getByRole("button", { name: "Sign In" }).click();
  await page.locator(".code").waitFor({ timeout: 5000 }).catch(() => {});
  await page.waitForTimeout(200);
  ok(await page.locator(".code").first().isVisible(), "Sign In opens the code screen");
  ok((await requests(page)) === 1, "which asks Apple for the code, once");
  await ctx.close();
}

// While the window is open.
{
  const { ctx, page } = await open("library");
  await page.evaluate(() => window.__emit?.("quince://needs-code", null));
  await page.waitForTimeout(300);
  ok(await page.locator(".library").isVisible(), "a lapse while open keeps the library in front");
  ok((await page.locator("body").innerText()).includes(LINE), "with the same line");
  ok((await requests(page)) === 0, "and nothing sent to the phone");
  await ctx.close();
}

await b.close();
if (failures) { console.log(`${failures} failed`); process.exit(1); }
console.log("all passed");
