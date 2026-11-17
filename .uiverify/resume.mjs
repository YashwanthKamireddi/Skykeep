// Coming back to a lapsed session: Quince signs itself back in, and when Apple wants a code
// before trusting this computer again, the code screen is what opens — not the library, which
// could not refresh, and not the sign-in screen, whose password is already kept.
//
// Nothing is sent to the person's phone until they ask: the window may have opened by itself
// (at login), and a code nobody asked for, pushed to their iPhone, is exactly what they should
// never get. The screen says Apple wants a code and offers Send Code.
import { engine } from "./engine.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1040, height: 720 } });
await ctx.addInitScript(() => {
  window.__TAURI_INTERNALS__ = {
    transformCallback: (cb) => cb,
    invoke: async (cmd) => {
      if (cmd === "auth_status") return { hasStoredSession: true, signedIn: false };
      if (cmd === "auth_resume") return "code";
      if (cmd === "auth_two_factor_options") return { trustedDeviceCount: 1, phones: [], tooManyCodesSent: false };
      if (cmd === "auth_request_code") {
        window.__codeRequests = (window.__codeRequests ?? 0) + 1;
        return null;
      }
      if (cmd === "plugin:event|listen") return 0;
      return [];
    },
  };
});
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await page.waitForTimeout(500);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const where = await page.evaluate(() => ({
  code: Boolean(document.querySelector(".code, input[autocomplete='one-time-code']")),
  library: Boolean(document.querySelector(".sidebar")),
  signIn: Boolean(document.querySelector("input[type='password']")),
}));
ok(where.code && !where.library && !where.signIn, `a lapsed session that needs a code opens on the code screen (${JSON.stringify(where)})`);
ok(!(await page.evaluate(() => window.__codeRequests)), "no code is asked for until the person asks");
ok((await page.locator(".lede").innerText()).includes("Apple wants a code before it trusts this computer again."), "the screen says Apple wants a code, not that one was sent");
const send = page.getByRole("button", { name: "Send Code" });
ok(await send.isVisible(), "and offers Send Code");
await send.click();
await page.waitForTimeout(200);
ok((await page.evaluate(() => window.__codeRequests)) === 1, "Send Code asks Apple once");
ok(!(await send.isVisible()), "then the button goes");
ok((await page.locator(".lede").innerText()).includes("Apple sent a code"), "and the screen says where it went");
await b.close();
process.exitCode = failures ? 1 : 0;
