// Menus take the keys a Mac menu takes: arrows, Home and End, type-select (letters go to the
// line that begins with them; the same letter again steps on), Return chooses, Escape leaves.
// The pointer leaving a menu takes its highlight with it. A pop-up's menu type-selects too.
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const all = { notesFixture: true, calendarFixture: true, contactsFixture: true, driveFixture: true, remindersFixture: true };
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, ...all });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };
const lit = () => page.locator(".row-menu .row-menu-item.is-active .row-menu-label").first().innerText().catch(() => null);

await openScreen(page, "Contacts");
await page.waitForTimeout(300);
const row = page.locator(".contact-row").first();
await row.click({ button: "right" });
await page.waitForTimeout(80);
const labels = await page.locator(".row-menu .row-menu-label").allInnerTexts();
ok(labels.length >= 4, `the menu opens  (${labels.join(" | ")})`);
ok((await lit()) === null, "with nothing lit, as a Mac menu opens");

await page.keyboard.press("e");
const firstE = labels.find((l) => l.toLowerCase().startsWith("e"));
ok((await lit()) === firstE, `a letter lights the first line beginning with it  (${await lit()})`);
await page.keyboard.press("e");
const es = labels.filter((l) => l.toLowerCase().startsWith("e"));
ok((await lit()) === (es[1] ?? es[0]), `the same letter again steps to the next  (${await lit()})`);
await page.waitForTimeout(1100);
await page.keyboard.type("del");
ok((await lit()) === labels.find((l) => l.toLowerCase().startsWith("del")), `typed in a run, letters spell a prefix  (${await lit()})`);
await page.keyboard.press("Home");
ok((await lit()) === labels[0], "Home goes to the first line");
await page.keyboard.press("End");
ok((await lit()) === labels[labels.length - 1], "End to the last");
await page.keyboard.press("ArrowDown");
ok((await lit()) === labels[0], "Down from the last wraps to the first");
await page.keyboard.press("q");
ok((await page.locator(".row-menu").count()) === 1 && (await lit()) === labels[0], "a letter nothing begins with changes nothing");

// The pointer: over a line lights it; off the menu, nothing is lit.
const edit = page.getByRole("menuitem", { name: "Edit Contact" });
await edit.hover();
ok((await lit()) === "Edit Contact", "the pointer lights the line under it");
const box = await page.locator(".row-menu").boundingBox();
await page.mouse.move(box.x + box.width + 120, box.y + box.height / 2);
ok((await lit()) === null, "leaving the menu takes the highlight with it");
await page.keyboard.press("Escape");
ok((await page.locator(".row-menu").count()) === 0, "Escape closes it");

// Return chooses the lit line: Edit Contact opens the editor.
await row.click({ button: "right" });
await page.waitForTimeout(80);
await page.keyboard.type("ed");
await page.keyboard.press("Enter");
await page.waitForTimeout(150);
ok((await page.locator(".row-menu").count()) === 0, "Return chooses, and the menu goes");

// A pop-up's menu: typing lights the option that begins with it, Return takes it.
// In a narrow window, where Contacts shows its lists as a pop-up.
await page.keyboard.press("Escape");
await page.setViewportSize({ width: 760, height: 600 });
await page.waitForTimeout(200);
const pop = page.locator(".contacts .picker-button:visible").first();
if ((await pop.count()) === 0) ok(false, "a pop-up button on Contacts to try");
else {
  await pop.click();
  await page.waitForSelector(".picker-menu");
  const opts = await page.locator(".picker-menu .picker-label").allInnerTexts();
  const target = opts.find((o, i) => i > 0 && !opts.slice(0, i).some((p) => p[0].toLowerCase() === o[0].toLowerCase()));
  if (!target) ok(true, `pop-up type-select skipped: no option with its own first letter  (${opts.join(" | ")})`);
  else {
    await page.keyboard.press(target[0].toLowerCase());
    const now = await page.locator(".picker-menu li.is-active .picker-label").innerText().catch(() => null);
    ok(now === target, `a pop-up's menu type-selects  (${now} for ${target})`);
    await page.keyboard.press("Enter");
    await page.waitForTimeout(100);
    ok((await page.locator(".picker-menu").count()) === 0 && (await pop.innerText()).includes(target), "and Return takes it");
  }
}
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
