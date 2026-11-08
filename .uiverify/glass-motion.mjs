// The glass and the motion that answers a person, frame by frame:
//
//   - the engine is marked, and what appears for a moment (a menu) takes the live blur only
//     where it is cheap (Chromium, the engine WebView2 is); in WebKit, WebKitGTK's sibling,
//     it is frosted without one; chrome that stays up over a scroll (toolbars, the sidebar)
//     is frosted in every engine; content never takes any of it;
//   - a segmented control's thumb slides from the old segment to the new one, from the first
//     painted frame, with a cap under the segments on every frame, and is gone once settled;
//   - a popover grows out of what opened it: drawn small and clear on its first frame, whole
//     and without any style of its own once settled; a menu is simply there;
//   - under Reduce Motion, none of it moves.
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };

async function open(reducedMotion) {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 820 }, reducedMotion });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
    notesFixture: true, calendarFixture: true, driveFixture: true });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); failures++; });
  await page.goto(process.env.URL ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await page.locator("main.content[data-screen]").first().waitFor();
  return { ctx, page };
}

/** Click `selector` in the page and record `look` (a function body run in the page) on that
    task and on each of the next `n` frames. */
const frames = (page, click, look, n = 14) => page.evaluate(async ([click, look, n]) => {
  const read = new Function(look);
  const out = [];
  (0, eval)(click);
  out.push(read());
  for (let i = 0; i < n; i++) {
    await new Promise((r) => requestAnimationFrame(r));
    out.push(read());
  }
  return out;
}, [click, look, n]);

