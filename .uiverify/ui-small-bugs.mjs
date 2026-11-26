// Two things the 25 Nov design review measured, held: Reminders' sidebar search field keeps its
// height in an 800×600 window (it was squeezed to 16px, its text clipped), and an app window's
// sidebar foot says a failed update in a few words, the whole sentence on hover and to a screen
// reader (it was the sentence itself, cut off mid-word).
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };
const fixtures = { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true, remindersFixture: true };
{
  const ctx = await b.newContext({ viewport: { width: 800, height: 600 } });
  await ctx.addInitScript(installStub, fixtures);
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(`${BASE}/?app=reminders`, { waitUntil: "networkidle" });
  const field = page.locator(".rx-side .search-field").first();
  await field.waitFor({ timeout: 5000 });
  const box = await field.boundingBox();
  ok(box && box.height >= 24, `Reminders' search field keeps its height at 800×600 (${box && box.height}px)`);
  await ctx.close();
}
{
  const SENTENCE = "Couldn't reach iCloud. Check your connection and try again.";
  const ctx = await b.newContext({ viewport: { width: 1200, height: 760 } });
  await ctx.addInitScript((s) => {
    window.__syncExtra = [{ service: "notes", label: "Notes", outcome: "failed", detail: s, lastSyncedAt: Math.floor(Date.now() / 1000) - 600 }];
  }, SENTENCE);
  await ctx.addInitScript(installStub, fixtures);
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(`${BASE}/?app=notes`, { waitUntil: "networkidle" });
  const card = page.locator(".notes-folders .app-foot .sidebar-account");
  await card.waitFor({ timeout: 5000 });
  await page.waitForTimeout(600);
  const text = await card.innerText();
  const title = (await card.getAttribute("title")) ?? "";
  ok(/Offline, retrying/i.test(text), `the foot says it in a few words  ${JSON.stringify(text)}`);
  ok(!text.includes("Check your connection"), "never the sentence itself, cut off");
  ok(title.includes(SENTENCE), `the whole sentence is there on hover  ${JSON.stringify(title)}`);
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
