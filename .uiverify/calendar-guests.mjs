// An event's guests: the count, the organizer first, each one's answer in a word and a mark,
// you named as You; an event with none shows no Guests section.
//
//   BASE=http://localhost:4173 ENGINE=webkit node .uiverify/calendar-guests.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };

for (const scheme of ["light", "dark"]) {
  const ctx = await b.newContext({ viewport: { width: 1300, height: 850 }, colorScheme: scheme });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
  await ctx.addInitScript(() => {
    window.__guests = { e1: [
      { name: "Priya Nair", email: "priya@example.com", status: "tentative", isMe: false, isOrganizer: true },
      { name: "Alex Rivera", email: "alex@example.com", status: "accepted", isMe: true, isOrganizer: false },
      { name: null, email: "sam@example.com", status: "declined", isMe: false, isOrganizer: false },
      { name: "Lee", email: null, status: "pending", isMe: false, isOrganizer: false },
    ] };
  });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(BASE, { waitUntil: "networkidle" });
  await openScreen(page, "Calendar");
  await page.keyboard.press("Control+1");
  await page.waitForTimeout(400);

  await page.locator(".cx-block", { hasText: "Design review" }).first().click();
  const pop = page.getByRole("dialog", { name: "Design review" });
  await pop.waitFor();
  const section = pop.getByRole("region", { name: "Guests" });
  await section.waitFor();
  ok((await section.locator(".cx-guests-head").innerText()).trim() === "4 Guests", `${scheme}: the count heads the list`);
  const rows = await section.locator(".cx-guest").allInnerTexts();
  const flat = rows.map((r) => r.replace(/\s+/g, " ").trim());
  ok(/Priya Nair · Organizer/.test(flat[0]) && /Maybe/.test(flat[0]), `${scheme}: the organizer first, with their answer  ${JSON.stringify(flat[0])}`);
  ok(flat.some((r) => /Alex Rivera · You/.test(r) && /Accepted/.test(r)), `${scheme}: you are named You`);
  ok(flat.some((r) => /sam@example\.com/.test(r) && /Declined/.test(r)), `${scheme}: one with no name shows the address`);
  ok(flat.some((r) => /Lee/.test(r) && /No reply/.test(r)), `${scheme}: no answer yet says so`);
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);

  await page.locator(".cx-block", { hasText: "Dentist" }).first().click();
  await page.getByRole("dialog", { name: "Dentist" }).waitFor();
  await page.waitForTimeout(300);
  ok((await page.getByRole("region", { name: "Guests" }).count()) === 0, `${scheme}: an event with no guests shows no Guests`);
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
