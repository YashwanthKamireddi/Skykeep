// Quick Event under the +: a line read as it is typed, shown as the event it will make, and
// made on Return — or opened in full with Details.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1300, height: 850 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
const page = await ctx.newPage();
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Calendar");
await page.waitForTimeout(300);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const saves = () => page.evaluate(() => window.__eventSaves ?? []);
const local = (dayOffset, h, m = 0) => page.evaluate(([n, h, m]) => {
  const d = new Date(); d.setHours(h, m, 0, 0); d.setDate(d.getDate() + n); return d.getTime() / 1000;
}, [dayOffset, h, m]);

await page.getByRole("button", { name: "New event" }).click();
const dialog = page.getByRole("dialog", { name: "Quick Event" });
ok(await dialog.isVisible(), "the + opens Quick Event");
ok(await dialog.getByRole("button", { name: "Add" }).isDisabled(), "Add waits for something to add");
await page.keyboard.type("Lunch with Asha tomorrow 1pm at Olive");
const preview = dialog.locator(".cx-quick-preview");
const text = (await preview.innerText()).replace(/\s+/g, " ");
ok(/Lunch with Asha/.test(text) && /1 ?PM.*2 ?PM|13:00.*14:00/i.test(text) && /Olive/.test(text) && /Home/.test(text),
  `the preview shows the title, time, place and calendar  [${text}]`);
await page.keyboard.press("Enter");
await page.waitForTimeout(300);
let sent = await saves();
const tomorrow1 = await local(1, 13);
ok(sent.length === 1 && sent[0].guid === null && sent[0].title === "Lunch with Asha" && sent[0].startsAt === tomorrow1 &&
  sent[0].endsAt === tomorrow1 + 3600 && sent[0].location === "Olive" && sent[0].calendarGuid === "home" && !sent[0].allDay,
  `Return saves it as read  ${JSON.stringify(sent[0])}`);
ok(await dialog.count() === 0, "and the field is put away");

// A repeat: Gym every Monday.
await page.getByRole("button", { name: "New event" }).click();
await page.keyboard.type("Gym every Monday 7am");
await page.keyboard.press("Enter");
await page.waitForTimeout(300);
sent = await saves();
const gym = sent[1];
const gymDay = gym && new Date(gym.startsAt * 1000);
ok(gym && gym.title === "Gym" && gymDay.getDay() === 1 && gymDay.getHours() === 7 && gym.repeatChanged === true &&
  gym.repeat?.freq === "weekly", `a weekly repeat is saved with it  ${JSON.stringify(gym)}`);

// Details opens the whole event, filled in, nothing saved yet.
await page.getByRole("button", { name: "New event" }).click();
await page.keyboard.type("Flight Fri 6:30am–9");
await dialog.getByRole("button", { name: /Details/ }).click();
await page.waitForTimeout(300);
const pop = page.getByRole("dialog", { name: "New event" });
ok(await pop.isVisible() && (await pop.getByLabel("Title").inputValue()) === "Flight", "Details opens the event, its title filled in");
ok((await saves()).length === 2, "nothing is saved until it is added");
await page.keyboard.press("Escape");

// Escape closes Quick Event without adding.
await page.getByRole("button", { name: "New event" }).click();
await page.keyboard.type("Something");
await page.keyboard.press("Escape");
ok(await dialog.count() === 0 && (await saves()).length === 2, "Escape closes it, nothing added");
await b.close();
process.exitCode = failures ? 1 : 0;
