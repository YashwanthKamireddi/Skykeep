// A detail's label is picked from Apple's own list, so the phone reads it as its own
// "work" or "mobile". Anything else is typed in as a custom label.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 820 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
  calendarFixture: false, notesFixture: false, contactsFixture: true, driveFixture: false, remindersFixture: false });
const page = await ctx.newPage();
await page.goto(process.env.URL ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Contacts");
await page.waitForTimeout(400);
await page.locator(".contact-row", { hasText: "Zoya" }).click();
await page.waitForTimeout(300);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

await page.getByRole("button", { name: "Edit", exact: true }).click();
await page.waitForTimeout(200);
const first = page.getByLabel("Label for detail 1");
ok(/mobile/.test(await first.innerText()) && (await page.getByLabel("Custom label for detail 1").count()) === 0,
  "an existing \"mobile\" shows as Apple's own, not as a custom label");
await first.click();
const offered = await page.getByRole("option").allInnerTexts();
ok(["mobile", "iPhone", "Apple Watch", "work", "Custom"].every((x) => offered.some((o) => o.replace("✓", "").trim() === x)), `Apple's phone labels are offered  ${JSON.stringify(offered)}`);
await page.getByRole("option", { name: "work", exact: true }).click();

const second = page.getByLabel("Label for detail 2");
await second.click();
await page.getByRole("option", { name: "Custom" }).click();
const typed = page.getByLabel("Custom label for detail 2");
ok((await typed.count()) === 1, "Custom opens a field to type a label in");
await typed.fill("gym");

await page.getByRole("button", { name: "Save" }).click();
await page.waitForTimeout(300);
const sent = await page.evaluate(() => (window.__contactSaves ?? []).at(-1));
// The save carries the card as edited first (and the one it began from after it).
const labels = (JSON.stringify(sent ?? {}).match(/"label":"[^"]*"/g) ?? []).slice(0, 2);
ok(JSON.stringify(labels) === JSON.stringify(['"label":"WORK"', '"label":"gym"']), `saved in Apple's spelling, and the custom one as typed  ${JSON.stringify(labels)}`);
await b.close();
process.exitCode = failures === 0 ? 0 : 1;
