// Calendar's half of the outbox: an event saved here and not yet at Apple says "Waiting to
// sync" where it opens, and a change Apple turned down shows with what it held — sent again or
// let go, never dropped. The marker goes once the queue says Apple has it.
//
//   BASE=http://localhost:4320 ENGINE=webkit node .uiverify/calendar-outbox.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

for (const scheme of ["light", "dark"]) {
  const ctx = await b.newContext({ viewport: { width: 1300, height: 850 }, colorScheme: scheme });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
  await ctx.addInitScript(() => {
    window.__outbox = {
      waiting: [{ service: "calendar", record: "e1", kind: "save" }],
      refused: [
        { id: 7, service: "calendar", record: "e2", kind: "save", madeAt: 0,
          reason: "This event was changed on another device at the same time.", content: "Dentist at noon" },
        { id: 8, service: "calendar", record: "e2", kind: "delete", madeAt: 0,
          reason: "This event was deleted on another device.", content: "Dentist" },
        // Another app's refusal stays in that app.
        { id: 9, service: "notes", record: "n1", kind: "save", madeAt: 0, reason: "Not this one.", content: "x" },
      ],
    };
  });
  const page = await ctx.newPage();
  await page.goto(BASE, { waitUntil: "networkidle" });
  await openScreen(page, "Calendar");
  await page.keyboard.press("Control+2");
  await page.waitForTimeout(300);
  const tag = `[${engineName} ${scheme}]`;

  // ── Refused: said with why and what it held, one at a time, oldest first ──
  const notice = page.locator(".unsent");
  await notice.waitFor();
  const words = await notice.innerText();
  ok(/changed on another device/.test(words) && words.includes("Dentist at noon"), `${tag} a refused change says why, with what it held`);
  ok(words.includes("(and 1 more)") && !words.includes("Not this one"), `${tag} only Calendar's refusals, the rest counted`);
  const label = await notice.locator("p").first().getAttribute("aria-label");
  ok(label?.startsWith("iCloud didn’t take a change to an event:"), `${tag} a screen reader hears what kind of change it was  ${JSON.stringify(label)}`);
  ok((await notice.getAttribute("role")) === "alert", `${tag} it is announced`);
  const box = await notice.boundingBox();
  // The scrolling day area, not its first event: Week opens scrolled to now, so an event
  // earlier in the day can sit above the visible area — measured against it, any notice failed.
  const grid = await page.locator(".cx-week-grid, .cx-timeline, .cx-scroll").first().boundingBox();
  ok(box && grid && box.y + box.height <= grid.y + 1, `${tag} in the flow above the grid, not over an event`);
  if (scheme === "dark") {
    const bg = await notice.evaluate((el) => getComputedStyle(el).backgroundColor);
    ok(bg !== "rgb(255, 255, 255)", `${tag} the notice follows the dark look  ${bg}`);
  }

  // ── Try Again sends that one again; the next refusal takes its place ──
  await notice.getByRole("button", { name: "Try Again" }).click();
  await page.waitForTimeout(300);
  ok(JSON.stringify(await page.evaluate(() => window.__outboxCalls)) === JSON.stringify([["outbox_resend", 7]]), `${tag} Try Again sends that change again`);
  ok((await notice.innerText()).includes("deleted on another device") && !(await notice.innerText()).includes("more)"), `${tag} the next one shows`);
  ok((await notice.getByRole("button", { name: "Copy Text" }).count()) === 1, `${tag} its words can be copied out`);

  // ── Discard lets it go: Apple's version stands ──
  await notice.getByRole("button", { name: "Discard" }).click();
  await page.waitForTimeout(300);
  ok(JSON.stringify((await page.evaluate(() => window.__outboxCalls)).at(-1)) === JSON.stringify(["outbox_discard", 8]), `${tag} Discard lets it go`);
  ok((await page.locator(".unsent").count()) === 0, `${tag} and the notice leaves`);

  // ── Waiting to sync, where the event opens ──
  await page.locator(".cx-block", { hasText: "Design review" }).first().click();
  const pop = page.getByRole("dialog", { name: "Design review" });
  await pop.waitFor();
  // The popover grows in (lib/fade.ts grow, drawn frame by frame): measured once it has settled.
  await pop.evaluate((el) => new Promise((done) => {
    const until = performance.now() + 1500;
    const look = () => {
      const t = getComputedStyle(el).transform;
      if ((t === "none" || t === "matrix(1, 0, 0, 1, 0, 0)") && getComputedStyle(el).opacity === "1") done();
      else if (performance.now() > until) done();
      else requestAnimationFrame(look);
    };
    look();
  }));
  const marker = pop.locator(".sync-waiting");
  ok((await marker.count()) === 1 && (await marker.innerText()) === "Waiting to sync", `${tag} an event saved here and not yet at Apple says so`);
  ok((await marker.getAttribute("role")) === "status", `${tag} the marker is a status, read out politely`);
  const actions = await pop.locator(".cx-pop-actions").boundingBox();
  const mbox = await marker.boundingBox();
  ok(mbox && actions && mbox.y >= actions.y - 1 && mbox.y + mbox.height <= actions.y + actions.height + 1, `${tag} it sits in the popover's action row`);
  await page.keyboard.press("Escape");
  await page.locator(".cx-block", { hasText: "Dentist" }).first().click();
  const other = page.getByRole("dialog", { name: "Dentist" });
  await other.waitFor();
  ok((await other.locator(".sync-waiting").count()) === 0, `${tag} an event Apple has says nothing`);
  await page.keyboard.press("Escape");

  // ── Sent: the backend says the queue moved, and the marker goes ──
  await page.evaluate(() => { window.__outbox = { waiting: [], refused: [] }; window.__emit("quince://outbox-changed", null); });
  await page.waitForTimeout(300);
  await page.locator(".cx-block", { hasText: "Design review" }).first().click();
  await pop.waitFor();
  ok((await pop.locator(".sync-waiting").count()) === 0, `${tag} once Apple has it, the marker goes`);
  await page.keyboard.press("Escape");

  // ── Refused while the window is open: told at once, without a reload ──
  await page.evaluate(() => {
    window.__outbox = { waiting: [], refused: [{ id: 11, service: "calendar", record: "e1", kind: "delete", madeAt: 0,
      reason: "Apple turned that change down.", content: "Design review" }] };
    window.__emit("quince://outbox-refused", { service: "calendar", record: "e1", message: "Apple turned that change down." });
  });
  await page.locator(".unsent").waitFor({ timeout: 2000 });
  ok((await page.locator(".unsent").innerText()).includes("Design review"), `${tag} a refusal that arrives later shows at once`);
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
