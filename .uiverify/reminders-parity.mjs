// Parity with the iPhone's Details screen, on a reminder shaped like the founder's own "Skykeep
// Test": New York 11:00, daily, Urgent, three tags, flagged, Medium, Home arriving, When
// Messaging. Every one shows; the phone's Repeat menu is there in its order; Custom, End Repeat,
// time, flag and priority each send what the phone would. node .uiverify/reminders-parity.mjs [png]
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1300, height: 900 }, colorScheme: "dark" });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
const DUE = 1792681200; // 22 Oct 2026, 11:00 in New York (15:00 UTC)
await ctx.addInitScript((due) => {
  window.__extraReminders = [{ guid: "sk", listGuid: "l1", title: "Skykeep Test", notes: "Parity", dueAt: due, allDay: false,
    timezone: "America/New_York", completedAt: null, priority: 5, flagged: true, parentGuid: null, etag: null,
    recurFrequency: 0, recurInterval: 1 }];
  window.__tagMap = { sk: ["Skykeep", "Tags", "Testing"] };
  window.__details = { sk: { url: "https://example.com", otherAttachments: 2, images: ["Attachment/A1", "Attachment/A2"] } };
  window.__marks = { sk: { urgent: true, location: { title: "Alex\u2019s Home", proximity: "arriving", kind: "Location" },
    messaging: "Alex Rivera", url: "https://www.google.com/" } };
  window.__more = { sk: { timeZone: "America/New_York", urgent: true,
    location: { title: "Alex’s Home", proximity: "arriving", kind: "Location" },
    messaging: { name: "Alex Rivera", handles: ["someone@example.com"] } } };
}, DUE);
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.locator(".rx-list", { hasText: "Daily" }).click();
await page.waitForTimeout(300);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
await page.getByRole("button", { name: "Details for Skykeep Test" }).click();
await page.waitForTimeout(400);
const panel = page.getByRole("group", { name: "Info for Skykeep Test" });
const text = await panel.innerText();
// The row, as the phone's list shows it.
const rowText = (await page.locator(".rx-row", { has: page.getByLabel("Rename Skykeep Test") }).innerText()).replace(/\n/g, " ");
ok(/\(11:00\s?AM GMT-4\)/.test(rowText), `the row adds New York's clock  (${rowText.slice(0, 80)})`);
ok((await page.getByRole("img", { name: "Urgent" }).count()) === 1, "the row carries the Urgent alarm");
ok(rowText.includes("Arriving:") && rowText.includes("Alex’s Home"), "a place chip: Arriving: Alex's Home");
ok(rowText.includes("Messaging:") && rowText.includes("Alex Rivera"), "a person chip: Messaging: Alex Rivera");
ok(rowText.includes("google.com") && !rowText.includes("www."), "a link chip: google.com");
const field = (label) => panel.locator(".rx-field", { has: page.locator(`span:text-is("${label}")`) }).first();

ok((await page.getByLabel("Time of day for Skykeep Test").inputValue()).replace(/\s/g, "").match(/^11:00(AM)?$/i) !== null,
  `the time is New York's 11:00, not this computer's  (${await page.getByLabel("Time of day for Skykeep Test").inputValue()})`);
ok(/New York\s*GMT−4/.test(await field("Time Zone").innerText()), `Time Zone: New York, GMT−4  (${(await field("Time Zone").innerText()).replace(/\n/g, " ")})`);
ok(/Urgent\s*On/.test(text) && text.includes("even in a Focus or on silent"), "Urgent is shown on, with what it does");
ok((await page.getByLabel("Repeat for Skykeep Test", { exact: true }).innerText()).includes("Daily"), "Repeat: Daily");
ok(/End Repeat\s*Never/.test(text), "End Repeat: Never");
ok(["#Skykeep", "#Tags", "#Testing"].every((t) => text.includes(t)), "all three tags");
ok((await page.getByRole("switch", { name: "Flag for Skykeep Test" }).isChecked()) === true, "Flag is on");
ok((await page.getByLabel("Priority for Skykeep Test").innerText()).includes("Medium"), "Priority: Medium");
ok(/Location\s*Alex’s Home\s*Arriving/.test(text), `Location: Alex's Home, Arriving`);
ok(/When Messaging\s*Alex Rivera/.test(text), "When Messaging: the person, by name");
ok((await page.locator(".rx-images .rx-image").count()) === 2, "its two pictures are shown");
ok(!/…/.test(text), "no trailing ellipsis anywhere in the panel");

