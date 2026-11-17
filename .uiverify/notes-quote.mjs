// Block Quote, as Notes has it: a quoted line shows the bar down its left; Command-' and the
// Aa panel's Block Quote put lines in a quote and take them out; Return on an empty quoted
// line ends the quote; and the save carries it to Apple on the lines that hold it.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
for (const scheme of ["light", "dark"]) {
  const ctx = await b.newContext({ viewport: { width: 1200, height: 800 }, colorScheme: scheme });
  await ctx.addInitScript(() => { window.__linkNote = true; });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
  await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await openScreen(page, "Notes");
  await page.locator(".note-row", { hasText: "Links" }).first().click();
  await page.waitForTimeout(500);
  const line = (t) => page.locator(".note-rich .nf-p", { hasText: t }).first();
  // A line's own start, on its first row: its first words, never the line above nor a link in it.
  const atStart = async (t) => {
    const box = await line(t).boundingBox();
    const pad = await line(t).evaluate((e) => parseFloat(getComputedStyle(e).paddingInlineStart) || 0);
    await page.mouse.click(box.x + pad + 2, box.y + Math.min(10, box.height / 2));
  };
  const bar = (t) => line(t).evaluate((e) => parseFloat(getComputedStyle(e).borderInlineStartWidth));
  ok((await bar("Read")) === 3 && (await bar("Yashps")) === 0, `${scheme}: a quoted line shows its bar, others none`);
  const lastSave = async () => { await page.waitForTimeout(1200); return page.evaluate(() => window.__noteSaves?.at(-1)); };
  const runAt = (save, text) => {
    const at = save.body.indexOf(text);
    let n = 0;
    for (const r of save.runs ?? []) { if (at < n + r.length) return r; n += r.length; }
    return null;
  };
  // Command-' on the address line quotes it; the save says so on that line.
  await atStart("Yashps");
  await page.waitForTimeout(80);
  await page.keyboard.press("Control+'");
  ok((await bar("Yashps")) === 3, `${scheme}: Command-' quotes the line`);
  let save = await lastSave();
  ok(save && runAt(save, "Yashps")?.quote === true && runAt(save, "Links")?.quote !== true, `${scheme}: saved quoted on that line only`);
  // The Aa panel shows it ticked, and takes it off.
  await page.getByRole("button", { name: /^(Aa|Format)/ }).first().click().catch(() => page.locator(".notes-aa").click());
  await page.waitForTimeout(150);
  const item = page.locator(".format-panel").getByRole("button", { name: "Block Quote" });
  ok((await item.getAttribute("aria-pressed")) === "true", `${scheme}: Aa shows Block Quote on for a quoted line`);
  await item.click();
  await page.waitForTimeout(150);
  ok((await bar("Yashps")) === 0, `${scheme}: and takes it off`);
  await page.keyboard.press("Escape");
  // Return on an empty quoted line ends the quote.
  // At the line's start: its middle is a link, and a click there opens it.
  await atStart("Read");
  await page.waitForTimeout(80);
  await page.keyboard.press("End");
  // The editor learns where the cursor went from the selection's own event: a key pressed in
  // the same instant would act on where it was (Chromium), which no hand can do.
  await page.waitForTimeout(80);
  await page.keyboard.press("Enter");
  ok(await page.evaluate(() => [...document.querySelectorAll(".note-rich .nf-p")].at(-1).dataset.quote === "true"), `${scheme}: a new line in a quote is quoted`);
  await page.keyboard.press("Enter");
  ok(await page.evaluate(() => [...document.querySelectorAll(".note-rich .nf-p")].at(-1).dataset.quote !== "true"), `${scheme}: Return on an empty quoted line ends the quote`);
  await page.screenshot({ path: `${process.env.TMPDIR ?? "/tmp"}/notes-quote-${scheme}-${process.env.ENGINE}.png` });
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
