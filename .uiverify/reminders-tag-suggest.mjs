// The tag field suggests the account's tags (#63): on focus the most used, as typing narrows
// them those starting with it first, never one the reminder has; arrows and Enter or a click
// take one; Escape closes the list and leaves the reminder open.
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
const offered = () => page.locator(".rx-tag-suggest [role=option]").allInnerTexts();
const added = () => page.evaluate(() => (window.__detailWrites ?? []).filter((w) => w.cmd === "reminder_tag_add").map((w) => w.name));

await page.getByRole("button", { name: "Details for Water the plants" }).click();
await page.waitForTimeout(300);
const input = page.getByLabel("Add a tag to Water the plants");
await input.focus();
await page.waitForTimeout(100);
const all = await offered();
ok(all.length === 3 && all.every((x) => ["#home", "#errands", "#fitness"].includes(x)), `focused, it offers the account's tags  ${JSON.stringify(all)}`);
ok(all[0] === "#home", `the most used first  ${JSON.stringify(all)}`);
ok((await input.getAttribute("aria-expanded")) === "true", "the field says its list is open");

await input.fill("e");
await page.waitForTimeout(100);
const e = await offered();
ok(e[0] === "#errands", `those starting with what is typed come first  ${JSON.stringify(e)}`);
ok(JSON.stringify([...e.slice(1)].sort()) === JSON.stringify(["#fitness", "#home"]), `then those holding it  ${JSON.stringify(e)}`);
await input.fill("#fi");
await page.waitForTimeout(100);
ok(JSON.stringify(await offered()) === JSON.stringify(["#fitness"]), `a typed # is ignored  ${JSON.stringify(await offered())}`);
await input.fill("zz");
await page.waitForTimeout(100);
ok((await page.locator(".rx-tag-suggest").count()) === 0, "nothing matching, no list");
ok((await input.getAttribute("aria-expanded")) === "false", "and the field says so");

// Arrows and Enter take the highlighted one.
await input.fill("h");
await page.waitForTimeout(100);
await input.press("ArrowDown");
ok((await page.locator(".rx-tag-suggest .is-active").innerText()) === "#home", "ArrowDown highlights the first");
ok(Boolean(await input.getAttribute("aria-activedescendant")), "and the field points at it");
await input.press("Enter");
await page.waitForTimeout(200);
ok(JSON.stringify(await added()) === JSON.stringify(["home"]), `Enter takes it  ${JSON.stringify(await added())}`);
ok((await input.inputValue()) === "", "and the field empties");

// Escape closes the list, not the reminder.
await input.focus();
await input.fill("");
await page.waitForTimeout(100);
ok((await page.locator(".rx-tag-suggest").count()) === 1, "the list is open again");
await input.press("Escape");
await page.waitForTimeout(150);
ok((await page.locator(".rx-tag-suggest").count()) === 0, "Escape closes the list");
ok((await page.locator(".rx-detail").count()) === 1, "and leaves the reminder's details open");

// A click takes one, and the field keeps its focus to type the next.
await input.fill("fit");
await page.waitForTimeout(100);
await page.locator(".rx-tag-suggest [role=option]", { hasText: "#fitness" }).click();
await page.waitForTimeout(200);
ok((await added()).at(-1) === "fitness", `a click takes it  ${JSON.stringify(await added())}`);
ok(await input.evaluate((el) => el === document.activeElement), "and the field keeps its focus");

// A tag the reminder already has is never offered.
await page.getByRole("button", { name: "Details for Pick up the parcel" }).click();
await page.waitForTimeout(300);
const second = page.getByLabel("Add a tag to Pick up the parcel");
await second.focus();
await page.waitForTimeout(100);
ok(JSON.stringify(await offered()) === JSON.stringify(["#fitness"]), `its own tags are left out  ${JSON.stringify(await offered())}`);

// Typing a new tag still works as before: Enter adds what is typed.
await second.fill("garden");
await page.waitForTimeout(100);
await second.press("Enter");
await page.waitForTimeout(200);
ok((await added()).at(-1) === "garden", `a new tag is added as typed  ${JSON.stringify(await added())}`);

ok(errors.length === 0, `no page errors  ${errors.join(" | ")}`);
await b.close();
process.exitCode = failures ? 1 : 0;
