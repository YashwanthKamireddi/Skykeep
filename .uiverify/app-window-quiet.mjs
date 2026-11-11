// An app's window does its own app's work and no other's (desktop audit M1, H9, H8):
// - opening it, and coming back to it, update its own service, not all six;
// - it never fetches the other apps' code, which it can never show;
// - a search typed quickly in Notes asks the library once it pauses, not per key.
// Skykeep's own window, which shows every service, still updates them all.
import { engine } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";

const BASE = process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

async function open(search) {
  const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
  await ctx.addInitScript(() => {
    const wrap = () => {
      const t = window.__TAURI_INTERNALS__;
      if (!t || t.__logged) return;
      const inner = t.invoke;
      t.invoke = (cmd, args) => {
        (window.__calls ??= []).push([cmd, args]);
        return inner(cmd, args);
      };
      t.__logged = true;
    };
    wrap(); document.addEventListener("DOMContentLoaded", wrap);
  });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); failures++; });
  const scripts = [];
  page.on("request", (r) => { if (r.resourceType() === "script") scripts.push(new URL(r.url()).pathname); });
  await page.clock.install();
  await page.goto(`${BASE}/${search}`, { waitUntil: "networkidle" });
  await page.clock.runFor(1500);
  await page.waitForLoadState("networkidle");
  const passes = () => page.evaluate(() => window.__calls.filter(([c]) => c === "sync_all" || c === "sync_some").map(([c, a]) => (c === "sync_all" ? "all" : a.services.join(","))));
  return { ctx, page, scripts, passes };
}

// Notes' own window.
{
  const { ctx, page, scripts, passes } = await open("?app=notes");
  await page.locator(".notes").first().waitFor();
  ok(JSON.stringify(await passes()) === '["notes"]', `opening Notes updates Notes alone  ${JSON.stringify(await passes())}`);
  const others = scripts.filter((s) => /(Photos|Drive|Contacts|Reminders|Calendar|Settings)View-/.test(s));
  ok(others.length === 0, `and fetches no other app's code  ${JSON.stringify(others)}`);
  // Back to the window after the half-minute it waits between returns.
  await page.clock.runFor(31_000);
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await page.clock.runFor(200);
  ok(JSON.stringify(await passes()) === '["notes","notes"]', `coming back updates Notes alone  ${JSON.stringify(await passes())}`);

  // A search typed quickly: asked once it pauses.
  await page.evaluate(() => { window.__calls = []; });
  const field = page.getByRole("searchbox", { name: "Search notes" });
  await field.click();
  for (const ch of "curtain") {
    await page.keyboard.type(ch);
    await page.clock.runFor(40);
  }
  // React commits on its own (unfaked) schedule, so the pause's timer is given real moments
  // to be set as well as the clock's time to run out.
  for (let i = 0; i < 8; i++) {
    await page.waitForTimeout(40);
    await page.clock.runFor(100);
  }
  const searches = await page.evaluate(() => window.__calls.filter(([c]) => c === "notes_search").map(([, a]) => a.query));
  ok(searches.length === 1 && searches[0] === "curtain", `a word typed quickly is searched once  ${JSON.stringify(searches)}`);
  ok((await page.locator(".note-row", { hasText: "Curtains" }).count()) >= 1, "and what it finds is listed");
  await ctx.close();
}

// Skykeep's own window still updates every service.
{
  const { ctx, page, passes } = await open("");
  await page.locator(".settings").first().waitFor();
  ok((await passes()).includes("all"), `Skykeep's window updates every service  ${JSON.stringify(await passes())}`);
  await ctx.close();
}

await b.close();
console.log(failures ? `\n${failures} failure(s)` : "\nall passed");
process.exit(failures ? 1 : 0);
