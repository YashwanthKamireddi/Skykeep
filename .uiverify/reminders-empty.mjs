// Today with nothing due: no empty Morning / Afternoon / Tonight headings, only that there
// is nothing — as icloud.com shows it.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: "lists" });
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.locator(".rx-tile", { hasText: "Today" }).click();
await page.waitForTimeout(400);
const seen = await page.evaluate(() => ({
  headings: [...document.querySelectorAll(".rx-section")].map((h) => h.textContent.trim()),
  empty: document.querySelector(".rx-empty")?.textContent.trim() ?? null,
}));
const pass = seen.headings.length === 0 && seen.empty === "No Reminders";
console.log(pass ? "PASS an empty Today says No Reminders, with no empty headings"
  : `FAIL an empty Today: ${JSON.stringify(seen)}`);

// And a Today with something in it heads only the parts of the day that hold something.
const full = await b.newContext({ viewport: { width: 1200, height: 800 } });
await full.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
const p2 = await full.newPage();
await p2.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(p2, "Reminders");
await p2.locator(".rx-tile", { hasText: "Today" }).click();
await p2.waitForTimeout(400);
const empty = await p2.evaluate(() => [...document.querySelectorAll(".rx-section")].filter((h) => {
  const next = h.nextElementSibling;
  return !next || next.querySelectorAll(".rx-row").length === 0;
}).map((h) => h.textContent.trim()));
const tonight = await p2.evaluate(() => {
  const h = [...document.querySelectorAll(".rx-section")].find((x) => x.textContent.trim() === "Tonight");
  return h ? [...h.nextElementSibling.querySelectorAll(".rx-row")].map((r) => r.querySelector(".rx-meta, .rx-due, .rx-sub")?.textContent ?? r.textContent) : [];
});
const times = tonight.map((t) => t.match(/(\d{1,2}):(\d{2})\s*(AM|PM)/i)).filter(Boolean).map((m) => ((+m[1] % 12) + (/p/i.test(m[3]) ? 12 : 0)) * 60 + +m[2]);
const ordered = times.every((t, i) => i === 0 || times[i - 1] <= t);
console.log(ordered ? `PASS Tonight runs in time order  ${JSON.stringify(times)}` : `FAIL Tonight out of order ${JSON.stringify(times)}`);
const pass2 = empty.length === 0 && ordered;
console.log(pass2 ? "PASS a Today with reminders shows no empty headings" : `FAIL empty headings: ${JSON.stringify(empty)}`);
await b.close();
process.exitCode = pass && pass2 ? 0 : 1;
