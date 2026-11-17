// Switching between lists: the pane changes heading and reminders together, in one draw —
// never the new list's name over the old list's reminders — and a quick run of switches
// ends on the last one picked.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(() => { window.__slowItems = 150; window.__filterLists = true; });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.waitForTimeout(500);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const lists = await page.locator(".rx-list .rx-list-name").allInnerTexts();
await page.locator(".rx-list", { hasText: "Daily" }).click();
await page.waitForTimeout(400);
await page.evaluate(() => {
  window.__frames = [];
  window.__watch = true;
  const tick = () => {
    const h = document.querySelector(".rx h1")?.textContent?.trim();
    // Daily holds three reminders and GYM one: the count says whose rows are drawn.
    const n = document.querySelectorAll(".rx-row").length;
    window.__frames.push(`${h}::${n === 1 ? "GYM" : n === 3 ? "DAILY" : n}`);
    if (window.__watch) requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
});
await page.locator(".rx-list", { hasText: "GYM" }).click();
await page.waitForTimeout(500);
const frames = await page.evaluate(() => { window.__watch = false; return window.__frames; });
const wrong = frames.filter((f) => (f.startsWith("GYM") && f.endsWith("DAILY")) || (f.startsWith("Daily") && f.endsWith("GYM")));
ok(wrong.length === 0, `heading and reminders change together  [${[...new Set(frames)].join(" → ")}]`);
// The sidebar answers the click at once.
ok((await page.locator('.rx-list[aria-current="true"]').innerText()).includes("GYM"), "the sidebar shows the pick at once");
// Three quick picks: the pane ends on the last.
for (const name of ["Daily", "Life Goals", "GYM"]) await page.locator(".rx-list", { hasText: name }).click();
await page.waitForTimeout(700);
const last = (await page.evaluate(() => window.__frames.at(-1))) ?? "";
const h = await page.locator(".rx h1").first().innerText();
ok(h.includes("GYM"), `a quick run of switches ends on the last one picked  [${h}]`);
await b.close();
process.exitCode = failures ? 1 : 0;
