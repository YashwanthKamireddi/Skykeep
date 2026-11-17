// Calendars subscribed to by address: added from the sidebar's +, listed under Subscriptions,
// their events drawn read-only, refreshed as often as chosen, and let go with Unsubscribe.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1300, height: 850 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
const page = await ctx.newPage();
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Calendar");
await page.keyboard.press("Control+2");
await page.waitForTimeout(300);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const writes = () => page.evaluate(() => window.__subWrites ?? []);
const side = page.getByRole("complementary", { name: "Calendars" });

ok(await side.getByRole("button", { name: "Subscriptions" }).count() === 0, "no Subscriptions group before there is one");

// A wrong address is said in the panel, and nothing is kept.
await page.evaluate(() => { window.__feedFails = "There’s no calendar at that address any more."; });
await side.getByRole("button", { name: "Add Calendar" }).click();
await page.getByRole("menuitem", { name: /New Subscription/ }).click();
const form = page.getByRole("form", { name: "New Calendar Subscription" });
ok(await form.isVisible(), "New Subscription opens the address field");
await form.getByRole("textbox").fill("webcal://fixtures.example/missing.ics");
await form.getByRole("button", { name: "Subscribe" }).click();
await page.waitForTimeout(200);
ok(/no calendar at that address/.test(await form.getByRole("alert").innerText().catch(() => "")), "a refusal is said where the address was typed");

await page.evaluate(() => { window.__feedFails = null; });
await form.getByRole("textbox").fill("webcal://fixtures.example/pl.ics");
await page.keyboard.press("Enter");
await page.waitForTimeout(500);
ok(await form.count() === 0, "subscribed, the panel closes");
ok(await side.getByRole("button", { name: "Subscriptions" }).isVisible() && await side.getByText("Premier League").isVisible(),
  "it is listed under Subscriptions");
ok((await writes()).some((w) => w.subscribe === "webcal://fixtures.example/pl.ics"), "the address is sent as typed (the engine reads webcal as https)");
const match = page.locator(".cx-block", { hasText: "Arsenal v Chelsea" });
ok(await match.count() === 1, "its events are drawn");
await match.click();
await page.waitForTimeout(200);
const info = page.getByRole("dialog", { name: "Arsenal v Chelsea" });
ok(await info.isVisible() && await info.getByRole("button", { name: "Save" }).count() === 0, "read-only: shown, not edited");
ok((await info.getByRole("link", { name: /Open in Maps/ }).getAttribute("href"))?.startsWith("geo:0,0?q=Emirates"), "its place opens in Maps");
await page.keyboard.press("Escape");

// Hidden with its tick, as any calendar.
await side.getByText("Premier League").click();
await page.waitForTimeout(150);
ok(await match.count() === 0, "its tick hides it");
await side.getByText("Premier League").click();

// Refreshed every hour, on choice.
await side.getByText("Premier League").click({ button: "right" });
await page.getByRole("menuitem", { name: "Refresh", exact: true }).hover();
await page.getByRole("menuitemcheckbox", { name: "Every Hour" }).click();
await page.waitForTimeout(200);
ok((await writes()).some((w) => w.update?.refreshMinutes === 60), "Refresh ▸ Every Hour is kept");

// Renamed from its menu.
await side.getByText("Premier League").click({ button: "right" });
await page.getByRole("menuitem", { name: "Rename" }).click();
await page.keyboard.press("Control+a");
await page.keyboard.type("Football");
await page.keyboard.press("Enter");
await page.waitForTimeout(200);
ok(await side.getByText("Football").isVisible() && (await writes()).some((w) => w.update?.title === "Football"), "renamed");

// Unsubscribe asks first.
await side.getByText("Football").click({ button: "right" });
await page.getByRole("menuitem", { name: "Unsubscribe" }).click();
const ask = page.getByRole("alertdialog");
ok(/Unsubscribe from/.test(await ask.innerText()), "Unsubscribe asks first");
await ask.getByRole("button", { name: "Unsubscribe" }).click();
await page.waitForTimeout(400);
ok(await side.getByText("Football").count() === 0 && await page.locator(".cx-block", { hasText: "Arsenal" }).count() === 0
  && (await writes()).some((w) => w.unsubscribe === "s1"), "unsubscribed, it and its events are gone");
await b.close();
process.exitCode = failures ? 1 : 0;
