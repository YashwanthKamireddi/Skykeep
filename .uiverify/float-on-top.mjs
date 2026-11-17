// Float on Top (#40): the pin beside the sidebar button keeps an app's window above the others,
// Ctrl+Shift+P too; the choice is kept for the app and asked again when its window opens; where
// the desktop can't (Wayland) neither the pin nor its shortcut is offered.
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };
const URL = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
async function open(possible = true) {
  const ctx = await b.newContext({ viewport: { width: 1180, height: 800 } });
  await ctx.addInitScript((p) => { window.__floatPossible = p; }, possible);
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(`${URL}/?app=reminders`, { waitUntil: "networkidle" });
  await page.waitForTimeout(500);
  return { ctx, page };
}
const asks = (page) => page.evaluate(() => window.__floatAsks ?? []);
{
  const { ctx, page } = await open(true);
  const pin = page.getByRole("button", { name: "Float on Top" });
  ok(await pin.isVisible(), "the pin is beside the sidebar button");
  ok((await pin.getAttribute("aria-pressed")) === "false", "off until asked");
  const side = await page.locator(".side-toggle").boundingBox(), box = await pin.boundingBox();
  ok(side && box && box.x > side.x + side.width && Math.abs(box.y - side.y) < 2, "in the same row, to its right");
  await pin.click();
  await page.waitForTimeout(150);
  ok((await pin.getAttribute("aria-pressed")) === "true" && JSON.stringify(await asks(page)) === "[true]", "pressed, the window is asked to float");
  ok(await page.evaluate(() => localStorage.getItem("skykeep.float.reminders")) === "1", "and it is kept for Reminders");
  await page.keyboard.press("Control+Shift+P");
  await page.waitForTimeout(150);
  ok((await pin.getAttribute("aria-pressed")) === "false" && (await asks(page)).at(-1) === false, "Ctrl+Shift+P puts it back");
  await page.keyboard.press("Control+Shift+P");
  await page.waitForTimeout(150);
  // Opened again (a new window), it floats again at once.
  await page.reload({ waitUntil: "networkidle" });
  await page.waitForTimeout(500);
  ok(JSON.stringify(await asks(page)) === "[true]", "opened again, the window floats again by itself");
  await page.keyboard.press("Control+/");
  ok(await page.getByText("Float the window on top").isVisible(), "the shortcut is listed in Keyboard Shortcuts");
  await ctx.close();
}
{
  const { ctx, page } = await open(false);
  ok((await page.getByRole("button", { name: "Float on Top" }).count()) === 0, "Wayland: no pin");
  await page.keyboard.press("Control+Shift+P");
  await page.waitForTimeout(150);
  ok((await asks(page)).length === 0, "Wayland: the shortcut asks nothing");
  await page.keyboard.press("Control+/");
  await page.getByRole("dialog", { name: "Keyboard Shortcuts" }).waitFor();
  ok((await page.getByText("Float the window on top").count()) === 0, "Wayland: and isn't listed");
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
