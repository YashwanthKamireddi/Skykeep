// Years, Months and All Photos, as Photos on a Mac steps through its Library: the switch in the
// toolbar (Library only), a year opening its months, a month opening All Photos at that month,
// Escape and Ctrl+[ stepping back out, the keys between tiles, the level kept, and a sync
// bringing a new month into Months as it lands.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
import { servePictures, status } from "./arrival.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
await servePictures(ctx);
// Three years, nine photos a month from November 2023 to October 2026 (noon, this machine's
// time, so every one is in the month it says); one favourite, in March 2025, and in April 2025
// a video in the middle of the month.
await ctx.addInitScript(() => {
  if (window.__photos) return;
  const P = (id, t, extra = {}) => ({ id, master: `m${id}`, takenAt: t, addedAt: t, kind: "image", width: 4032, height: 3024, duration: null,
    filename: `IMG_${id}.JPG`, size: 1, uti: "public.jpeg", favorite: false, hidden: false, deleted: false, deletedAt: null, subtype: 0, changeTag: null, ...extra });
  window.__mk = P;
  const out = [];
  for (let m = 0; m < 36; m++) {
    const d = new Date(2026, 9 - m, 1);
    for (let k = 0; k < 9; k++) {
      const id = `y${d.getFullYear()}m${d.getMonth() + 1}k${k}`;
      const extra = d.getFullYear() === 2025 && d.getMonth() === 2 && k === 7 ? { favorite: true }
        : d.getFullYear() === 2025 && d.getMonth() === 3 && k === 4 ? { kind: "video", duration: 4, uti: "public.mpeg-4" } : {};
      out.push(P(id, new Date(d.getFullYear(), d.getMonth(), 2 + k * 3, 12).getTime() / 1000, extra));
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
const pressed = async () => (await page.locator('.px-levels [aria-pressed="true"]').textContent())?.trim();
// What the grid shows at its top, under the floating bar: the month whose photos are there.
const gridTop = () => page.evaluate(() => {
  const el = document.querySelector(".px-scroll");
  const edge = el.getBoundingClientRect().top + parseFloat(getComputedStyle(el).scrollPaddingTop) + 4;
  const sec = [...el.querySelectorAll(".px-month")].find((s) => s.getBoundingClientRect().bottom > edge);
  return sec?.dataset.month ?? null;
});
// The row at the top of Years or Months, under the floating bar: its tiles' names.
const periodTop = () => page.evaluate(() => {
  const el = document.querySelector(".px-periods");
  const edge = el.getBoundingClientRect().top + parseFloat(getComputedStyle(el).scrollPaddingTop);
  const row = [...el.querySelectorAll(".px-period")].filter((t) => Math.abs(t.getBoundingClientRect().top - edge) < 8);
  return row.map((t) => t.querySelector("strong").textContent);
});

// The switch: the Library's, with All Photos to begin with.
ok(await page.locator(".px-levels").count() === 1, "the Library has the Years, Months and All Photos switch");
ok(await pressed() === "All Photos", `…on All Photos to begin with (${await pressed()})`);

// Years: a tile a year, newest first, each with its count; the grid's own controls go.
await seg("Years").click();
await page.locator('.px-periods[data-level="years"] .px-period').first().waitFor();
const years = await page.locator(".px-period strong").allTextContents();
ok(years.join(",") === "2026,2025,2024,2023", `Years shows a tile a year, newest first (${years.join(", ")})`);
const y2025 = await page.locator('.px-period[data-period="2025"]').getAttribute("aria-label");
ok(y2025 === "2025, 107 Photos, 1 Video", `…each saying what it holds (${y2025})`);
ok(await page.locator('.px-period[data-period="2025"] img').getAttribute("src").then((s) => s.includes("y2025m3k7")), "…2025 shown by its favourite");
ok(await page.locator(".px-scroll").evaluate((e) => getComputedStyle(e).display) === "none" && await page.locator(".px-tile").count() > 0, "the grid stays beneath, kept but out of the layout");
ok(await page.getByRole("button", { name: "Zoom In" }).count() === 0 && await page.getByRole("button", { name: "Show Whole Photos" }).count() === 0, "the grid's zoom and shape controls go while Years shows");
ok(await page.getByRole("button", { name: "Add Photos" }).count() === 1, "…Add Photos stays");

// A year opens Months at that year: its newest month at the top, focused.
await page.locator('.px-period[data-period="2025"]').click();
await page.locator('.px-periods[data-level="months"]').waitFor();
ok(await pressed() === "Months", "a year opens Months");
ok((await periodTop()).includes("December 2025"), `…its row at the top, with that year's newest month (${(await periodTop()).join(", ")})`);
ok(await page.evaluate(() => document.activeElement?.dataset.period) === "2025-12", "…which takes the keys");
const apr = page.locator('.px-period[data-period="2025-4"]');
ok(!(await apr.locator("img").first().getAttribute("src")).includes("y2025m4k4"), "a month is shown by a still, not the video in its middle");
ok(await apr.locator("img").count() === 4, `…with three more of its photos beside it (${await apr.locator("img").count()} pictures)`);
const sizes = await page.evaluate(() => [...document.querySelectorAll(".px-period img")].map((i) => Number(/fit(\d+)/.exec(i.getAttribute("src"))?.[1] ?? 0)));
ok(Math.max(...sizes) <= 512, `…its pictures thumbnails, never originals (largest ${Math.max(...sizes)}px)`);

// The arrows go between tiles as they are laid out.
const across = await page.evaluate(() => getComputedStyle(document.querySelector(".px-periods-grid")).gridTemplateColumns.split(" ").length);
await page.keyboard.press("ArrowRight");
ok(await page.evaluate(() => document.activeElement?.dataset.period) === "2025-11", "the right arrow goes to the next month");
await page.keyboard.press("ArrowDown");
const down = await page.evaluate(() => document.activeElement?.dataset.period);
ok(down === `2025-${11 - across}`, `…the down arrow a row down (${down}, ${across} across)`);
await page.keyboard.press("ArrowUp");
await page.keyboard.press("ArrowLeft");
ok(await page.evaluate(() => document.activeElement?.dataset.period) === "2025-12", "…and back");

// Enter opens the month: All Photos at December 2025.
await page.keyboard.press("Enter");
await page.waitForFunction(() => document.querySelector('.px-levels [aria-pressed="true"]')?.textContent === "All Photos");
await page.waitForTimeout(150);
ok(await gridTop() === "December 2025", `a month opens All Photos at that month (${await gridTop()})`);
ok(await page.locator(".px-scroll").evaluate((e) => getComputedStyle(e).display) !== "none" && await page.locator(".px-periods").count() === 0, "…the grid shown, the tiles gone");

// Escape steps out to Months, at the month the grid was at; Ctrl+[ steps out to Years.
await page.locator(".px-scroll").evaluate((e) => (e.scrollTop += 900));
await page.waitForTimeout(300);
// The month the title names, as the grid reads its top.
const there = await page.locator(".px-heading .px-span").textContent();
const scrolled = await page.locator(".px-scroll").evaluate((e) => e.scrollTop);
await page.evaluate(() => document.activeElement?.blur());
await page.keyboard.press("Escape");
await page.locator('.px-periods[data-level="months"]').waitFor();
ok((await periodTop()).includes(there) && await page.evaluate(() => document.activeElement?.querySelector("strong")?.textContent) === there,
  `Escape steps out to Months, at the month the grid was at (${there}; top row ${(await periodTop()).join(", ")})`);
// And back, to exactly where the grid was.
await seg("All Photos").click();
await page.waitForTimeout(100);
const back = await page.locator(".px-scroll").evaluate((e) => e.scrollTop);
ok(Math.abs(back - scrolled) < 2, `…All Photos again comes back exactly where it was (${back} for ${scrolled})`);
await page.keyboard.press("Control+BracketLeft");
await page.locator('.px-periods[data-level="months"]').waitFor();
await page.keyboard.press("Control+BracketLeft");
await page.locator('.px-periods[data-level="years"]').waitFor();
ok(await pressed() === "Years", "Ctrl+[ steps out to Months, then to Years");
const yearTop = await page.evaluate(() => document.activeElement?.dataset.period);
ok(yearTop === there.split(" ").pop(), `…at the year it was in (${yearTop})`);
await page.keyboard.press("Escape");
await page.waitForTimeout(100);
ok(await pressed() === "Years", "Escape in Years stays in Years");

// With a selection in All Photos, Escape lets the selection go first.
await seg("All Photos").click();
await page.locator(".px-tile").nth(2).click();
await page.keyboard.press("Escape");
await page.waitForTimeout(100);
ok(await pressed() === "All Photos" && await page.locator('.px-tile[aria-selected="true"]').count() === 0, "Escape with photos selected lets them go, and stays");

// The switch from the keys: Tab to it, Enter on Months.
await seg("Months").focus();
await page.keyboard.press("Enter");
await page.locator('.px-periods[data-level="months"]').waitFor();
ok(await pressed() === "Months", "the switch answers the keys");
// Ctrl+A in Months chooses nothing behind it.
await page.keyboard.press("Control+a");
ok(await page.locator('.px-tile[aria-selected="true"]').count() === 0, "Ctrl+A in Months selects nothing behind it");

// A sync bringing a new month: it shows in Months as it lands.
await page.evaluate(() => {
  const t = new Date(2026, 10, 3, 12).getTime() / 1000;
  window.__photos = [window.__mk("nov1", t), window.__mk("nov2", t + 60), ...window.__photos];
});
await page.evaluate((payload) => window.__emit("quince://synced", payload), status(["photos"]));
await page.locator('.px-period[data-period="2026-11"]').waitFor({ timeout: 5000 }).catch(() => {});
ok(await page.locator('.px-period[data-period="2026-11"]').getAttribute("aria-label").catch(() => null) === "November 2026, 2 Photos", "a month brought by a sync shows in Months as it lands");

// Only the Library has the levels: Favorites is its grid.
await page.locator(".px-side").getByRole("button", { name: /Favorites/ }).first().click();
await page.waitForFunction(() => document.querySelector(".px-heading h1")?.textContent?.includes("Favorites"));
ok(await page.locator(".px-levels").count() === 0 && await page.locator(".px-periods").count() === 0, "Favorites has no levels: its own grid");
await page.locator(".px-side").getByRole("button", { name: /Library/ }).first().click();
await page.locator('.px-periods[data-level="months"]').waitFor();
ok(await pressed() === "Months", "back in the Library, Months as it was left");

// Kept on this machine.
await page.reload({ waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.locator('.px-periods[data-level="months"]').waitFor({ timeout: 5000 }).catch(() => {});
ok(await pressed() === "Months", "the level is kept for next time");

await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
