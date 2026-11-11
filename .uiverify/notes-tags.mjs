// A #tag made on a phone sits in its line as its words, in the Notes accent — never a file
// card, never a gap — in the reader and the editor alike, and opening the note leaves it alone.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
for (const theme of ["light", "dark"]) {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 800 }, colorScheme: theme });
  await ctx.addInitScript(() => { window.__tagNote = true; });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
  const page = await ctx.newPage();
  await page.goto(process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await openScreen(page, "Notes");
  await page.waitForTimeout(400);
  await page.locator(".note-row", { hasText: "Pasta" }).first().click();
  const tag = page.locator(".nf-attachment.is-inline .nf-inline");
  await tag.first().waitFor({ timeout: 5000 }).catch(() => {});
  ok((await tag.count()) === 1 && (await tag.first().textContent()) === "#Recipes", `${theme}: the tag shows its words`);
  ok((await page.locator(".note-rich .nf-file").count()) === 0, `${theme}: and is not drawn as a file`);
  const look = await tag.first().evaluate((el) => {
    const host = el.parentElement.getBoundingClientRect(), line = el.closest("p, div")?.getBoundingClientRect();
    const cs = getComputedStyle(el), hs = getComputedStyle(el.parentElement);
    return { color: cs.color, bg: hs.backgroundColor, fontSize: cs.fontSize, lineSize: getComputedStyle(el.closest("p, div")).fontSize,
      inLine: line ? host.top >= line.top - 1 && host.bottom <= line.bottom + 1 : false };
  });
  ok(look.bg === "rgba(0, 0, 0, 0)" && look.fontSize === look.lineSize && look.inLine, `${theme}: set in its line at the text's size  ${JSON.stringify(look)}`);
  const text = await page.locator(".note-rich").first().innerText();
  ok(/Dinner for #Recipes tonight/.test(text.replace(/\s+/g, " ")), `${theme}: the line reads through the tag  ${JSON.stringify(text.slice(0, 60))}`);
  const wanted = theme === "light" ? "rgb(122, 86, 0)" : "rgb(242, 179, 61)";
  ok(look.color === wanted, `${theme}: in the Notes accent  ${look.color}`);
  await page.waitForTimeout(300);
  ok((await page.evaluate(() => (window.__noteSaves ?? []).length)) === 0, `${theme}: opening it saves nothing`);
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
