// The Keyboard Shortcuts sheet (Ctrl+/) never cuts a row off: in a small window, in an app's
// own window (its own keys and those that work everywhere) and in Skykeep's window (all six apps),
// every row can be scrolled to and lies wholly inside the sheet, none poured into a column off
// its right edge; and it is solid, so the window's text never shows through it.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen, openSettings } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const URL_ = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";

for (const [where, size] of [["Notes", [800, 680]], ["Skykeep", [800, 680]], ["Skykeep", [1280, 820]], ["Photos", [520, 480]]]) {
  const ctx = await b.newContext({ viewport: { width: size[0], height: size[1] } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `${where}: page error ${e.message}`));
  await page.goto(URL_, { waitUntil: "networkidle" });
  if (where === "Skykeep") await openSettings(page); else await openScreen(page, where);
  await page.waitForTimeout(300);
  await page.keyboard.press("Control+/");
  const sheet = page.getByRole("dialog", { name: "Keyboard Shortcuts" });
  await sheet.waitFor();
  const tag = `${where} at ${size[0]}x${size[1]}`;
  const groups = await sheet.locator("h3").allInnerTexts();
  ok(where === "Skykeep" ? groups.length === 7 : groups.length === 2 && groups[1] === "Everywhere", `${tag}: ${where === "Skykeep" ? "all six apps and Everywhere" : "its own keys and Everywhere"}  ${JSON.stringify(groups)}`);
  // Each row scrolled to, then measured against the scrolling box.
  const cut = await page.evaluate(() => {
    const box = document.querySelector(".shortcuts-groups");
    const bad = [];
    for (const row of box.querySelectorAll(".shortcuts-row")) {
      row.scrollIntoView({ block: "nearest" });
      const r = row.getBoundingClientRect();
      const c = box.getBoundingClientRect();
      if (r.left < c.left - 1 || r.right > c.right + 1 || r.top < c.top - 1 || r.bottom > c.bottom + 1) bad.push(row.querySelector("dt")?.textContent);
    }
    return { bad, sideways: box.scrollWidth > box.clientWidth + 1 };
  });
  ok(cut.bad.length === 0 && !cut.sideways, `${tag}: every row can be reached and lies wholly inside the sheet  ${JSON.stringify(cut)}`);
  const sheetBox = await sheet.boundingBox();
  ok(sheetBox && sheetBox.y >= 0 && sheetBox.y + sheetBox.height <= size[1], `${tag}: the sheet itself fits the window`);
  const alpha = await sheet.evaluate((el) => {
    const m = getComputedStyle(el).backgroundColor.match(/rgba?\(([^)]+)\)/);
    const parts = m ? m[1].split(",").map(Number) : [];
    return parts.length === 4 ? parts[3] : 1;
  });
  ok(alpha === 1, `${tag}: solid, nothing shows through  (alpha ${alpha})`);
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