// ---- the material ------------------------------------------------------------------------
{
  const { ctx, page } = await open("no-preference");
  const marked = await page.evaluate(() => document.documentElement.dataset.engine ?? null);
  ok(marked === (engineName === "chromium" || engineName === "msedge" ? null : "gtk"), `the window says which engine draws it (${marked})`);
  await openScreen(page, "Drive");
  await page.locator(".drive-row").first().waitFor();
  const filters = await page.evaluate(() => {
    const f = (el) => { if (!el) return null; const cs = getComputedStyle(el); return cs.backdropFilter || cs.webkitBackdropFilter || "none"; };
    return { tool: f(document.querySelector(".toolbar-group")), list: f(document.querySelector(".drive-rows")),
      sidebar: f(document.querySelector(".sidebar")), row: f(document.querySelector(".drive-row")) };
  });
  // Toolbars stay up while lists scroll under them: frosted in every engine, never blurred.
  ok(filters.tool === "none", `a toolbar's capsule is frosted, not blurred (${filters.tool})`);
  ok(filters.list === "none" && filters.row === "none", "the list it sits over is content: no glass");
  ok(filters.sidebar === "none", "the sidebar, which nothing moves under, takes no blur");

  // A menu: there on its first frame, whole; regular glass.
  const r = await page.locator(".drive-row").nth(1).boundingBox();
  const menu = await frames(page, `(() => { const row = document.querySelectorAll(".drive-row")[1];
      row.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: ${r.x + 60}, clientY: ${r.y + r.height / 2}, button: 2 })); })()`,
    `const m = document.querySelector(".row-menu"); if (!m) return null; const cs = getComputedStyle(m);
     return { opacity: Number(cs.opacity), scale: cs.scale, filter: cs.backdropFilter || cs.webkitBackdropFilter || "none" };`, 3);
  const first = menu.slice(1).find(Boolean);
  ok(first && first.opacity === 1 && (first.scale === "none" || first.scale === "1"), `a menu is simply there on its first frame, whole (${JSON.stringify(first)})`);
  ok(first && (marked ? first.filter === "none" : /blur/.test(first.filter)), `a menu is regular glass (${first?.filter})`);
  await page.keyboard.press("Escape");

  // ---- a segmented control's thumb ----
  await openScreen(page, "Calendar");
  await page.locator(".cx").first().waitFor();
  await page.waitForTimeout(300);
  const look = `const g = document.querySelector('.segmented[aria-label="View"]');
    const on = g.querySelector('.segment[aria-pressed="true"]');
    const before = getComputedStyle(g, "::before");
    const sliding = g.classList.contains("is-sliding");
    const gx = g.getBoundingClientRect().left;
    return { sliding, x: sliding ? parseFloat(g.style.getPropertyValue("--thumb-x")) : null,
      onX: on.getBoundingClientRect().left - gx, label: on.textContent,
      capOwn: getComputedStyle(on).backgroundColor, ghost: sliding ? before.backgroundColor : null };`;
  const segs = await page.locator('.segmented[aria-label="View"] .segment').allTextContents();
  const pressed = await page.locator('.segmented[aria-label="View"] .segment[aria-pressed="true"]').textContent();
  const target = segs.indexOf(pressed) === 0 ? segs.length - 1 : 0;
  const slide = await frames(page, `document.querySelectorAll('.segmented[aria-label="View"] .segment')[${target}].click()`, look, 20);
  const painted = slide.slice(1);
  ok(painted[0].sliding, "the thumb is on its way from the first painted frame");
  const xs = painted.filter((f) => f.sliding).map((f) => f.x);
  const toward = Math.sign(painted.at(-1).onX - xs[0]);
  ok(xs.length >= 3 && xs.every((x, i) => i === 0 || Math.sign(x - xs[i - 1]) !== -toward), `it moves one way only, toward the new segment (${xs.map((x) => x.toFixed(0)).join(" ")})`);
  ok(painted.filter((f) => f.sliding).every((f) => f.ghost && f.ghost !== "rgba(0, 0, 0, 0)"), "a cap is drawn on every frame of the way");
  const last = painted.at(-1);
  ok(!last.sliding && last.label === segs[target], `and it settles on ${segs[target]}, the thumb gone, the segment's own cap back`);
  const settledAt = painted.findIndex((f) => !f.sliding);
  ok(settledAt > 0 && settledAt <= 16, `settled in ${settledAt} frames (160ms; at most 16)`);

  // ---- a popover grows out of what opened it ----
  await page.keyboard.press("Control+2");
  await page.waitForTimeout(400);
  const block = page.locator(".cx-block", { hasText: "Dentist" }).first();
  ok(await block.count() > 0, "the week shows the sample event");
  {
    const bb = await block.boundingBox();
    // Each frame's popover from before the click on, the click made as a person makes it (a
    // block opens its popover on the press and release, with the engine's own events).
    await page.evaluate(() => {
      window.__pop = [];
      const t0 = performance.now();
      const rec = () => {
        const p = document.querySelector(".cx-popover");
        if (p) { const cs = getComputedStyle(p); window.__pop.push({ opacity: Number(cs.opacity), scale: p.style.scale || null, origin: p.style.transformOrigin || null }); }
        if (performance.now() - t0 < 3000 && window.__pop.length < 24) requestAnimationFrame(rec);
      };
      requestAnimationFrame(rec);
    });
    await page.mouse.click(bb.x + bb.width / 2, bb.y + bb.height / 2);
    await page.waitForFunction(() => window.__pop.length >= 24 || (window.__pop.length > 0 && window.__pop.at(-1).opacity === 1 && !window.__pop.at(-1).scale), null, { timeout: 5000 }).catch(() => {});
    await page.waitForTimeout(300);
    const pop = [null, ...await page.evaluate(() => window.__pop)];
    const shown = pop.slice(1).filter(Boolean);
    if (shown.length) {
      ok(shown[0].opacity < 1 && Number(shown[0].scale) < 1, `a popover's first frame is small and clear (${JSON.stringify(shown[0])})`);
      ok(shown.every((f, i) => i === 0 || f.opacity >= shown[i - 1].opacity), "it only ever grows clearer");
      const end = shown.at(-1);
      ok(end.opacity === 1 && end.scale === null && end.origin === null, "and settles whole, with no style of its own left");
    } else ok(false, "the event's popover opens");
    await page.keyboard.press("Escape");
  }
  await ctx.close();
}

// ---- Reduce Motion: nothing moves ---------------------------------------------------------
{
  const { ctx, page } = await open("reduce");
  await openScreen(page, "Calendar");
  await page.locator(".cx").first().waitFor();
  await page.waitForTimeout(300);
  const seen = await frames(page, `document.querySelectorAll('.segmented[aria-label="View"] .segment')[1].click()`,
    `return document.querySelector('.segmented[aria-label="View"]').classList.contains("is-sliding");`, 4);
  ok(seen.every((s) => !s), "under Reduce Motion the thumb is simply on the new segment");
  await page.keyboard.press("Control+2");
  await page.waitForTimeout(400);
  const block = page.locator(".cx-block", { hasText: "Dentist" }).first();
  if (await block.count()) {
    await block.click();
    const p = await page.evaluate(() => { const el = document.querySelector(".cx-popover"); return el ? { opacity: getComputedStyle(el).opacity, scale: el.style.scale } : null; });
    ok(p && p.opacity === "1" && !p.scale, "and a popover is simply there, whole");
  }
  await ctx.close();
}

await b.close();
process.exitCode = failures ? 1 : 0;