// The phone's Repeat menu, in its order, with the rules chosen on the phone greyed.
await page.getByLabel("Repeat for Skykeep Test", { exact: true }).click();
await page.waitForTimeout(150);
const menu = page.getByRole("listbox", { name: "Repeat for Skykeep Test", exact: true });
const labels = (await menu.getByRole("option").allInnerTexts()).map((t) => t.replace(/✓/g, "").trim());
ok(JSON.stringify(labels) === JSON.stringify(["Never", "Hourly", "Daily", "Weekdays", "Weekends", "Weekly", "Biweekly", "Monthly", "Every 3 Months", "Every 6 Months", "Yearly", "Custom"]),
  `the phone's Repeat menu, in its order  (${labels.join(", ")})`);
ok((await menu.locator("li.has-divider").count()) === 2, "Never and Custom set apart, as on the phone");
await menu.getByRole("option", { name: "Weekdays" }).click();
await page.waitForTimeout(300);
const last = () => page.evaluate(() => (window.__repeats ?? []).at(-1));
let sent = await last();
ok(sent?.repeat?.frequency === 1 && sent?.repeat?.interval === 1 && JSON.stringify(sent?.daysOfWeek) === "[2,3,4,5,6]", `Weekdays is weekly on Monday to Friday, as the phone writes it  (${JSON.stringify(sent)})`);
ok((await page.locator(".rx-row", { has: page.getByLabel("Rename Skykeep Test") }).locator(".rx-repeat").innerText()).includes("Weekdays"), "and the row says Weekdays");
await page.getByLabel("Repeat for Skykeep Test", { exact: true }).click();
await page.getByRole("option", { name: "Weekends" }).click();
await page.waitForTimeout(300);
sent = await last();
ok(JSON.stringify(sent?.daysOfWeek) === "[1,7]", "Weekends is Saturday and Sunday");
ok((await page.getByLabel("Repeat for Skykeep Test", { exact: true }).innerText()).includes("Weekends"), "and the menu shows it");
await page.getByLabel("Repeat for Skykeep Test", { exact: true }).click();
await page.getByRole("option", { name: "Every 3 Months" }).click();
await page.waitForTimeout(300);
sent = await last();
ok(sent?.repeat?.frequency === 2 && sent?.repeat?.interval === 3 && sent?.endAt === null && sent?.daysOfWeek.length === 0, `Every 3 Months is monthly, every 3  (${JSON.stringify(sent)})`);
ok((await page.getByLabel("Repeat for Skykeep Test", { exact: true }).innerText()).includes("Every 3 Months"), "and the menu shows it at once");

// Custom: Frequency and Every, then the days it falls on.
await page.getByLabel("Repeat for Skykeep Test", { exact: true }).click();
await page.getByRole("option", { name: "Custom" }).click();
await page.waitForTimeout(200);
const custom = page.getByRole("group", { name: "Custom repeat for Skykeep Test" });
ok(await custom.isVisible(), "Custom opens its Frequency and Every");
ok((await page.getByLabel("Frequency for Skykeep Test").innerText()).includes("Monthly"), "with the rule as it stands: Monthly");
ok((await custom.locator(".rx-day.is-on").innerText()).trim() === "22", "on day 22, the day it is due");
await page.getByLabel("Frequency for Skykeep Test").click();
const freqs = (await page.getByRole("listbox", { name: "Frequency for Skykeep Test" }).getByRole("option").allInnerTexts()).map((t) => t.replace(/✓/g, "").trim());
ok(JSON.stringify(freqs) === JSON.stringify(["Hourly", "Daily", "Weekly", "Monthly", "Yearly"]), `Frequency offers the phone's five  (${freqs.join(", ")})`);
await page.getByRole("option", { name: "Weekly" }).click();
await page.waitForTimeout(250);
sent = await last();
ok(sent?.repeat?.frequency === 1 && sent?.repeat?.interval === 3, `Weekly keeps every 3  (${JSON.stringify(sent?.repeat)})`);
ok((await custom.locator(".rx-day.is-on").getAttribute("aria-label")) === "Thursday", "weekly falls on Thursday, the 22nd's weekday in New York");
// Monday, Wednesday, Friday, chosen in Custom.
for (const day of ["Monday", "Wednesday", "Friday", "Thursday"]) {
  await custom.getByRole("button", { name: day }).click();
  await page.waitForTimeout(200);
}
sent = await last();
ok(JSON.stringify(sent?.daysOfWeek) === "[2,4,6]" && sent?.repeat?.interval === 3, `Monday, Wednesday and Friday, every 3 weeks  (${JSON.stringify(sent)})`);
ok((await custom.innerText()).includes("on Monday, Wednesday, and Friday"), "and says so");
await custom.getByRole("button", { name: "More" }).click();
await page.waitForTimeout(250);
sent = await last();
ok(sent?.repeat?.interval === 4, `+ makes it every 4  (${sent?.repeat?.interval})`);
ok((await custom.innerText()).includes("Repeats every 4 weeks on Monday, Wednesday, and Friday."), "and says so, keeping the days");

