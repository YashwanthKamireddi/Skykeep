// A month picked in Go to a Month glides there (photos/place). A wheel, a press or a key while
// it glides is the person taking over: the view stays where they took it, and is never thrown
// the rest of the way to the month first (18 Nov: taken over part way, it jumped to the month
// at once, and the wheel's own scroll went on from there).
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 }, reducedMotion: "no-preference" });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
await ctx.addInitScript(() => {
  const P = (i, t) => ({ id: `q${i}`, master: `mq${i}`, takenAt: t, addedAt: t, kind: "image", width: 4032, height: 3024,
    duration: null, filename: `IMG_${i}.JPG`, size: 2400000, uti: "public.jpeg", favorite: false, hidden: false,
    deleted: false, deletedAt: null, subtype: 0, changeTag: null });
  // Four photos a day for three years: a long way from the newest to the oldest.
  window.__photos = Array.from({ length: 1600 }, (_, i) => P(i, 1792600000 - i * 21600 * 2.6));
});
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(1500);
const at = () => page.evaluate(() => Math.round(document.querySelector(".px-scroll").scrollTop));
const start = await at();
ok(start > 3000, `opened at the foot, far from the oldest  (${start})`);

const jump = async () => {
  await page.locator(".px-scroll").evaluate((s) => { s.scrollTop = s.scrollHeight; });
  await page.waitForTimeout(400);
  await page.getByRole("button", { name: "Go to a Month" }).click();
  const dialog = page.getByRole("dialog", { name: "Go to a Month" });
  await dialog.getByRole("tab").first().click();
  await dialog.locator(".px-jump-month").first().click();
};
// Where the month is: a glide left alone ends there.
await jump();
await page.waitForTimeout(1200);
const month = await at();
ok(Math.abs(month - start) > 3000, `the month is far from the foot  (${start} → ${month})`);

// Each way of taking over: a wheel, a press on the grid, a key.
for (const how of ["wheel", "press", "key"]) {
  await jump();
  // Part way down the glide, the person takes over.
  await page.waitForTimeout(60);
  const mid = await at();
  const box = await page.locator(".px-scroll").boundingBox();
  if (how === "wheel") {
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.wheel(0, 40);
  } else if (how === "press") {
    await page.locator(".px-scroll").dispatchEvent("pointerdown", { bubbles: true });
  } else {
    await page.keyboard.press("Shift");
  }
  // The take-over has landed (WebKit hands a wheel over after the call returns): from here the
  // view must not move, and must not have been thrown on to the month.
  await page.waitForTimeout(300);
  const taken = await at();
  await page.waitForTimeout(600);
  const end = await at();
  ok(mid > 1500, `${how}: the glide was part way when taken over  (${start} → ${mid})`);
  ok(Math.abs(taken - month) > 200 && Math.abs(end - taken) < 40,
    `${how}: taken over, the view stops where it was, not thrown to the month at ${month}  (${mid} → ${taken} → ${end})`);
}
await b.close();
process.exitCode = failures ? 1 : 0;
