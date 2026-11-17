// Every kind of menu, where menus went wrong: near the window's edges, inside a popover, in a
// small window. Each must open whole and on screen, in one draw (no flicker), take the keys,
// and go on Escape or a click elsewhere; a pop-up's menu drops from its button; a submenu opens beside its line and flips left when the right is full.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const all = { notesFixture: true, calendarFixture: true, contactsFixture: true, driveFixture: true, remindersFixture: true };

/** Watch menus being added: one insertion per opening is drawn once. */
const watch = (page) => page.evaluate(() => {
  window.__menus = 0;
  window.__mw?.disconnect();
  window.__mw = new MutationObserver((m) => {
    for (const r of m) for (const n of r.addedNodes) if (n.nodeType === 1 && (n.matches?.(".row-menu, .picker-menu") || n.querySelector?.(".row-menu, .picker-menu"))) window.__menus++;
  });
  window.__mw.observe(document.body, { childList: true, subtree: true });
});
/** The menu wholly in the window, and on top where it is drawn. */
const onScreen = (page, sel) => page.evaluate((sel) => {
  const els = [...document.querySelectorAll(sel)];
  if (!els.length) return { ok: false, why: "no menu" };
  for (const el of els) {
    const r = el.getBoundingClientRect();
    if (r.left < 0 || r.top < 0 || r.right > innerWidth || r.bottom > innerHeight) return { ok: false, why: `off screen ${JSON.stringify([r.left, r.top, r.right, r.bottom])} in ${innerWidth}x${innerHeight}` };
    const hit = document.elementFromPoint(r.left + r.width / 2, r.top + Math.min(14, r.height / 2));
    if (!el.contains(hit)) return { ok: false, why: `covered by ${hit?.className}` };
  }
  return { ok: true };
}, sel);

