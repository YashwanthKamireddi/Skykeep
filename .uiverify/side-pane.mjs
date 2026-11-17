// Every app window's sidebar button (components/SidePane): at the window's top left in all six
// apps; it hides the sidebar and its column closes, the first row beside it stepping right of
// the button and nothing lying over it; F9 does the same; the choice is kept for that app
// alone. Where Calendar and Drive fold their sidebar away in a narrow window, the button brings
// it over the content, and Escape, a click outside, or (Drive) picking a place puts it back.
// Where the other apps fold theirs into a picker or a column of their own, the button gives way.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const URL_ = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const SIDE = { Notes: ".notes-folders", Reminders: ".rx-side", Calendar: ".cx-side", Contacts: ".contacts-lists", Drive: ".dx-side", Photos: ".px-side" };
const HEAD = { Notes: ".notes-list-head", Reminders: ".rx-head", Calendar: ".cx-bar", Contacts: ".contacts-head", Drive: ".drive-bar", Photos: ".px-bar" };
const open = async (name, width, ctx) => {
  ctx ??= await b.newContext({ viewport: { width, height: 760 } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true,
    calendarFixture: true, remindersFixture: true, contactsFixture: true, driveFixture: true });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `${name}: page error ${e.message}`));
  await page.goto(URL_, { waitUntil: "networkidle" });
  await openScreen(page, name);
  await page.locator(SIDE[name]).first().waitFor({ state: "attached" });
  await page.waitForTimeout(250);
  return { ctx, page };
};
const box = (page, sel) => page.locator(sel).first().evaluate((el) => {
  const r = el.getBoundingClientRect(); const s = getComputedStyle(el);
  return { x: r.x, y: r.y, w: r.width, h: r.height, right: r.right, bottom: r.bottom, vis: s.visibility, display: s.display, position: s.position };
});
const overlaps = (a, c) => a.x < c.right && c.x < a.right && a.y < c.bottom && c.y < a.bottom;
// Where the first row's own words start: its first child that holds text or a control.
const headStart = (page, sel) => page.locator(sel).first().evaluate((el) => {
  const first = [...el.querySelectorAll("h1, h2, button, input, .segmented, nav, [role=group]")].find((e) => e.getBoundingClientRect().width > 0);
  const r = (first ?? el).getBoundingClientRect();
  return { x: r.x, y: r.y, w: r.width, h: r.height, right: r.right, bottom: r.bottom };
});

for (const name of Object.keys(SIDE)) {
  const { ctx, page } = await open(name, 1280);
  const btn = page.locator(".side-toggle");
  const at = await box(page, ".side-toggle");
  // The round glass button (058155d): 16 from the left, centred on the first line beside it,
  // 14 from the top, 18 where a toolbar of taller groups starts the window (Calendar, Photos).
  const top = name === "Calendar" || name === "Photos" ? 18 : 14;
  ok(at.display !== "none" && Math.round(at.x) === 16 && Math.round(at.y) === top, `${name}: the button sits at the window's top left (${Math.round(at.x)}, ${Math.round(at.y)})`);
  ok((await btn.getAttribute("aria-label")) === "Hide Sidebar" && (await btn.getAttribute("aria-expanded")) === "true", `${name}: it says Hide Sidebar while the sidebar shows`);
  // Shown: the button lies over the card's own row, not over any of the sidebar's items.
  const items = await page.locator(`${SIDE[name]} :is(button, input, label, h2, h3, .side-heading)`).evaluateAll((els) =>
    els.map((e) => e.getBoundingClientRect()).filter((r) => r.width > 0).map((r) => ({ x: r.x, y: r.y, right: r.right, bottom: r.bottom })));
  ok(!items.some((r) => overlaps(at, r)), `${name}: shown, nothing of the sidebar lies under the button`);
  const before = await box(page, SIDE[name]);
  await btn.click();
  await page.waitForTimeout(120);
  const hid = await box(page, SIDE[name]);
  ok(hid.vis === "hidden" && hid.w <= 1, `${name}: a click hides the sidebar and closes its column (${Math.round(before.w)} → ${Math.round(hid.w)}px)`);
  ok((await btn.getAttribute("aria-label")) === "Show Sidebar", `${name}: and it says Show Sidebar`);
  const head = await headStart(page, HEAD[name]);
  ok(head.x >= at.right + 4 && !overlaps(at, head), `${name}: the first row steps right of the button (starts at ${Math.round(head.x)}px, button ends ${Math.round(at.right)}px)`);
  const focusable = await page.locator(`${SIDE[name]} :is(button, input)`).evaluateAll((els) => els.filter((e) => e.checkVisibility?.({ visibilityProperty: true }) ?? true).length);
  ok(focusable === 0, `${name}: hidden, none of it can be reached`);
  // Kept for this app: a new window of it opens hidden.
  const again = await ctx.newPage();
  await again.goto(page.url(), { waitUntil: "networkidle" });
  await again.locator(SIDE[name]).first().waitFor({ state: "attached" });
  await again.waitForTimeout(250);
  ok((await box(again, SIDE[name])).vis === "hidden", `${name}: the choice is kept the next time it opens`);
  await again.close();
  // F9 brings it back, at once.
  await page.locator("body").click({ position: { x: 600, y: 700 } }).catch(() => {});
  await page.keyboard.press("F9");
  await page.waitForTimeout(120);
  ok((await box(page, SIDE[name])).vis === "visible" && (await box(page, SIDE[name])).w > 150, `${name}: F9 brings it back`);
  await ctx.close();
}

