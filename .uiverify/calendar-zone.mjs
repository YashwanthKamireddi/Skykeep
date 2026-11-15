// An event's time zone, as Calendar sets one: the popover shows its city and offset, and its
// times on that zone's clock; a zone found by typing a city replaces it and keeps the clock
// time (10:00 in Kolkata becomes 10:00 in New York, nine and a half hours later), and the save
// carries the zone and the moved instants to Apple. An all-day event has no zone row.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
// New York's offset on the day the event is made (today): daylight time in summer, standard in
// winter. The suite was written in October and said GMT−4 always, so it failed every winter.
const day = new Date(); day.setHours(10, 0, 0, 0);
const offset = (zone) => {
  const name = new Intl.DateTimeFormat("en-US", { timeZone: zone, timeZoneName: "shortOffset" }).formatToParts(day).find((p) => p.type === "timeZoneName").value;
  const [, sign, h, m] = /GMT([+-])?(\d+)?(?::(\d+))?/.exec(name);
  return { label: name.replace("-", "−"), hours: (sign === "-" ? -1 : 1) * ((Number(h) || 0) + (Number(m) || 0) / 60) };
};
const NY = offset("America/New_York").label;
// The event starts in Kolkata (GMT+5:30) and keeps its clock time in New York.
const MOVED = offset("Asia/Kolkata").hours - offset("America/New_York").hours;
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
for (const scheme of ["light", "dark"]) {
  const ctx = await b.newContext({ viewport: { width: 1300, height: 850 }, colorScheme: scheme, timezoneId: "Asia/Kolkata" });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
  await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
  await openScreen(page, "Calendar");
  await page.keyboard.press("Control+2");
  await page.waitForTimeout(300);
  await page.locator(".cx-block", { hasText: "Design review" }).first().click();
  await page.waitForTimeout(300);
  const zone = page.getByRole("button", { name: /^Time zone:/ });
  ok(/Kolkata, GMT\+5:30/.test(await zone.getAttribute("aria-label")), `${scheme}: the event's zone is shown  (${await zone.getAttribute("aria-label")})`);
  const time = () => page.getByRole("group", { name: "Starts time" }).or(page.getByLabel("Starts time")).first().innerText().catch(() => "");
  const before = await page.evaluate(() => [...document.querySelectorAll(".cx-popover .cx-when")][0]?.innerText ?? "");
  await zone.click();
  await page.keyboard.type("new york");
  await page.waitForTimeout(150);
  // Cities that begin with what is typed come first: "lon" is London before Blanc-Sablon.
  await page.keyboard.press("Control+a");
  await page.keyboard.type("lon");
  await page.waitForTimeout(150);
  ok(/^London/.test((await page.getByRole("option").first().innerText()) ?? ""), `${scheme}: "lon" finds London first  ${JSON.stringify(await page.getByRole("option").allInnerTexts())}`);
  await page.keyboard.press("Control+a");
  await page.keyboard.type("new york");
  await page.waitForTimeout(150);
  const options = await page.getByRole("option").allInnerTexts();
  ok(options.some((o) => /New York/.test(o) && o.includes(NY)), `${scheme}: typing a city finds its zone  ${JSON.stringify(options.slice(0, 3))}`);
  await page.keyboard.press("Enter");
  await page.waitForTimeout(150);
  ok((await zone.getAttribute("aria-label")).includes(`New York, ${NY}`), `${scheme}: chosen, the zone is New York`);
  const timesNow = await page.evaluate(() => [...document.querySelectorAll(".cx-popover .cx-when")].map((w) => [...w.querySelectorAll("input, [role=spinbutton]")].map((i) => i.value ?? i.textContent).join(" ")));
  await page.locator('input[placeholder="New Event"]').first().press("Enter");
  await page.waitForTimeout(300);
  const sent = await page.evaluate(() => window.__eventSaves?.at(-1));
  const was = await page.evaluate(() => {
    const base = Math.floor(new Date(new Date().setHours(0, 0, 0, 0)).getTime() / 1000) + 9 * 3600;
    return base + 3600;
  });
  ok(sent?.timezone === "America/New_York" && sent.startsAt - was === MOVED * 3600 && sent.endsAt - sent.startsAt === 3600,
    `${scheme}: saved in New York at the same clock time, an hour long  (${sent?.timezone}, moved ${(sent?.startsAt - was) / 3600} h)  ${JSON.stringify(timesNow)}`);
  // All day: a date, not an instant — no zone to set.
  await page.locator(".cx-block", { hasText: "Dentist" }).first().click();
  await page.waitForTimeout(300);
  ok((await page.getByRole("button", { name: /^Time zone:/ }).count()) === 1, `${scheme}: a timed event has a zone row`);
  await page.locator(".cx-popover").getByRole("checkbox").first().check();
  await page.waitForTimeout(150);
  ok((await page.getByRole("button", { name: /^Time zone:/ }).count()) === 0, `${scheme}: made all-day, it has none`);
  await page.keyboard.press("Escape");
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