// End Repeat: On Date sends an end after the due date; Never takes it off.
await page.getByLabel("End repeat for Skykeep Test").click();
await page.getByRole("option", { name: "On Date" }).click();
await page.waitForTimeout(250);
sent = await last();
ok(sent?.endAt > DUE && sent?.repeat?.frequency === 1 && JSON.stringify(sent?.daysOfWeek) === "[2,4,6]", `On Date ends it after the day it is due  (${sent?.endAt && new Date(sent.endAt * 1000).toISOString()})`);
ok(await page.getByLabel("End repeat date for Skykeep Test").isVisible(), "with the date to change");
await page.getByLabel("End repeat for Skykeep Test").click();
await page.getByRole("option", { name: "Never" }).click();
await page.waitForTimeout(250);
sent = await last();
ok(sent?.endAt === null && sent?.repeat?.interval === 4, "Never takes the end off and keeps the repeat");

// The time is read and written on New York's clock.
const time = page.getByLabel("Time of day for Skykeep Test");
await time.fill("10:30 AM");
await time.press("Enter");
await page.waitForTimeout(300);
const edit = await page.evaluate(() => (window.__edits ?? []).filter((e) => "dueAt" in e).at(-1));
ok(edit?.dueAt === DUE - 1800 && edit?.allDay === false, `10:30 is New York's 10:30  (${edit && new Date(edit.dueAt * 1000).toISOString()})`);

// Flag and priority.
await page.getByRole("switch", { name: "Flag for Skykeep Test" }).click({ force: true });
await page.waitForTimeout(250);
ok((await page.evaluate(() => (window.__edits ?? []).some((e) => e.flagged === false))), "the Flag switch unflags it");
await page.getByLabel("Priority for Skykeep Test").click();
await page.getByRole("option", { name: "Low" }).click();
await page.waitForTimeout(250);
ok((await page.evaluate(() => (window.__edits ?? []).some((e) => e.priority === 9))), "Low is Apple's 9");

// A rule whose days were chosen on the phone: shown as Custom, its days left to the phone.
await page.evaluate(() => { window.__more.sk.repeat = { frequency: 1, interval: 1, endAt: null, occurrenceCount: 0, other: ["DaysOfTheWeek"] }; window.__extraReminders[0].recurInterval = 1; });
await page.getByRole("button", { name: "Close details for Skykeep Test" }).click();
await page.getByRole("button", { name: "Details for Skykeep Test" }).click();
await page.waitForTimeout(400);
ok((await page.getByLabel("Repeat for Skykeep Test", { exact: true }).innerText()).includes("Custom"), "a rule with days chosen on the phone reads Custom");
ok((await page.getByRole("group", { name: "Custom repeat for Skykeep Test" }).innerText()).includes("on the days chosen on your iPhone"), "and says where its days were chosen");

if (process.argv[2]) await page.screenshot({ path: process.argv[2] });
console.log(failures ? `${failures} failed` : "all passed");
process.exitCode = failures ? 1 : process.exitCode;
await b.close();