for (const size of [{ width: 1280, height: 800 }, { width: 760, height: 520 }]) {
  const label = `${size.width}x${size.height}`;
  const ctx = await b.newContext({ viewport: size });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, ...all });
  const page = await ctx.newPage();
  await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });

  // A right-click menu in the bottom-right corner, and its submenu with no room on the right.
  await openScreen(page, "Contacts");
  await page.waitForTimeout(300);
  await watch(page);
  await page.locator(".contact-row").first().click({ button: "right", position: { x: 5, y: 5 } });
  await page.mouse.click(0, 0); // gone on a click elsewhere
  ok((await page.locator(".row-menu").count()) === 0, `${label} a click elsewhere closes a menu`);
  const row = page.locator(".contact-row").first();
  const box = await row.boundingBox();
  await page.mouse.click(size.width - 6, size.height - 6, { button: "right" }).catch(() => {});
  await row.dispatchEvent("contextmenu", { clientX: size.width - 6, clientY: size.height - 6, bubbles: true });
  await page.waitForTimeout(100);
  let where = await onScreen(page, ".row-menu");
  ok(where.ok, `${label} a right-click menu at the corner stays on screen ${where.why ?? ""}`);
  await page.getByRole("menuitem", { name: "Add to List" }).hover();
  await page.waitForTimeout(100);
  where = await onScreen(page, ".row-menu");
  ok(where.ok && (await page.locator(".row-menu").count()) === 2, `${label} its submenu opens beside it, on screen ${where.why ?? ""}`);
  const [parent, child] = await page.locator(".row-menu").evaluateAll((els) => els.map((e) => e.getBoundingClientRect().toJSON()));
  ok(child && (child.left >= parent.right - 4 || child.right <= parent.left + 4), `${label} beside, not over, its line`);
  await page.keyboard.press("Escape");
  ok((await page.locator(".row-menu").count()) === 1, `${label} Escape closes the submenu first`);
  await page.keyboard.press("Escape");
  ok((await page.locator(".row-menu").count()) === 0, `${label} and then the menu`);
  // Keyboard: Down, Right into the submenu, Enter chooses — and everything closes.
  await row.click({ button: "right" });
  await page.waitForTimeout(80);
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowRight");
  await page.waitForTimeout(80);
  ok((await page.locator(".row-menu").count()) === 2 && (await page.locator(".row-menu").nth(1).locator(".is-active").count()) === 1, `${label} → opens the submenu with its first line ready`);
  await page.keyboard.press("Enter");
  await page.waitForTimeout(80);
  ok((await page.locator(".row-menu").count()) === 0, `${label} choosing in a submenu closes every level`);

  // A pop-up inside the event popover, at the bottom of the window.
  await openScreen(page, "Calendar");
  await page.keyboard.press("Control+2");
  await page.waitForTimeout(300);
  await page.keyboard.press("Control+n");
  await page.waitForTimeout(300);
  await watch(page);
  const picker = page.locator(".cx-popover .picker-button").last();
  const pb = await picker.boundingBox();
  await picker.click();
  await page.waitForTimeout(100);
  where = await onScreen(page, ".picker-menu");
  ok(where.ok, `${label} a pop-up's menu inside a popover is whole and on screen ${where.why ?? ""}`);
  // Dropped from the button, never laid over it: laid over, a menu in a form covered the
  // rows around it. Below, or above where there is no room below.
  const mb = await page.locator(".picker-menu").boundingBox();
  ok(mb && (mb.y >= pb.y + pb.height - 1 || mb.y + mb.height <= pb.y + 1),
    `${label} its menu drops from the button and leaves it in sight  [button ${Math.round(pb.y)}–${Math.round(pb.y + pb.height)}, menu ${Math.round(mb?.y)}–${Math.round(mb?.y + mb?.height)}]`);
  ok((await page.evaluate(() => window.__menus)) === 1, `${label} drawn once, no flicker`);
  await page.keyboard.press("Escape");
  ok((await page.locator(".picker-menu").count()) === 0, `${label} Escape closes a pop-up's menu`);
  ok(await page.evaluate(() => document.activeElement?.classList.contains("picker-button")), `${label} and focus goes back to its button`);
  // By keyboard alone: ↓ opens it, ↓ moves, Return takes the choice.
  const before = await picker.innerText();
  await page.keyboard.press("ArrowDown");
  await page.waitForTimeout(60);
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await page.waitForTimeout(60);
  ok((await picker.innerText()) !== before && (await page.locator(".picker-menu").count()) === 0, `${label} the keyboard alone changes a pop-up  [${before.trim()} → ${(await picker.innerText()).trim()}]`);
  // Chosen with the mouse, the popover it is in stays open with the choice made — the menu
  // is drawn outside the popover, and a press there must not read as a press elsewhere.
  const was = await picker.innerText();
  await picker.click();
  await page.waitForTimeout(80);
  await page.locator(".picker-menu [role=option]:not([aria-selected=true])").first().click();
  await page.waitForTimeout(80);
  ok((await page.locator(".cx-popover").count()) === 1 && (await picker.innerText()) !== was, `${label} a choice made with the mouse keeps the popover open`);
  await page.keyboard.press("Escape");

  // Menu aim: from "Add to List" to its submenu, diagonally across "Edit Contact". The
  // submenu stays up the whole way — it used to close on the line crossed and open again.
  await openScreen(page, "Contacts");
  await page.waitForTimeout(300);
  await page.locator(".contact-row").first().click({ button: "right" });
  const parentLine = page.getByRole("menuitem", { name: "Add to List" });
  await parentLine.hover();
  await page.waitForTimeout(120);
  const crossed = await page.getByRole("menuitem", { name: "Edit Contact" }).boundingBox();
  const target = await page.locator(".row-menu").nth(1).getByRole("menuitem").last().boundingBox();
  await page.evaluate(() => {
    window.__subGone = false;
    const tick = () => { if (document.querySelectorAll(".row-menu").length < 2) window.__subGone = true; if (!window.__stopAim) requestAnimationFrame(tick); };
    window.__stopAim = false;
    requestAnimationFrame(tick);
  });
  await page.mouse.move(crossed.x + crossed.width - 30, crossed.y + crossed.height / 2, { steps: 3 });
  await page.mouse.move(target.x + 20, target.y + target.height / 2, { steps: 3 });
  await page.waitForTimeout(400);
  const gone = await page.evaluate(() => { window.__stopAim = true; return window.__subGone; });
  ok(!gone && (await page.locator(".row-menu").count()) === 2, `${label} a submenu stays open while the pointer crosses a line on its way in`);
  // Resting on another line does still switch to it, a moment later.
  await page.getByRole("menuitem", { name: "Export vCard" }).hover();
  await page.waitForTimeout(450);
  ok((await page.locator(".row-menu").count()) === 1, `${label} resting on another line closes the submenu`);
  await page.keyboard.press("Escape");

  // A pop-up stays open through a scroll that leaves its button where it was — a page
  // settling as it opened used to shut it at once — and goes when its button moves.
  await openScreen(page, "Reminders");
  await page.waitForTimeout(300);
  await page.locator(".rx-rows li").first().hover();
  await page.locator(".rx-trailing .rx-icon-button[aria-expanded]").first().click({ force: true });
  await page.waitForTimeout(300);
  const pop = page.locator(".rx-detail .picker-button").first();
  if (await pop.count()) {
    await pop.click();
    await page.waitForTimeout(100);
    await page.evaluate(() => document.dispatchEvent(new Event("scroll")));
    await page.waitForTimeout(80);
    ok((await page.locator(".picker-menu").count()) === 1, `${label} a scroll that leaves the button in place does not close its menu`);
    await page.keyboard.press("Escape");
  }
  await page.keyboard.press("Escape");

  // The ⋯ that opened a menu closes it again, in one press, and it does not blink back.
  const listRow = page.locator(".contacts-lists .notes-folder-row").nth(1);
  if (await listRow.isVisible()) {
    await listRow.hover();
    const dots = listRow.locator("button[aria-haspopup=menu]");
    await dots.click();
    await page.waitForTimeout(150);
    const opened = await page.locator(".row-menu").count();
    await watch(page);
    await dots.click();
    await page.waitForTimeout(250);
    const reopened = await page.evaluate(() => window.__menus);
    ok(opened === 1 && (await page.locator(".row-menu").count()) === 0 && reopened === 0, `${label} pressing ⋯ again closes its menu, without it opening again  [${opened} open, ${reopened} redrawn]`);
  }
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
