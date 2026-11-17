// Delete and Ctrl+D change the grid at once, while the event's alerts and repeat are still on
// their way from Apple — held here for 2 s, as a slow network holds them. Undo and the copy
// still carry the alerts once they come. And no button or picker says "…".
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1300, height: 850 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
await ctx.addInitScript(() => {
  const wrap = () => {
    const t = window.__TAURI_INTERNALS__;
    if (!t || t.__slow) return;
    const inner = t.invoke;
    t.invoke = async (cmd, args) => {
      if (cmd === "event_alerts" || cmd === "event_repeat") await new Promise((r) => setTimeout(r, 2000));
      return inner(cmd, args);
    };
    t.__slow = true;
  };
  wrap(); document.addEventListener("DOMContentLoaded", wrap);
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); failures++; });
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Calendar");
await page.keyboard.press("Control+2");
await page.waitForTimeout(400);
const saves = () => page.evaluate(() => window.__eventSaves ?? []);

// Ctrl+D on Dentist (whose alert, 15 minutes, the stub keeps at Apple): the copy is drawn at
// once, and made with the alert once it comes.
const dentist = page.locator(".cx-block", { hasText: "Dentist" });
await dentist.first().click();
await page.waitForTimeout(200);
await page.keyboard.press("Escape");
const before = await dentist.count();
// Watched frame by frame until the copy is made: never two of it (the stand-in and the copy).
// (The stub keeps no saved event, so the copy goes again on the read after; that is the stub.)
const watch = (text, until) => page.evaluate(([text, until]) => {
  window.__most = 0; window.__least = Infinity;
  const look = () => {
    if ((window[until] ?? []).length) return;
    const n = [...document.querySelectorAll(".cx-block")].filter((el) => el.textContent.includes(text)).length;
    window.__most = Math.max(window.__most, n); window.__least = Math.min(window.__least, n);
    requestAnimationFrame(look);
  };
  requestAnimationFrame(look);
}, [text, until]);
await watch("Dentist", "__eventSaves");
const t0 = Date.now();
await page.keyboard.press("Control+d");
await page.waitForFunction((n) => document.querySelectorAll(".cx-block").length > 0 &&
  [...document.querySelectorAll(".cx-block")].filter((el) => el.textContent.includes("Dentist")).length > n, before, { timeout: 1500 }).catch(() => {});
const drawn = Date.now() - t0;
ok((await dentist.count()) === before + 1 && drawn < 500, `${engineName}: Ctrl+D draws the copy at once (${drawn} ms), before its alerts come`);
ok((await saves()).length === 0, "the copy is not made before its alerts are known");
await page.waitForTimeout(2600);
const s = await saves();
ok(s.length === 1 && s[0].guid === null && JSON.stringify(s[0].alerts) === "[15]",
  `the copy is made with the event's alerts  ${JSON.stringify(s[0]?.alerts)}`);
ok(await page.evaluate(() => window.__most) === before + 1, "one copy on the grid until it is made, never the stand-in and the copy both");

// Delete: gone from the grid at once; Undo makes it again with its alert.
const review = page.locator(".cx-block", { hasText: "Design review" });
await review.first().click();
await page.waitForTimeout(250);
await page.evaluate(() => document.activeElement?.blur());
await page.keyboard.press("Delete");
const t1 = Date.now();
await page.getByRole("alertdialog").getByRole("button", { name: "Delete" }).click();
await watch("Design review", "__eventDeletes");
await page.waitForFunction(() => ![...document.querySelectorAll(".cx-block")].some((el) => el.textContent.includes("Design review")), null, { timeout: 1500 }).catch(() => {});
const gone = Date.now() - t1;
ok((await review.count()) === 0 && gone < 500, `${engineName}: a deleted event leaves the grid at once (${gone} ms), before its alerts are read`);
const deletes = () => page.evaluate(() => (window.__eventDeletes ?? []).length);
await page.waitForTimeout(2300);
ok((await deletes()) === 1, "the deletion is queued once the alerts are in hand");
ok(await page.evaluate(() => window.__most) === 0, "and it stays off the grid while its alerts are read");
await page.getByRole("button", { name: "Undo" }).click();
await page.waitForTimeout(400);
const again = (await saves()).at(-1);
ok(again?.title === "Design review" && again?.guid === null, `Undo makes it again  ${JSON.stringify(again?.title)}`);

// No label ends in "…": Subscribe says Subscribe; a loading picker says Loading.
const labels = await page.evaluate(() => [...document.querySelectorAll("button, [role=combobox], select, option")]
  .map((el) => (el.getAttribute("aria-label") ?? el.textContent ?? "").trim()).filter((t) => t.endsWith("…")));
ok(labels.length === 0, `no button or picker ends in "…"  ${JSON.stringify(labels)}`);
// Opened while its alerts are still coming, an event's Alert picker says Loading, disabled.
await page.locator(".cx-block", { hasText: "Dentist" }).first().click();
await page.waitForTimeout(300);
const alert = page.getByRole("button", { name: "Alert", exact: true }).first();
const alertText = (await alert.textContent().catch(() => "")) ?? "";
ok(/Loading/.test(alertText) && !alertText.includes("…") && await alert.isDisabled(), `the Alert picker says Loading, disabled, while it loads  [${alertText.trim()}]`);
await page.keyboard.press("Escape");

await b.close();
process.exitCode = failures ? 1 : 0;
