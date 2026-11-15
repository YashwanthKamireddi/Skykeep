// Settings as System Settings: the account first (Home), then the sections, one at a time.
// Arrow keys walk the sidebar; a section opens at its top; at every size and in every language
// nothing runs out of its box sideways.
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openSettings, openPane } from "./stub.mjs";
const BASE = process.env.URL ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };
const open = async (w, h, lang) => {
  const ctx = await b.newContext({ viewport: { width: w, height: h } });
  if (lang) await ctx.addInitScript((l) => localStorage.setItem("quince.language", l), lang);
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(BASE, { waitUntil: "networkidle" });
  await openSettings(page);
  await page.locator(".prefs-side").waitFor();
  return { ctx, page };
};
// Every element whose content is wider than its box, and not cut on purpose with "…".
const spill = (page) => page.evaluate(() => [...document.querySelectorAll(".settings *")]
  .filter((e) => e.offsetParent && e.scrollWidth > e.clientWidth + 1 && getComputedStyle(e).textOverflow !== "ellipsis"
    && !["auto", "scroll"].includes(getComputedStyle(e).overflowX) && getComputedStyle(e).overflowX !== "hidden")
  .map((e) => `${e.className || e.tagName}: ${e.textContent.slice(0, 40)}`));
const wide = (page) => page.evaluate(() => document.querySelector(".content").scrollWidth <= document.querySelector(".content").clientWidth);

{
  const { ctx, page } = await open(1180, 820);
  ok((await page.locator('[data-pane="home"]').getAttribute("aria-selected")) === "true", "Settings opens on Home, the account");
  ok(await page.locator(".launch-tile").first().isVisible(), "Home holds the apps");
  await page.locator('[data-pane="home"]').focus();
  await page.keyboard.press("ArrowDown");
  ok((await page.locator('[data-pane="backup"]').getAttribute("aria-selected")) === "true"
    && await page.evaluate(() => document.activeElement?.dataset.pane === "backup"), "Down opens the next section and takes the focus there");
  await page.keyboard.press("End");
  ok((await page.locator('[data-pane="updates"]').getAttribute("aria-selected")) === "true", "End goes to the last section");
  await page.keyboard.press("Home");
  ok((await page.locator('[data-pane="home"]').getAttribute("aria-selected")) === "true", "Home goes back to the account");
  ok((await page.locator('[role="tab"][tabindex="0"]').count()) === 1, "one stop for Tab in the sidebar, the section open");
  await ctx.close();
}
for (const [w, h] of [[1180, 820], [800, 600], [560, 700]]) {
  for (const lang of [null, "de", "zh-CN"]) {
    const { ctx, page } = await open(w, h, lang);
    const panes = await page.locator('[role="tab"]').evaluateAll((els) => els.map((e) => e.dataset.pane));
    const bad = [];
    for (const p of panes) {
      await page.locator(`[data-pane="${p}"]`).click();
      await page.locator(".prefs-pane:not([hidden])").first().waitFor();
      await page.waitForTimeout(160);
      const s = await spill(page);
      if (s.length || !(await wide(page))) bad.push(`${p}: ${s.slice(0, 2).join(" | ") || "page wider than the window"}`);
    }
    ok(bad.length === 0, `${w}x${h} ${lang ?? "en"}: no section runs out sideways  ${JSON.stringify(bad)}`);
    await ctx.close();
  }
}
await b.close();
process.exitCode = failures ? 1 : 0;
