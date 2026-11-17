// Reminders' tags, custom smart lists, and what the info panel reads beside a reminder:
// #chips on rows, a Tags section in the sidebar that filters, smart lists with their counts,
// the early alert and the link — and no write control while Apple's side is off.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
const page = await ctx.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.locator(".rx-list", { hasText: "Daily" }).click();
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const row = (title) => page.locator(".rx-row", { has: page.getByLabel(`Rename ${title}`) });

// Chips on rows.
const chips = await row("Pick up the parcel").locator(".rx-meta .rx-tag").allInnerTexts();
ok(JSON.stringify(chips) === JSON.stringify(["#errands", "#home"]), `a row shows its tags as #chips  ${JSON.stringify(chips)}`);
ok((await row("Water the plants").locator(".rx-tag").count()) === 0, "a reminder without tags shows none");

// The sidebar's Tags.
const side = await page.locator(".rx-tags .rx-tag-chip").allInnerTexts();
ok(JSON.stringify(side) === JSON.stringify(["#errands", "#fitness", "#home"]), `the sidebar lists every tag  ${JSON.stringify(side)}`);
await page.locator(".rx-tag-chip", { hasText: "#home" }).click();
await page.waitForTimeout(300);
const heading = await page.locator(".rx-head h1").innerText();
const shown = await page.locator(".rx-row .rx-title").evaluateAll((els) => els.map((e) => e.value).sort());
ok(heading === "#home", `tapping a tag opens it  ${heading}`);
ok(JSON.stringify(shown) === JSON.stringify(["Pick up the parcel", "Plan tomorrow"]), `and shows only what carries it  ${JSON.stringify(shown)}`);
ok(await page.locator('.rx-tag-chip[aria-current="true"]', { hasText: "#home" }).count() === 1, "the open tag is marked in the sidebar");
ok(await page.locator(".rx-add").count() === 1, "a tag view has its add field now tag writes are on");
// A chip on a row opens its tag too.
await row("Pick up the parcel").locator(".rx-meta .rx-tag", { hasText: "#errands" }).click();
await page.waitForTimeout(300);
ok((await page.locator(".rx-head h1").innerText()) === "#errands", "a chip on a row opens its tag");

// Custom smart lists under My Lists, with their counts.
const smart = page.locator(".rx-smart-row", { hasText: "Important" });
ok(await smart.count() === 1, "a custom smart list sits under My Lists");
ok((await page.locator(".rx-smart-row").count()) === 1, "built-in smart lists are not listed twice");
ok((await smart.locator(".count").innerText()) === "1", `with how many it holds  ${await smart.locator(".count").innerText()}`);
await smart.click();
await page.waitForTimeout(300);
ok((await page.locator(".rx-head h1").innerText()) === "Important", "opening it shows its name");
const inSmart = await page.locator(".rx-row .rx-title").evaluateAll((els) => els.map((e) => e.value));
ok(JSON.stringify(inSmart) === JSON.stringify(["Pick up the parcel"]), `and what it holds  ${JSON.stringify(inSmart)}`);
ok(await page.locator(".rx-capped").count() === 0, "no note when every rule is read");

// The info panel: early alert and link, read only.
await page.locator(".rx-list", { hasText: "Daily" }).click();
await page.waitForTimeout(300);
await page.getByRole("button", { name: "Details for Pick up the parcel" }).click();
await page.waitForTimeout(300);
const panel = page.locator(".rx-detail");
const early = await panel.locator(".rx-field", { hasText: "Early Reminder" }).innerText().catch(() => "");
ok(early.includes("30 minutes before"), `the info shows the early alert  ${JSON.stringify(early)}`);
// Links can be changed (proven on the iPhone): the field holds it, Open opens it.
const field = panel.getByLabel("URL for Pick up the parcel");
ok((await field.inputValue()) === "https://www.apple.com/shop/order", "the link is in its field");
ok((await panel.locator("a.rx-url-open").getAttribute("href")) === "https://www.apple.com/shop/order", "and Open opens it");
const tagsField = await panel.locator(".rx-tag-field .rx-tag-name").allInnerTexts();
ok(JSON.stringify(tagsField) === JSON.stringify(["#errands", "#home"]), `and its tags  ${JSON.stringify(tagsField)}`);
ok(await panel.locator("input[type=url]").count() === 1, "the link can be changed now link writes are on");
// A reminder with none of these shows no empty rows for them.
await page.getByRole("button", { name: "Details for Plan tomorrow" }).click();
await page.waitForTimeout(300);
// Early alerts can be set (proven on the iPhone): the row is there, at None, as on the phone.
ok(/None/.test(await page.locator(".rx-detail .rx-field", { hasText: "Early Reminder" }).innerText().catch(() => "")), "an Early Reminder row at None where there is none");
ok(await page.locator(".rx-detail a.rx-url-open").count() === 0, "no Open where there is no link");
ok(await page.locator(".rx-detail").count() === 1, "one info panel open at a time");

// Rules not read here are said, once.
await page.evaluate(() => { window.__smartUnknown = ["location"]; });
await smart.click();
await page.waitForTimeout(300);
ok(await page.locator(".rx-capped").count() === 1, "a smart list with rules not read here says so");
ok(errors.length === 0, `no page errors  ${errors.join(" | ")}`);
await b.close();
process.exitCode = failures ? 1 : 0;
