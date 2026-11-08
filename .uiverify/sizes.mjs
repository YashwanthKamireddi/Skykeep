// Every screen at every size a window is likely to be, checked for what breaks when it is
// resized: the page scrolling sideways, a control cut off by the box it sits in, or anything
// running off the edge of the window.
//
// Screens break at the sizes nobody looked at — the half-screen window, the small laptop —
// so each is opened at a small, an ordinary and a very large size, and in Calendar each of
// its four views too.
import { engine as chromium } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

// The smallest the window may be made, half of a 1080p screen, a small laptop, the owner's
// own maximised window, and a large display.
const SIZES = [
  { width: 480, height: 400 },
  { width: 800, height: 600 },
  { width: 960, height: 1040 },
  { width: 1280, height: 800 },
  { width: 1920, height: 1040 },
  { width: 2560, height: 1440 },
];
const SCREENS = ["Reminders", "Notes", "Drive", "Calendar", "Contacts"];
const CALENDAR_VIEWS = ["Day", "Week", "Month", "Year"];

/** What is broken on the page as it stands, described so a failure says where to look. */
function problems() {
  const out = [];
  const doc = document.documentElement;
  if (doc.scrollWidth > window.innerWidth + 1) out.push(`the page scrolls sideways by ${doc.scrollWidth - window.innerWidth}px`);

  const describe = (el) => {
    const label = el.getAttribute("aria-label") || el.textContent?.trim().slice(0, 30) || el.tagName.toLowerCase();
    return `${el.tagName.toLowerCase()}.${[...el.classList].join(".")} "${label}"`;
  };
  const clippers = (el) => {
    const found = [];
    for (let p = el.parentElement; p && p !== document.body; p = p.parentElement) {
      const cs = getComputedStyle(p);
      if (/(hidden|clip|auto|scroll)/.test(cs.overflowX + cs.overflowY)) found.push(p);
      // Inside something that scrolls, what is out of view is scrolled to, not lost — and
      // nothing further out decides whether it can be reached.
      if (/(auto|scroll)/.test(cs.overflowY) && /(auto|scroll)/.test(cs.overflowX + cs.overflowY)) break;
    }
    return found;
  };

  for (const el of document.querySelectorAll("button, input, select, textarea, [role=checkbox], a[href]")) {
    const r = el.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) continue;
    const style = getComputedStyle(el);
    if (style.visibility === "hidden" || style.opacity === "0") continue;
    // Off the window sideways: a control nobody can reach. (Below the fold is only scrolled to.)
    if (r.right > window.innerWidth + 1 || r.left < -1) {
      out.push(`${describe(el)} runs off the window`);
      continue;
    }
    // Cut off by a box that does not scroll to it: part of a control shown, part not.
    for (const box of clippers(el)) {
      const b = box.getBoundingClientRect();
      const cs = getComputedStyle(box);
      const scrollsX = /(auto|scroll)/.test(cs.overflowX) && box.scrollWidth > box.clientWidth;
      const scrollsY = /(auto|scroll)/.test(cs.overflowY);
      const cutX = !scrollsX && (r.left < b.left - 1 || r.right > b.right + 1) && r.right > b.left && r.left < b.right;
      const cutY = !scrollsY && (r.top < b.top - 1 || r.bottom > b.bottom + 1) && r.bottom > b.top && r.top < b.bottom;
      if (cutX || cutY) {
        out.push(`${describe(el)} is cut off by ${describe(box)}`);
        break;
      }
    }
  }
  return [...new Set(out)];
}

const b = await chromium.launch();
let failures = 0;
for (const size of SIZES) {
  const ctx = await b.newContext({ viewport: size });
  await ctx.addInitScript(installStub, {
    screen: "library", services: SERVICES, twoFactor: null,
    calendarFixture: true, notesFixture: true, contactsFixture: true, driveFixture: true, remindersFixture: true,
  });
  const page = await ctx.newPage();
  await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
  await page.waitForSelector("main.content[data-screen]", { state: "attached" });
  for (const screen of SCREENS) {
    await openScreen(page, screen);
    await page.waitForTimeout(350);
    const views = screen === "Calendar" ? CALENDAR_VIEWS : [null];
    for (const view of views) {
      if (view) {
        await page.getByRole("button", { name: view, exact: true }).click();
        await page.waitForTimeout(250);
      }
      const where = `${screen}${view ? ` (${view})` : ""} at ${size.width}×${size.height}`;
      const found = await page.evaluate(problems);
      if (found.length === 0) {
        console.log(`PASS ${where}`);
      } else {
        failures++;
        console.log(`FAIL ${where}:\n    ${found.slice(0, 6).join("\n    ")}${found.length > 6 ? `\n    …and ${found.length - 6} more` : ""}`);
      }
    }
  }
  await ctx.close();
}
await b.close();
process.exitCode = failures === 0 ? 0 : 1;
