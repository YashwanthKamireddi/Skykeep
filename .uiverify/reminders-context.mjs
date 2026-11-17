// Right-clicking a reminder opens what can be done to it, as on the Mac: info, flag,
// complete, indent, move to another list, delete — each doing what it says.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Reminders");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const row = page.locator(".rx-row").first();
await row.click({ button: "right", position: { x: 300, y: 10 } });
await page.waitForTimeout(150);
const items = await page.locator(".row-menu").first().getByRole("menuitem").allInnerTexts();
ok(["Show Info", "Flag", "Mark as Completed", "Indent", "Move to List", "Delete"].every((x) => items.some((i) => i.startsWith(x))), `a reminder's menu holds what can be done to it  ${JSON.stringify(items)}`);
await page.getByRole("menuitem", { name: "Show Info" }).click();
await page.waitForTimeout(200);
ok((await page.locator(".rx-detail").count()) === 1, "Show Info opens its details");
await row.click({ button: "right", position: { x: 300, y: 10 } });
await page.getByRole("menuitem", { name: "Move to List" }).hover();
await page.waitForTimeout(200);
ok((await page.locator(".row-menu").count()) === 2, "Move to List offers the other lists beside it");
await page.keyboard.press("Escape");
await b.close();
process.exitCode = failures ? 1 : 0;
