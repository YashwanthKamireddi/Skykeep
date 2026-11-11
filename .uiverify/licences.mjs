// Settings → Open-Source Licences: the sheet lists every licence in src/licences.json (crates and
// npm packages, from `npm run licences`), what is shipped under each, and unfolds a text on a
// click, selectable; Escape and Done close it.
import AxeBuilder from "@axe-core/playwright";
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openSettings } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
for (const scheme of ["light", "dark"]) {
  const ctx = await b.newContext({ viewport: { width: 1200, height: 900 }, colorScheme: scheme });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
  await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
  await openSettings(page);
  await page.getByRole("button", { name: "Open-Source Licences" }).click();
  const sheet = page.getByRole("dialog", { name: "Open-Source Licences" });
  await sheet.waitFor();
  const headings = await sheet.locator("h3").allInnerTexts();
  ok(headings.includes("MIT License") && headings.includes("ISC License") && headings.includes("Apache License 2.0"), `${scheme}: licences listed (${headings.length})`);
  const rows = sheet.locator(".licences-row");
  const all = await rows.allInnerTexts();
  ok(all.length > 100, `${scheme}: ${all.length} licence texts`);
  ok(all.some((t) => /lucide-react \d/.test(t)) && all.some((t) => /\breact \d/.test(t)) && all.some((t) => /\btokio \d/.test(t)), `${scheme}: crates and npm packages both named`);
  ok(!all.some((t) => /\bquince-/.test(t)), `${scheme}: Skykeep's own crates are not listed as third-party`);
  const repeated = await sheet.locator("section").evaluateAll((sections) =>
    sections.flatMap((s) => {
      const labels = [...s.querySelectorAll(".licences-row")].map((r) => r.textContent);
      return labels.filter((t, i) => labels.indexOf(t) !== i);
    }),
  );
  ok(repeated.length === 0, `${scheme}: no row repeats another under the same licence (${repeated.slice(0, 3).join("; ")})`);
  const title = sheet.getByRole("heading", { name: "Open-Source Licences" });
  ok((await title.boundingBox())?.height > 10, `${scheme}: the title keeps its height`);
  // Lucide's ISC text, unfolded on a click, folded again on the next row's.
  const lucide = rows.filter({ hasText: "lucide-react" });
  await lucide.click();
  const text = sheet.locator(".licences-text");
  ok((await text.count()) === 1 && /ISC License/.test(await text.innerText()) && /Lucide/i.test(await text.innerText()), `${scheme}: Lucide's licence text shown`);
  ok((await lucide.getAttribute("aria-expanded")) === "true", `${scheme}: row says it is open`);
  ok((await text.evaluate((e) => getComputedStyle(e).userSelect)) === "text", `${scheme}: text selectable`);
  const [fg, bg] = await text.evaluate((e) => [getComputedStyle(e).color, getComputedStyle(e.closest(".modal-sheet")).backgroundColor]);
  ok(fg !== bg, `${scheme}: readable (${fg} on ${bg})`);
  // WCAG 2.2 AA over the open sheet, as a11y.mjs checks every screen.
  const { violations } = await new AxeBuilder({ page })
    .include(".licences-sheet")
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
    .analyze();
  const bad = violations.filter((v) => v.impact === "serious" || v.impact === "critical");
  ok(bad.length === 0, `${scheme}: no serious WCAG 2.2 AA problems${bad.length ? ` (${bad.map((v) => v.id).join(", ")})` : ""}`);
  await page.screenshot({ path: `${process.env.TMPDIR ?? "/tmp"}/licences-${scheme}-${process.env.ENGINE ?? "chromium"}.png` });
  await rows.filter({ hasText: "tokio" }).first().click();
  ok((await text.count()) === 1 && (await lucide.getAttribute("aria-expanded")) === "false", `${scheme}: one text open at a time`);
  await page.keyboard.press("Escape");
  ok((await sheet.count()) === 0, `${scheme}: Escape closes`);
  await page.getByRole("button", { name: "Open-Source Licences" }).click();
  await sheet.getByRole("button", { name: "Done" }).click();
  ok((await sheet.count()) === 0, `${scheme}: Done closes`);
  await ctx.close();
}
await b.close();
if (failures) process.exitCode = 1;
