// The controls for tags, links, early alerts and sections, once REMINDER_DETAIL_WRITES is on
// (RemindersView.tsx, mirroring reminders_details.rs). While it is off this says so and stops:
// reminders-tags and reminders-sections check that none of these controls show.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 900 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
await ctx.addInitScript(() => {
  window.__filterLists = true;
  window.__sections = [{ guid: "s1", listGuid: "l1", title: "Errands", createdAt: 1 }];
});
const page = await ctx.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.locator(".rx-list", { hasText: "Daily" }).click();
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const writes = () => page.evaluate(() => window.__detailWrites ?? []);

await page.getByRole("button", { name: "Details for Pick up the parcel" }).click();
await page.waitForTimeout(300);
if (await page.locator(".rx-detail .rx-tag-input").count() === 0) {
  console.log("SKIP detail writes are off (REMINDER_DETAIL_WRITES = false)");
  await b.close();
  process.exit(0);
}
// Tags: typed, then shown at once and sent.
await page.locator(".rx-detail .rx-tag-input").fill("#urgent");
await page.keyboard.press("Enter");
await page.waitForTimeout(60);
ok((await page.locator(".rx-detail .rx-tag-name").allInnerTexts()).includes("#urgent"), "a typed tag shows at once");
await page.waitForTimeout(300);
ok((await writes()).some((w) => w.cmd === "reminder_tag_add" && w.name === "urgent" && w.guid === "r2"), "and is sent without its #");
ok((await page.locator(".rx-tags .rx-tag-chip").allInnerTexts()).includes("#urgent"), "and joins the sidebar's tags");
await page.getByRole("button", { name: "Remove tag home" }).click();
await page.waitForTimeout(300);
ok(!(await page.locator(".rx-detail .rx-tag-name").allInnerTexts()).includes("#home"), "a tag is taken off");
ok((await writes()).some((w) => w.cmd === "reminder_tag_remove" && w.name === "home"), "and that is sent");
// Link.
const url = page.getByLabel("URL for Pick up the parcel");
await url.fill("https://example.com/parcel");
await url.press("Enter");
await page.waitForTimeout(200);
ok((await writes()).some((w) => w.cmd === "reminder_url_set" && w.url === "https://example.com/parcel"), "a link is saved");
// Early alert.
await page.getByRole("button", { name: /Early reminder for Pick up the parcel/ }).click();
await page.locator(".picker-menu li", { hasText: "15 minutes before" }).click();
await page.waitForTimeout(200);
ok((await writes()).some((w) => w.cmd === "reminder_early_alert_set" && w.count === 15 && w.unit === 0), "an early alert is set");
// Sections: move, make, rename.
await page.getByRole("button", { name: /Section for Pick up the parcel/ }).click();
await page.locator(".picker-menu li", { hasText: "Errands" }).click();
await page.waitForTimeout(300);
ok((await writes()).some((w) => w.cmd === "reminder_section_move" && w.section === "s1" && w.guid === "r2"), "a reminder moves into a section");
const order = await page.evaluate(() => [...document.querySelectorAll(".rx-main .rx-section, .rx-main .rx-row .rx-title")].map((el) => el.classList.contains("rx-section") ? `## ${el.textContent}` : el.value));
ok(order.indexOf("Pick up the parcel") > order.indexOf("## Errands"), `and is shown under it  ${JSON.stringify(order)}`);
await page.getByRole("button", { name: "Add Section" }).click();
await page.keyboard.type("Weekend");
await page.keyboard.press("Enter");
await page.waitForTimeout(300);
ok(await page.locator(".rx-section", { hasText: "Weekend" }).count() === 1, "a new section appears");
ok((await writes()).some((w) => w.cmd === "reminder_section_create" && w.title === "Weekend" && w.listGuid === "l1"), "and is made at Apple");
await page.locator(".rx-section", { hasText: "Errands" }).dblclick();
await page.keyboard.press("Control+a");
await page.keyboard.type("Out and about");
await page.keyboard.press("Enter");
await page.waitForTimeout(300);
ok(await page.locator(".rx-section", { hasText: "Out and about" }).count() === 1, "a section is renamed in place");
ok((await writes()).some((w) => w.cmd === "reminder_section_rename" && w.guid === "s1"), "and that is sent");
ok(errors.length === 0, `no page errors  ${errors.join(" | ")}`);
await b.close();
process.exitCode = failures ? 1 : 0;
