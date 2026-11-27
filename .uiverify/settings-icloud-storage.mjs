// Settings › Home: iCloud storage as Apple's own bar shows it. One part per kind of what fills it
// (largest first, Apple's colours), their names and sizes under it, and full or nearly full said
// in words. Without Apple's split, the one bar as before. Light and dark, roomy and 800×600.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openSettings } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const shots = process.env.SHOTS;
const GB = 1_000_000_000;
const split = {
  usedBytes: 47.6 * GB, totalBytes: 50 * GB, full: false, almostFull: true,
  kinds: [
    { key: "photos", label: "Photos", color: "#FFC800", bytes: 38 * GB },
    { key: "backup", label: "Backups", color: "#7B61FF", bytes: 6 * GB },
    { key: "docs", label: "Docs", color: "#FF8A00", bytes: 3 * GB },
    { key: "newkind", label: "Apple's own name", color: null, bytes: 0.6 * GB },
  ],
};
for (const [scheme, width, height] of [["light", 1180, 820], ["dark", 1180, 820], ["light", 800, 600]]) {
  for (const answer of [split, null, { ...split, kinds: [], full: true, almostFull: true }]) {
    const label = `${scheme} ${width}px ${answer === null ? "no split" : answer.full ? "full" : "split"}`;
    const ctx = await b.newContext({ viewport: { width, height }, colorScheme: scheme });
    await ctx.addInitScript(() => { window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} }; });
    if (answer) await ctx.addInitScript((s) => { window.__storage = s; }, answer);
    await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
    const page = await ctx.newPage();
    page.on("pageerror", (e) => ok(false, `${label}: page error: ${e.message}`));
    await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
    await openSettings(page);
    await page.waitForSelector(".prefs-storage");
    const box = page.locator(".prefs-storage");
    const bar = box.getByRole("meter", { name: "iCloud storage used" });
    const parts = bar.locator(".storage-part");
    if (answer === split) {
      ok((await parts.count()) === 4, `${label}: one part per kind`);
      const widths = await parts.evaluateAll((ps) => ps.map((p) => p.getBoundingClientRect().width));
      const barWidth = await bar.evaluate((e) => e.getBoundingClientRect().width);
      ok(Math.abs(widths[0] / barWidth - 0.76) < 0.02, `${label}: Photos takes its share of the bar (${(widths[0] / barWidth).toFixed(3)})`);
      ok(widths.every((w, i) => i === 0 || w <= widths[i - 1]), `${label}: largest first`);
      const bg = await parts.first().evaluate((e) => getComputedStyle(e).backgroundColor);
      ok(bg === "rgb(255, 200, 0)", `${label}: in Apple's colour (${bg})`);
      const legend = await box.locator(".storage-legend li").allInnerTexts();
      ok(legend.length === 4 && legend[0].startsWith("Photos") && legend[1].startsWith("Backups") && legend[2].startsWith("Documents"), `${label}: named in the app's words (${legend.map((l) => l.split("\n")[0]).join(", ")})`);
      ok(legend[3].startsWith("Apple's own name"), `${label}: a kind the app doesn't know keeps Apple's name`);
      ok(legend[0].includes("38 GB"), `${label}: with its size`);
      ok((await box.innerText()).includes("iCloud storage is almost full."), `${label}: almost full is said`);
      const overflow = await box.evaluate((e) => e.scrollWidth > e.clientWidth + 1);
      ok(!overflow, `${label}: nothing runs out of the card`);
      if (shots) await page.locator(".prefs-hero").screenshot({ path: `${shots}/storage-${scheme}-${width}.png` });
    } else {
      ok((await parts.count()) === 0 && (await bar.locator("span").count()) === 1, `${label}: one bar for all of it`);
      ok((await box.locator(".storage-legend").count()) === 0, `${label}: no legend without Apple's split`);
      const text = await box.innerText();
      if (answer?.full) ok(text.includes("iCloud storage is full.") && !text.includes("almost full"), `${label}: full is said, once`);
      else ok(!text.includes("full"), `${label}: nothing said while there is room`);
    }
    await ctx.close();
  }
}
// Apple refusing the question (a lapsed sign-in): said, never "Reading storage…" for ever.
{
  const ctx = await b.newContext({ viewport: { width: 1180, height: 820 } });
  await ctx.addInitScript(() => { window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} }; window.__storageFails = true; });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
  const page = await ctx.newPage();
  await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await openSettings(page);
  await page.waitForSelector(".prefs-storage");
  await page.waitForTimeout(300);
  const text = await page.locator(".prefs-storage").innerText();
  ok(text.includes("iCloud storage couldn't be read.") && !text.includes("Reading storage"), "a refused storage question is said, not left reading");
  await ctx.close();
}
await b.close();
if (failures) { console.log(`${failures} failed`); process.exit(1); }
console.log("all passed");
