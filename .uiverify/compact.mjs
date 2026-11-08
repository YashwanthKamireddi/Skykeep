// A narrow window, as iPadOS and iPhone lay one out: the sidebar steps aside for the content
// and comes back over it on request, and each two-column screen shows one column at a time —
// the list, or what was opened from it, with a way back.
//
// At 480 wide the old layout left the content 260px beside a 216px sidebar: notes read three
// words to a line, Drive cut every name to five letters, and a month cell said "3 more" over
// the next day.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 480, height: 400 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
  calendarFixture: true, notesFixture: true, contactsFixture: true, driveFixture: true, remindersFixture: true });
const page = await ctx.newPage();
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await page.waitForTimeout(400);

let failures = 0;
const ok = (pass, yes, no) => { console.log(pass ? `PASS ${yes}` : `FAIL ${no}`); if (!pass) failures++; };
const visible = (selector) => page.evaluate((sel) => {
  const el = document.querySelector(sel);
  if (!el) return false;
  const r = el.getBoundingClientRect();
  const cs = getComputedStyle(el);
  return r.width > 0 && r.height > 0 && cs.visibility !== "hidden" && r.right > 0 && r.left < window.innerWidth;
}, selector);
/** Open an app in its own window, as its launcher does (docs/apps-as-windows.md). */
async function go(name) {
  await openScreen(page, name);
  await page.waitForTimeout(400);
}

// Each app's own window, narrow: one column at a time, its own sidebar folded away.
await go("Notes");
// Notes: the list, then the note — titled — then back.
ok((await visible(".notes-list")) && !(await visible(".note-body")), "Notes shows its list alone",
  "Notes shows list and note squeezed side by side");
await page.locator(".note-row", { hasText: "Curtains" }).click();
await page.waitForTimeout(300);
const title = await page.evaluate(() => {
  // The note's first line, set as its title.
  const t = document.querySelector(".note-rich .nf-p");
  return t ? { value: t.textContent, height: t.getBoundingClientRect().height } : null;
});
ok(!(await visible(".notes-list")) && (await visible(".note-body")), "a note opens in its place",
  "the note did not take the list's place");
ok(title?.value === "Curtains" && title.height > 20, `with its title showing  [${title?.value}, ${Math.round(title?.height ?? 0)}px]`,
  `its title is not shown: ${JSON.stringify(title)}`);
await page.locator(".note-body .compact-back").click();
await page.waitForTimeout(300);
ok(await visible(".notes-list"), "and back returns to the list", "back did not return to the list");

// Reminders.
await go("Reminders");
ok((await visible(".rx-side")) && !(await visible(".rx-main")), "Reminders shows its lists alone",
  "Reminders shows lists and reminders side by side");
await page.locator(".rx-tile").first().click();
await page.waitForTimeout(300);
ok(!(await visible(".rx-side")) && (await visible(".rx-main")), "a list opens in their place", "the list did not open in place");
await page.locator(".rx-main .compact-back").click();
await page.waitForTimeout(300);
ok(await visible(".rx-side"), "and back returns to the lists", "back did not return to the lists");

// Contacts.
await go("Contacts");
ok((await visible(".contacts-list")) && !(await visible(".contact-detail")), "Contacts shows its list alone",
  "Contacts shows list and card side by side");
await page.locator(".contact-row").first().click();
await page.waitForTimeout(300);
ok(!(await visible(".contacts-list")) && (await visible(".contact-detail")), "a person opens in its place",
  "the card did not open in place");

// A month too narrow for titles draws dots, not "N more" spilling over the next day.
await go("Calendar");
await page.getByRole("button", { name: "Month", exact: true }).click();
await page.waitForTimeout(400);
const month = await page.evaluate(() => ({
  dots: document.querySelectorAll(".cx-items.is-dots .cx-item").length,
  more: [...document.querySelectorAll(".cx-more")].filter((m) => m.getBoundingClientRect().height > 0).length,
}));
ok(month.dots > 0 && month.more === 0, `a narrow month shows its days' items as dots  [${month.dots}]`,
  `a narrow month: ${JSON.stringify(month)}`);

// A small laptop's window: 800 wide, where beside the sidebar a contact's card had 250px and
// wrapped a phone number over four lines. The sidebar steps aside from 900 down.
{
  const mid = await ctx.newPage();
  await mid.setViewportSize({ width: 800, height: 600 });
  await mid.goto("http://localhost:4173", { waitUntil: "networkidle" });
  await openScreen(mid, "Contacts");
  await mid.waitForTimeout(400);
  const phone = await mid.evaluate(() => {
    const a = [...document.querySelectorAll(".contact-detail a")].find((x) => x.textContent.includes("+91"));
    if (!a) return null;
    return { lines: Math.round(a.getBoundingClientRect().height / parseFloat(getComputedStyle(a).lineHeight || "20")), text: a.textContent };
  });
  ok(phone && phone.lines <= 1, "at 800 wide a phone number keeps to one line", `at 800 wide: ${JSON.stringify(phone)}`);
  await mid.close();
}

await b.close();
process.exitCode = failures === 0 ? 0 : 1;
