// One design language across the apps: what the kit and tokens promise, held in every screen.
//   - every screen's head starts on the same line: pane titles, column hairlines, source-list headings
//   - a chosen row is the one --fill-selected everywhere, and firms up under Increase Contrast
//   - a pop-up button is the control height and takes the shell's focus ring
//   - small motion stays within 160ms; Drive's row lines are straight, inset hairlines
//   - an empty contact list says so in the middle of its column; "Not synced yet" has a grey dot
// BASE=http://localhost:4201 node .uiverify/design-language.mjs
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen, openSettings } from "./stub.mjs";

const BASE = process.env.BASE ?? "http://localhost:4173";
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const b = await engine.launch();

async function open(opts = {}, scheme = "light") {
  const ctx = await b.newContext({ viewport: { width: 1400, height: 850 }, colorScheme: scheme });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, ...opts });
  const page = await ctx.newPage();
  await page.goto(BASE, { waitUntil: "networkidle" });
  return page;
}
const fixtures = { calendarFixture: true, notesFixture: true, contactsFixture: true, driveFixture: true, remindersFixture: true };
const go = async (page, s) => {
  if (s === "Settings") await openSettings(page);
  else await openScreen(page, s);
  await page.waitForTimeout(400);
};
// What the visible screen's head measures: its title's top, its tallest hairline's top, its first
// source-list heading's left edge.
const head = (page) => page.evaluate(() => {
  const slot = [...document.querySelectorAll("main.content .screen-slot")].find((e) => !e.hidden) ?? document.querySelector("main.content");
  const vis = (el) => el.getClientRects().length && getComputedStyle(el).visibility !== "hidden";
  const h1 = [...slot.querySelectorAll("h1")].find(vis);
  const rule = [...slot.querySelectorAll("*")].find((e) => vis(e) && parseFloat(getComputedStyle(e).borderRightWidth) > 0 && e.getBoundingClientRect().height > 300);
  const heading = [...slot.querySelectorAll(".side-heading")].find(vis);
  return {
    title: h1 ? Math.round(h1.getBoundingClientRect().top) : null,
    rule: rule ? Math.round(rule.getBoundingClientRect().top) : null,
    heading: heading ? Math.round(heading.getBoundingClientRect().left) : null,
  };
});
// A colour as the engine resolves it, for comparing a row's fill against a token.
const resolve = (page, value) => page.evaluate((v) => {
  const probe = document.createElement("div");
  probe.style.background = v;
  document.body.appendChild(probe);
  const c = getComputedStyle(probe).backgroundColor;
  probe.remove();
  return c;
}, value);

