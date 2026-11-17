// An Apple Account that signs in with a security key gets no code — Apple asks for the key,
// which Skykeep can't use yet. The code screen says so plainly, with a way back, instead of six
// empty boxes waiting for a code that never comes (icloudpd#583, #1423).
import { engine } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
const URL = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

async function codeScreen(twoFactor) {
  const ctx = await b.newContext({ viewport: { width: 1100, height: 800 } });
  await ctx.addInitScript(installStub, { screen: "code", services: SERVICES, twoFactor });
  const page = await ctx.newPage();
  await page.goto(URL, { waitUntil: "networkidle" });
  await page.fill('input[aria-label="Apple Account"]', "someone@example.com");
  await page.fill('input[aria-label="Password"]', "not-a-real-password");
  await page.click('button[type="submit"]');
  await page.waitForTimeout(500);
  return { ctx, page };
}

{
  const { ctx, page } = await codeScreen({ trustedDeviceCount: 0, phones: [], tooManyCodesSent: false, securityKey: true });
  ok(await page.getByRole("heading", { name: "This account uses a security key" }).isVisible(), "a security-key account is told so");
  ok((await page.locator(".lede").innerText()).includes("can't sign in with security keys yet"), "plainly, and truly");
  ok(!(await page.locator('[aria-label="Digit 1"]').count()), "no code boxes to wait at");
  await page.getByRole("button", { name: "Use a different Apple Account" }).click();
  await page.waitForTimeout(200);
  ok(await page.locator('input[aria-label="Password"]').isVisible(), "and the way back is there");
  await ctx.close();
}

{
  const { ctx, page } = await codeScreen({ trustedDeviceCount: 1, phones: [], tooManyCodesSent: false });
  ok(await page.locator('[aria-label="Digit 1"]').isVisible(), "an account with codes gets the code screen as before");
  await ctx.close();
}

await b.close();
if (failures) { console.log(`${failures} failed`); process.exit(1); }
console.log("all passed");