// The other apps' choices are their own.
{
  const ctx = await b.newContext({ viewport: { width: 1280, height: 760 } });
  const { page } = await open("Notes", 1280, ctx);
  await page.locator(".side-toggle").click();
  await page.close();
  const { page: cal } = await open("Calendar", 1280, ctx);
  ok((await box(cal, ".cx-side")).vis === "visible", "hiding Notes' sidebar leaves Calendar's shown");
  await ctx.close();
}

// Narrow Calendar: its bar fills the first line, so the button gives way and More brings the
// calendars over the month; Escape returns the focus to More. Hidden on a wide window first,
// the choice waits for the window to widen again.
for (const hideFirst of [false, true]) {
  const { ctx, page } = await open("Calendar", 1280);
  if (hideFirst) await page.locator(".side-toggle").click();
  await page.setViewportSize({ width: 760, height: 760 });
  await page.waitForTimeout(300);
  const tag = hideFirst ? "Calendar hidden, then narrowed" : "Calendar at 760px";
  ok(!(await page.locator(".side-toggle").isVisible()), `${tag}: the button gives way to the bar`);
  ok((await page.locator(".cx-bar").evaluate((e) => getComputedStyle(e).paddingLeft)) !== "44px", `${tag}: the bar keeps its whole first line`);
  await page.getByRole("button", { name: "More", exact: true }).click();
  await page.getByRole("menuitem", { name: "Show Calendars" }).click();
  await page.waitForTimeout(150);
  const over = await box(page, ".cx-side");
  ok(over.display !== "none" && over.vis === "visible" && over.position === "fixed" && over.w > 150, `${tag}: More, Show Calendars brings them over the month (${Math.round(over.w)}px)`);
  await page.keyboard.press("Escape");
  await page.waitForTimeout(150);
  ok((await box(page, ".cx-side")).display === "none" && (await page.evaluate(() => document.activeElement?.getAttribute("aria-label"))) === "More", `${tag}: Escape puts them back, the focus on More`);
  await page.keyboard.press("F9");
  await page.waitForTimeout(150);
  ok((await box(page, ".cx-side")).position === "fixed", `${tag}: F9 brings them too`);
  await page.keyboard.press("F9");
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.waitForTimeout(300);
  const wide = await box(page, ".cx-side");
  ok(hideFirst ? wide.vis === "hidden" : wide.vis === "visible", `${tag}: widened again, the sidebar is as it was chosen (${wide.vis})`);
  await ctx.close();
}
{
  const { ctx, page } = await open("Calendar", 480);
  await page.getByRole("button", { name: "Month", exact: true }).click();
  await page.waitForTimeout(200);
  const h = await page.locator(".cx-bar").evaluate((e) => e.getBoundingClientRect().height);
  ok(h < 100, `Calendar at 480px: the bar stays on its one line of controls (${Math.round(h)}px)`);
  await ctx.close();
}

// Narrow Drive brings its sidebar over the content from the button.
for (const [name, width] of [["Drive", 640]]) {
  const { ctx, page } = await open(name, width);
  const btn = page.locator(".side-toggle");
  ok((await box(page, SIDE[name])).display === "none", `${name} at ${width}px: the window folds the sidebar away`);
  ok(await btn.isVisible() && (await btn.getAttribute("aria-label")) === "Show Sidebar", `${name} at ${width}px: the button offers to show it`);
  const at = await box(page, ".side-toggle");
  const head = await headStart(page, HEAD[name]);
  ok(!overlaps(at, head), `${name} at ${width}px: the first row clears the button`);
  await btn.click();
  await page.waitForTimeout(120);
  const over = await box(page, SIDE[name]);
  ok(over.display !== "none" && over.position === "fixed" && Math.round(over.x) === 8, `${name} at ${width}px: a click brings it over the content`);
  ok(await page.evaluate((sel) => !!document.activeElement?.closest(sel), SIDE[name]), `${name} at ${width}px: the focus goes into it`);
  ok((await btn.getAttribute("aria-label")) === "Hide Sidebar", `${name} at ${width}px: and the button offers to hide it`);
  await page.keyboard.press("Escape");
  await page.waitForTimeout(120);
  ok((await box(page, SIDE[name])).display === "none" && (await page.evaluate(() => document.activeElement?.classList.contains("side-toggle"))), `${name} at ${width}px: Escape puts it back, the focus on the button`);
  await btn.click();
  await page.waitForTimeout(100);
  await page.mouse.click(width - 40, 600);
  await page.waitForTimeout(120);
  ok((await box(page, SIDE[name])).display === "none", `${name} at ${width}px: a click outside puts it back`);
  if (name === "Drive") {
    await btn.click();
    await page.waitForTimeout(100);
    await page.locator(".dx-side button", { hasText: "Recents" }).click();
    await page.waitForTimeout(200);
    ok((await box(page, SIDE[name])).display === "none", "Drive at 640px: picking a place puts it back");
  }
  // Grown wide again while it is over the content: back in its column.
  await btn.click();
  await page.waitForTimeout(100);
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.waitForTimeout(300);
  const wide = await box(page, SIDE[name]);
  ok(wide.position !== "fixed" && wide.vis === "visible", `${name}: widened, the sidebar is back in its column`);
  await ctx.close();
}

// Notes, Contacts, Photos and Reminders reach theirs another way when narrow: no button there.
for (const [name, width] of [["Notes", 900], ["Contacts", 900], ["Photos", 680], ["Reminders", 680]]) {
  const { ctx, page } = await open(name, width);
  ok(!(await page.locator(".side-toggle").isVisible()), `${name} at ${width}px: the button gives way to its own picker`);
  await ctx.close();
}

await b.close();
process.exitCode = failures ? 1 : 0;
