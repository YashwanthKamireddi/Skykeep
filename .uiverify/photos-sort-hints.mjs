// Photos' sort menu says where the newest photos are under each order (#187): "Oldest First"
// read as "you see your oldest photos first", while both orders open on the newest. Each
// order carries a second line, "Newest at the bottom" / "Newest at the top", in both themes.
//
//   URL=http://localhost:4173 node .uiverify/photos-sort-hints.mjs   (ENGINE=webkit for WebKit; SHOTS=dir to keep pictures)
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const URL = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

for (const scheme of ["light", "dark"]) {
  const ctx = await b.newContext({ viewport: { width: 1100, height: 720 }, colorScheme: scheme });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(URL, { waitUntil: "networkidle" });
  await openScreen(page, "Photos");
  await page.getByRole("button", { name: "Sort" }).click();
  const menu = page.locator(".row-menu");
  await menu.waitFor();
  const oldest = menu.getByRole("menuitemcheckbox", { name: /Oldest First/ });
  const newest = menu.getByRole("menuitemcheckbox", { name: /Newest First/ });
  ok((await oldest.textContent()).includes("Newest at the bottom"), `${scheme}: Oldest First says the newest are at the bottom`);
  ok((await newest.textContent()).includes("Newest at the top"), `${scheme}: Newest First says the newest are at the top`);
  // The second line is quieter than the name, and inside the menu's width.
  const [name, detail] = await oldest.evaluate((el) => {
    const spans = el.querySelectorAll(".row-menu-two > span");
    return [...spans].map((s) => ({ size: parseFloat(getComputedStyle(s).fontSize), right: s.getBoundingClientRect().right }));
  });
  const box = await menu.boundingBox();
  ok(detail.size < name.size, `${scheme}: the second line is smaller (${detail.size} < ${name.size})`);
  ok(detail.right <= box.x + box.width, `${scheme}: the second line fits in the menu`);
  if (process.env.SHOTS) await page.screenshot({ path: `${process.env.SHOTS}/sort-${process.env.ENGINE ?? "chromium"}-${scheme}.png`, clip: { x: box.x - 260, y: box.y - 50, width: box.width + 280, height: box.height + 70 } });
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