// ---- one line for every screen's head --------------------------------------------------------
{
  const page = await open(fixtures);
  const tops = {}, rules = {}, headings = {};
  for (const s of ["Reminders", "Notes", "Drive", "Contacts", "Photos", "Settings"]) {
    await go(page, s);
    const m = await head(page);
    tops[s] = m.title;
    if (m.rule !== null && s !== "Settings") rules[s] = m.rule; // Settings has no columns
    if (m.heading !== null) headings[s] = m.heading;
  }
  ok(new Set(Object.values(tops)).size === 1, `every pane title starts on one line  ${JSON.stringify(tops)}`);
  ok(new Set(Object.values(rules)).size === 1, `every column's hairline starts on that line too  ${JSON.stringify(rules)}`);
  await go(page, "Calendar");
  const cal = await head(page);
  if (cal.heading !== null) headings.Calendar = cal.heading;
  if (cal.rule !== null) rules.Calendar = cal.rule;
  ok(new Set(Object.values(headings)).size === 1, `every source list's first heading starts at one inset  ${JSON.stringify(headings)}`);
  // Calendar's own sidebar is a card now (no hairline): app-window.mjs holds it 8px from the top.

  // ---- one selection ----------------------------------------------------------------------
  const selected = await resolve(page, "var(--fill-selected)");
  const chosen = {};
  await go(page, "Notes");
  chosen["a source row"] = await page.locator(".source-row[aria-current='true']").first().evaluate((el) => getComputedStyle(el).backgroundColor);
  chosen["a note"] = await page.locator(".note-row[aria-selected='true']").first().evaluate((el) => getComputedStyle(el).backgroundColor);
  await go(page, "Contacts");
  await page.locator(".contact-row[aria-selected='true']").first().waitFor();
  // Focus elsewhere, so the list draws its resting selection (the grey one).
  await page.locator(".contacts-search input, .search-field input").first().focus();
  chosen["a contact"] = await page.locator(".contact-row[aria-selected='true']").first().evaluate((el) => getComputedStyle(el).backgroundColor);
  const off = Object.entries(chosen).filter(([, c]) => c !== selected);
  ok(off.length === 0, `a chosen row is --fill-selected everywhere  [${selected}${off.length ? `; off: ${JSON.stringify(Object.fromEntries(off))}` : ""}]`);

  // Under Increase Contrast (the desktop's setting, data-contrast) the selection firms up with the token.
  await page.evaluate(() => document.documentElement.setAttribute("data-contrast", "more"));
  const firm = await resolve(page, "var(--fill-selected)");
  const contact = await page.locator(".contact-row[aria-selected='true']").first().evaluate((el) => getComputedStyle(el).backgroundColor);
  const nav = await page.locator(".source-row[aria-current='true']").first().evaluate((el) => getComputedStyle(el).backgroundColor);
  ok(firm !== selected && contact === firm && nav === firm, `under Increase Contrast the selection firms up in the lists too  [${selected} → ${firm}; contact ${contact}, sidebar ${nav}]`);
  await page.evaluate(() => document.documentElement.removeAttribute("data-contrast"));

  // ---- controls ---------------------------------------------------------------------------
  await go(page, "Settings");
  const control = await page.evaluate(() => parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--h-control")));
  const pickers = await page.locator(".settings .picker-button:visible").evaluateAll((els) => els.map((e) => Math.round(e.getBoundingClientRect().height)));
  ok(pickers.length > 0 && pickers.every((h) => h === control), `Settings' pop-up buttons are the control height  [${pickers.join(", ")} vs ${control}]`);
  const tracks = await page.locator(".settings .segmented:visible").evaluateAll((els) => els.map((e) => Math.round(e.getBoundingClientRect().height)));
  ok(tracks.every((h) => h === control), `a segmented control is the control height  [${tracks.join(", ")}]`);
  const weight = await page.locator(".settings-title").evaluate((el) => getComputedStyle(el).fontWeight);
  ok(weight === "700", `Settings' title has the weight it asks for  [${weight}]`);
  // The pop-up button's keyboard focus: the shell's ring, not a translucent glow.
  const picker = page.locator(".settings .picker-button:visible").first();
  await picker.focus();
  await page.keyboard.press("Shift+Tab");
  await page.keyboard.press("Tab");
  const ring = await page.evaluate(() => {
    const el = document.activeElement;
    if (!el?.classList.contains("picker-button")) return null;
    const cs = getComputedStyle(el);
    return { style: cs.outlineStyle, width: cs.outlineWidth };
  });
  ok(ring?.style === "solid" && ring?.width === "2px", `a focused pop-up button takes the shell's focus ring  ${JSON.stringify(ring)}`);

  // ---- motion stays small -----------------------------------------------------------------
  const motion = await page.evaluate(() => {
    const ms = (v) => Math.max(...v.split(",").map((s) => parseFloat(s) * (s.trim().endsWith("ms") ? 1 : 1000)));
    const quiet = document.querySelector(".quiet");
    const knob = document.querySelector(".switch-knob, span.switch");
    const knobStyle = knob ? getComputedStyle(knob, knob.matches("span.switch") ? "::after" : null) : null;
    return { press: quiet ? ms(getComputedStyle(quiet).transitionDuration) : null, knob: knobStyle ? ms(knobStyle.transitionDuration) : null };
  });
  ok(motion.press !== null && motion.press <= 160, `a button springs back within 160ms  [${motion.press}ms]`);
  ok(motion.knob === null || motion.knob <= 160, `a switch's knob slides within 160ms  [${motion.knob}ms]`);

  // ---- Drive's lines ----------------------------------------------------------------------
  await go(page, "Drive");
  const line = await page.locator(".drive-rows .drive-row").first().evaluate((el) => {
    const after = getComputedStyle(el, "::after");
    return { border: getComputedStyle(el).borderBottomWidth, left: after.left, height: after.height, content: after.content };
  });
  ok(line.border === "0px" && line.content !== "none" && parseFloat(line.left) > 0, `Drive's row line is its own inset hairline, not the rounded row's border  ${JSON.stringify(line)}`);
  const font = await page.locator(".drive-name").first().evaluate((el) => getComputedStyle(el).fontFamily);
  ok(/Adwaita Sans/.test(font), `the type stack offers a face with every weight before system-ui  [${font.slice(0, 90)}]`);
  await page.context().close();
}

// ---- empty and idle ---------------------------------------------------------------------------
{
  const page = await open({});
  await go(page, "Contacts");
  const placed = await page.evaluate(() => {
    const col = document.querySelector(".contacts-list");
    const line = col?.querySelector(".empty-state");
    if (!col || !line) return null;
    const c = col.getBoundingClientRect(), l = line.getBoundingClientRect();
    return { share: ((l.top + l.bottom) / 2 - c.top) / c.height };
  });
  ok(placed !== null && placed.share > 0.25 && placed.share < 0.65, `an empty contact list says so in the middle of its column, as Notes does  ${JSON.stringify(placed)}`);
  await go(page, "Photos");
  // Nothing synced yet: the account card says so plainly, in its quiet state, never the
  // all-clear (no ring turning, no trouble badge).
  const card = await page.evaluate(() => {
    const c = document.querySelector(".app-foot .account-card");
    return { text: c?.querySelector(".account-status")?.textContent, cls: c?.className };
  });
  ok(card.text === "Not synced yet" && /\bis-idle\b/.test(card.cls ?? ""), `"Not synced yet" sits on the account card in its quiet state  ${JSON.stringify(card)}`);
  await page.context().close();
}

await b.close();
process.exitCode = failures ? 1 : 0;
