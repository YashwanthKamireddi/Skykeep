// Years and Months are by when photos were taken, whatever All Photos is sorted by. Sorted by
// Date Added — an old camera roll brought in last month — a month picked in Months opens All
// Photos at that month's own photo, not at the top, and stepping back out lands on the month
// the photo at the top was taken in, not the month it was added.
//
//   URL=http://localhost:4173 ENGINE=webkit node .uiverify/photos-levels-added.mjs
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
import { servePictures } from "./arrival.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
await servePictures(ctx);
// Three years, nine photos a month from November 2023 to October 2026; everything taken before
// 2025 was added on 20 October 2026, in one go.
await ctx.addInitScript(() => {
  if (window.__photos) return;
  const P = (id, t, added) => ({ id, master: `m${id}`, takenAt: t, addedAt: added, kind: "image", width: 4032, height: 3024, duration: null,
    filename: `IMG_${id}.JPG`, size: 1, uti: "public.jpeg", favorite: false, hidden: false, deleted: false, deletedAt: null, subtype: 0, changeTag: null });
  const batch = new Date(2026, 9, 20, 12).getTime() / 1000;
  const out = [];
  for (let m = 0; m < 36; m++) {
    const d = new Date(2026, 9 - m, 1);
    for (let k = 0; k < 9; k++) {
      const t = new Date(d.getFullYear(), d.getMonth(), 2 + k * 3, 12).getTime() / 1000;
      out.push(P(`y${d.getFullYear()}m${d.getMonth() + 1}k${k}`, t, d.getFullYear() < 2025 ? batch - out.length : t));
    }
  }
  window.__photos = out.sort((a, b) => b.takenAt - a.takenAt);
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
await openScreen(page, "Photos");
await page.locator(".px-tile").first().waitFor();
const seg = (name) => page.locator(".px-levels").getByRole("button", { name, exact: true });
// The tiles in the grid's first row under the floating bar.
const gridTopIds = () => page.evaluate(() => {
  const el = document.querySelector(".px-scroll");
  const edge = el.getBoundingClientRect().top + parseFloat(getComputedStyle(el).scrollPaddingTop);
  const tiles = [...el.querySelectorAll(".px-tile")];
  const first = tiles.find((t) => t.getBoundingClientRect().bottom > edge + 2);
  if (!first) return [];
  const top = first.getBoundingClientRect().top;
  return tiles.filter((t) => Math.abs(t.getBoundingClientRect().top - top) < 2).map((t) => t.dataset.id);
});

// Sorted by Date Added: the batch is one month of the grid, October 2026.
await page.getByRole("button", { name: "Sort" }).click();
await page.locator(".row-menu").getByText("Date Added").click();
await page.waitForTimeout(300);
const titles = await page.locator(".px-month-title").allInnerTexts();
ok(titles.length > 0 && !titles.some((t) => /2024|2023/.test(t)), `All Photos by Date Added groups by the month photos came in  ${JSON.stringify(titles.slice(0, 4))}`);

// Months still by date taken.
await seg("Years").click();
await page.locator('.px-period[data-period="2024"]').click();
await page.locator('.px-periods[data-level="months"] .px-period').first().waitFor();
await page.waitForTimeout(300);
ok(await page.locator('.px-period[data-period="2024-3"]').count() === 1, "Months still has every month photos were taken in");

// March 2024 picked: All Photos opens on its photo, inside the batch.
const key = await page.evaluate(async () => (await window.__TAURI_INTERNALS__.invoke("photos_months")).find((m) => m.year === 2024 && m.month === 3).key);
await page.locator('.px-period[data-period="2024-3"]').click();
await page.waitForFunction(() => document.querySelector('.px-levels [aria-pressed="true"]')?.textContent === "All Photos");
await page.waitForTimeout(400);
const top = await gridTopIds();
ok(top.includes(key), `a month opens All Photos at that month's photo  (${key} in top row ${top.join(",")})`);

// And back out: Months at the month the top photo was taken in, not October 2026.
await page.evaluate(() => document.activeElement?.blur());
await page.keyboard.press("Escape");
await page.locator('.px-periods[data-level="months"]').waitFor();
await page.waitForTimeout(300);
// The month it lands on takes the keys (March 2024 is too near the end to scroll to the top).
const at = await page.evaluate(() => document.activeElement?.dataset.period);
const want = /y(\d+)m(\d+)/.exec(top[0] ?? "");
ok(want && at === `${want[1]}-${want[2]}`, `stepping out lands on the month it was taken  (${want?.slice(1).join("-")}; at ${at})`);

// Date Captured again: the months are the months taken, and the same jump lands on the month.
await seg("All Photos").click();
await page.getByRole("button", { name: "Sort" }).click();
await page.locator(".row-menu").getByText("Date Captured").click();
await page.waitForTimeout(300);
await seg("Years").click();
await page.locator('.px-period[data-period="2024"]').click();
await page.locator('.px-period[data-period="2024-3"]').click();
await page.waitForFunction(() => document.querySelector('.px-levels [aria-pressed="true"]')?.textContent === "All Photos");
await page.waitForTimeout(400);
const heading = await page.locator(".px-heading .px-span").textContent();
ok(/March 2024/.test(heading ?? ""), `by Date Captured a month opens at its own month  (${heading})`);
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
