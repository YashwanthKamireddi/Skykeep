// The window's content security policy (tauri.conf.json): Quince's own code and pictures, its
// local video server and nothing else — no page anywhere on the internet can be loaded into it.
// Every screen, a photo and a note are visited under that exact policy, and not one load the
// app makes is refused by it.
import fs from "node:fs";
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const csp = JSON.parse(fs.readFileSync(new URL("../src-tauri/tauri.conf.json", import.meta.url))).app.security.csp;
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
  calendarFixture: true, notesFixture: true, contactsFixture: true, driveFixture: true, remindersFixture: true });
await ctx.addInitScript(() => {
  window.__refused = [];
  document.addEventListener("securitypolicyviolation", (e) => window.__refused.push(`${e.violatedDirective} ${e.blockedURI}`));
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
// The policy as Tauri sends it, in the page's own head.
// Every window's page: Skykeep's at "/", each app's at "/?app=notes" and so on.
await page.route((url) => url.origin === "http://localhost:4173" && url.pathname === "/", async (route) => {
  const res = await route.fetch();
  const html = (await res.text()).replace("<head>", `<head><meta http-equiv="Content-Security-Policy" content="${csp}">`);
  await route.fulfill({ response: res, body: html, headers: { ...res.headers(), "content-type": "text/html" } });
});
await page.goto("http://localhost:4173/", { waitUntil: "networkidle" });
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
ok(await page.evaluate(() => !!document.querySelector('meta[http-equiv="Content-Security-Policy"]')), "the policy is in force");
for (const s of ["Reminders", "Notes", "Drive", "Photos", "Calendar", "Contacts"]) {
  await openScreen(page, s);
  await page.waitForTimeout(500);
}
await openScreen(page, "Photos");
await page.locator(".px-tile").first().dblclick();
await page.waitForTimeout(600);
await page.keyboard.press("Escape");
await openScreen(page, "Notes");
await page.locator(".note-row").first().click().catch(() => {});
await page.waitForTimeout(400);
// And a page from anywhere else is refused, as it must be.
await page.evaluate(() => { const i = new Image(); i.src = "https://example.com/x.png"; document.body.append(i); });
await page.waitForTimeout(300);
const refused = await page.evaluate(() => window.__refused);
const ours = refused.filter((r) => !r.includes("example.com"));
ok(ours.length === 0, `nothing the app loads is refused  ${JSON.stringify(ours.slice(0, 5))}`);
ok(refused.some((r) => r.includes("example.com")), "a picture from elsewhere on the internet is refused");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
