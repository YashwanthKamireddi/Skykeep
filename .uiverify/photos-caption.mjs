// Title and caption, as Photos' Info has them (Apple's captionEnc and extendedDescEnc): typed in
// place, both saved together when the words are left — going from one to the other is not
// leaving — kept when the photo opens again; the letters typed are words, not the photo's keys;
// Escape puts back what was there and leaves the photo open.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const sent = () => page.evaluate(() => window.__texts ?? []);

await page.locator(".px-tile").nth(1).dblclick();
await page.waitForTimeout(600);
await page.keyboard.press("i");
ok((await page.getByLabel("Title").getAttribute("placeholder")) === "Add a Title" &&
  (await page.getByLabel("Caption").getAttribute("placeholder")) === "Add a Caption", "Info offers Add a Title and Add a Caption");
await page.getByLabel("Title").click();
await page.keyboard.type("Ferry");
ok((await page.evaluate(() => (window.__flags ?? []).length)) === 0 && (await page.locator(".px-viewer").count()) === 1,
  "the letters typed are words, not the photo's keys");
await page.getByLabel("Caption").click();
ok((await sent()).length === 0, "going from the title to the caption saves nothing yet");
await page.keyboard.type("Half five, fish and hills");
await page.keyboard.press("Enter");
await page.waitForTimeout(200);
ok(JSON.stringify((await sent()).at(-1)) === JSON.stringify({ id: "p2", title: "Ferry", caption: "Half five, fish and hills" }) && (await sent()).length === 1,
  "leaving them saves both, once");
await page.keyboard.press("Escape");
await page.waitForTimeout(650);
await page.locator(".px-tile").nth(1).dblclick();
await page.waitForTimeout(600);
ok((await page.locator(".px-info").count()) === 0, "the next photo opens without Info");
await page.keyboard.press("i");
ok((await page.getByLabel("Title").inputValue()) === "Ferry" && (await page.getByLabel("Caption").inputValue()) === "Half five, fish and hills",
  "and both are there when the photo opens again");
await page.getByLabel("Title").click();
await page.keyboard.type(" — not this");
await page.keyboard.press("Escape");
await page.waitForTimeout(200);
ok((await page.getByLabel("Title").inputValue()) === "Ferry" && (await page.locator(".px-viewer").count()) === 1,
  "Escape puts it back and keeps the photo open");
ok((await sent()).length === 1, "and sends nothing");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
