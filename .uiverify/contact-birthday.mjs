// A contact's birthday, as Contacts shows and edits it — the source the Birthdays calendar
// draws from, which could be neither seen nor set here.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 820 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
  calendarFixture: false, notesFixture: false, contactsFixture: true, driveFixture: false, remindersFixture: false });
await ctx.addInitScript(() => {
  const internals = window.__TAURI_INTERNALS__;
  const invoke = internals.invoke;
  internals.invoke = async (cmd, args) => {
    const answer = await invoke(cmd, args);
    if (cmd === "contacts_list" && Array.isArray(answer)) return answer.map((c) => (c.guid === "c1" ? { ...c, birthday: "1604-03-17" } : c));
    return answer;
  };
});
const page = await ctx.newPage();
await page.goto(process.env.URL ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Contacts");
await page.waitForTimeout(400);
await page.locator(".contact-row", { hasText: "Zoya" }).click();
await page.waitForTimeout(300);

let failures = 0;
const ok = (pass, yes, no) => { console.log(pass ? `PASS ${yes}` : `FAIL ${no}`); if (!pass) failures++; };

const card = await page.locator(".contact-detail").innerText();
ok(/birthday\s*(17 March|March 17)/i.test(card) && !card.includes("1604"),
  "the card shows the birthday, without Apple's no-year 1604", `the card: ${card.replace(/\s+/g, " ").slice(0, 160)}`);

await page.getByRole("button", { name: "Edit", exact: true }).click();
await page.waitForTimeout(200);
// Month, day and year as three fields, the year left empty for a birthday Apple keeps without one.
const month = await page.getByLabel("Birthday, month").innerText();
const day = await page.getByLabel("Birthday, day").inputValue();
const year = await page.getByLabel("Birthday, year").inputValue();
ok(/March/.test(month) && day === "17" && year === "", "editing keeps it as a birthday with no year",
  `month ${month}, day ${day}, year ${year}`);
await page.getByLabel("Birthday, year").fill("1998");
await page.getByRole("button", { name: "Save" }).click();
await page.waitForTimeout(300);
const sent = await page.evaluate(() => (window.__contactSaves ?? []).at(-1));
const birthday = JSON.stringify(sent ?? {}).match(/"birthday":"([^"]*)"/)?.[1];
ok(birthday === "1998-03-17", "and a birthday set here goes to iCloud as Apple writes it", `sent: ${birthday}`);

// The year taken away again: Apple's no-year 1604.
await page.locator(".contact-row", { hasText: "Zoya" }).click();
await page.getByRole("button", { name: "Edit", exact: true }).click();
await page.waitForTimeout(200);
await page.getByLabel("Birthday, year").fill("");
await page.getByLabel("Birthday, day").fill("31");
await page.getByRole("button", { name: "Save" }).click();
await page.waitForTimeout(300);
const again = JSON.stringify(await page.evaluate(() => (window.__contactSaves ?? []).at(-1)) ?? {}).match(/"birthday":"([^"]*)"/)?.[1];
ok(again === "1604-03-31", "with no year it goes as Apple writes one", `sent: ${again}`);

await b.close();
process.exitCode = failures === 0 ? 0 : 1;
