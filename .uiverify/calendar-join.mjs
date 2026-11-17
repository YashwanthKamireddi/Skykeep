// Join: an event with a video call's link shows a Join pill naming the service, opening the
// call's own link; an event with none — or a look-alike link — shows nothing.
//
//   BASE=http://localhost:4173 ENGINE=webkit node .uiverify/calendar-join.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };

for (const scheme of ["light", "dark"]) {
  const ctx = await b.newContext({ viewport: { width: 1300, height: 850 }, colorScheme: scheme });
  await ctx.addInitScript(installStub, {
    screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true,
    extraEvents: [
      { guid: "call", title: "Weekly sync", hour: 7, notes: "Agenda first.\nJoin: https://us02web.zoom.us/j/8123456789?pwd=abc." },
      { guid: "fake", title: "Phishy", hour: 8, notes: "https://zoom.us.evil.example/j/1" },
    ],
  });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(BASE, { waitUntil: "networkidle" });
  await openScreen(page, "Calendar");
  await page.keyboard.press("Control+1");
  await page.waitForTimeout(400);

  await page.locator(".cx-block", { hasText: "Weekly sync" }).first().click();
  const pop = page.getByRole("dialog", { name: "Weekly sync" });
  await pop.waitFor();
  const join = pop.locator(".cx-join");
  ok(await join.isVisible(), `${scheme}: an event with a Zoom link shows Join`);
  ok((await join.innerText()).trim() === "Join Zoom", `${scheme}: it names the service  "${(await join.innerText()).trim()}"`);
  ok((await join.getAttribute("href")) === "https://us02web.zoom.us/j/8123456789?pwd=abc", `${scheme}: it opens the call's own link, the full stop left off`);
  const contrast = await join.evaluate((el) => {
    const rgb = (c) => c.match(/\d+(\.\d+)?/g).slice(0, 3).map(Number).map((v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; });
    const lum = (c) => { const [r, g, b2] = rgb(c); return 0.2126 * r + 0.7152 * g + 0.0722 * b2; };
    const s = getComputedStyle(el); const a = lum(s.color), c = lum(s.backgroundColor);
    return (Math.max(a, c) + 0.05) / (Math.min(a, c) + 0.05);
  });
  ok(contrast >= 4.5, `${scheme}: its text reads at ${contrast.toFixed(2)}:1`);
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);

  await page.locator(".cx-block", { hasText: "Phishy" }).first().click();
  await page.getByRole("dialog", { name: "Phishy" }).waitFor();
  ok((await page.locator(".cx-join").count()) === 0, `${scheme}: a look-alike host makes no Join`);
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);

  await page.locator(".cx-block", { hasText: "Dentist" }).first().click();
  await page.getByRole("dialog", { name: "Dentist" }).waitFor();
  ok((await page.locator(".cx-join").count()) === 0, `${scheme}: an event with no call shows no Join`);
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
