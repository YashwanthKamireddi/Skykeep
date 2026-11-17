// Go to Date, as in Calendar's View menu: Shift-Ctrl-T opens a date field, Return goes there,
// Escape closes it without moving.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Calendar");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const title = () => page.locator(".cx-title").innerText();
const before = await title();

await page.keyboard.press("Shift+Control+T");
const field = page.getByRole("dialog", { name: "Go to Date" }).getByLabel("Date");
ok((await field.count()) === 1, "Shift-Ctrl-T opens Go to Date");
await page.keyboard.press("Escape");
ok((await field.count()) === 0 && (await title()) === before, "Escape closes it without moving");

await page.keyboard.press("Shift+Control+T");
await field.fill("2031-02-14");
await page.keyboard.press("Enter");
await page.waitForTimeout(200);
const after = await title();
ok((await field.count()) === 0 && /February/.test(after) && /2031/.test(after), `Return goes to that day  ${before.replace(/\s+/g, " ")} → ${after.replace(/\s+/g, " ")}`);
await b.close();
process.exitCode = failures ? 1 : 0;
