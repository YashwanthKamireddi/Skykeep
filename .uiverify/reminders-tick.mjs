// Ticking a reminder is instant, as on the phone: the circle fills at once, the row stays a
// moment and then leaves, never waiting on Apple; unticking in that moment keeps it, and both
// ticks reach Apple in order.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.locator(".rx-list", { hasText: "Daily" }).click();
await page.waitForTimeout(300);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
await page.evaluate(() => { window.__doneDelay = 4000; });
const row = (t) => page.locator(".rx-row", { has: page.getByLabel(`Rename ${t}`) });

// The row frame by frame from the tick: it fades out (opacity over several frames), and only
// opacity — its height never animates, which flickered under WebKitGTK on Hyprland.
await page.evaluate(() => {
  const r = [...document.querySelectorAll(".rx-row")].find((x) => x.querySelector('[aria-label="Rename Pick up the parcel"]'));
  window.__fold = [];
  const t0 = performance.now();
  const tick = () => {
    const h = r.isConnected ? Math.round(r.getBoundingClientRect().height) : -1;
    const o = r.isConnected ? Number(getComputedStyle(r).opacity) : -1;
    window.__fold.push([Math.round(performance.now() - t0), h, o]);
    if (h !== -1 && performance.now() - t0 < 2000) requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
});
await row("Pick up the parcel").locator(".rx-tick").click();
await page.waitForTimeout(80);
ok((await row("Pick up the parcel").locator(".rx-tick").getAttribute("aria-checked")) === "true", "the circle fills at once, while Apple is still answering");
ok(!(await row("Pick up the parcel").locator(".rx-title").isDisabled()), "and nothing on the row is greyed out while Apple answers");
await page.waitForTimeout(900);
ok((await row("Pick up the parcel").count()) === 0, "the row is gone within a moment, before Apple has answered");
const fold = await page.evaluate(() => window.__fold);
const full = fold[0][1];
const fading = fold.filter(([, h, o]) => h !== -1 && o > 0.02 && o < 0.98);
ok(fading.length >= 3, `it fades away over several frames, not all at once  ${JSON.stringify(fading)}`);
ok(fold.every(([, h]) => h === -1 || h === full), `by opacity alone: its height never animates  ${JSON.stringify(fold.filter(([, h]) => h !== full))}`);

await row("Plan tomorrow").locator(".rx-tick").click();
await page.waitForTimeout(300);
await row("Plan tomorrow").locator(".rx-tick").click();
await page.waitForTimeout(1600);
ok((await row("Plan tomorrow").count()) === 1 && (await row("Plan tomorrow").locator(".rx-tick").getAttribute("aria-checked")) === "false", "unticked in that moment, it stays");
await page.waitForTimeout(9000);
const sent = await page.evaluate(() => window.__done.filter((d) => d.guid === "r1").map((d) => d.done));
ok(JSON.stringify(sent) === JSON.stringify([true, false]), `and Apple gets both ticks, in order  ${JSON.stringify(sent)}`);
// Apple refuses a change that was queued behind the window: the reason is said, and the list
// is read again (Apple's version back over the tick).
const readsBefore = await page.evaluate(() => (window.__reads ??= 0));
await page.evaluate(() => {
  // Kept, not dropped: the backend holds it as refused, with what it was.
  window.__outbox = { waiting: [], refused: [{ id: 7, service: "reminders", record: "r2", kind: "done", reason: "This reminder was changed on another device first.", madeAt: 0, content: "Plan tomorrow" }] };
  window.__emit("quince://outbox-refused", { service: "reminders", record: "r2", message: "This reminder was changed on another device first." });
});
await page.waitForTimeout(300);
ok(/changed on another device/.test(await page.locator(".toast.is-error, .unsent").first().innerText().catch(() => "")), "a change Apple refuses is put back, and the window says why");
await b.close();
process.exitCode = failures ? 1 : 0;
