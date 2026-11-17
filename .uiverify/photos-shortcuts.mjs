// The viewer's and the library's keys, as Photos on a Mac takes them: ← and → go through the
// photos, F favorites, I shows Info (which moves the photo aside rather than covering it), Space
// plays a Live Photo's moving part and takes a still back to the library, Ctrl+Backspace deletes
// after asking, Escape closes — and closing leaves the photo last shown chosen and in view.
// In the library: Ctrl + and − zoom, keeping the photo looked at in place, grouped by day from
// five across; and a rectangle swept from the space around the photos selects what it touches.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
await ctx.addInitScript(() => {
  // A moving part that never arrives: the Live Photo stays playing for as long as it is looked at.
  window.__videoBase = "http://video.test/hang";
  const P = (i, t, x = {}) => ({ id: `k${i}`, master: `mk${i}`, takenAt: t, addedAt: t, kind: "image", width: 4032, height: 3024, duration: null,
    filename: `IMG_${i}.JPG`, size: 2400000, uti: "public.jpeg", favorite: false, hidden: false, deleted: false, deletedAt: null, subtype: 0, changeTag: null, ...x });
  // Three photos a day, back over four months.
  window.__photos = Array.from({ length: 360 }, (_, i) => P(i, 1792600000 - Math.floor(i / 3) * 86400 - (i % 3) * 3600, i === 1 ? { kind: "live" } : {}));
});
await ctx.route("http://video.test/**", () => {}); // never answered
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
// Every question answered yes, and remembered.
await page.addInitScript(() => addEventListener("DOMContentLoaded", () =>
  new MutationObserver(() => {
    const q = document.querySelector(".confirm:not([data-answered])");
    if (!q) return;
    q.setAttribute("data-answered", "");
    (window.__asked ??= []).push(q.querySelector("h2")?.textContent ?? "");
    q.querySelector(".confirm-answer")?.click();
  }).observe(document.body, { childList: true, subtree: true })));
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.locator(".px-tile").first().waitFor();
let failures = 0;
/** Until `fn` (run in the page, given `arg`) holds, or 8 s: the checks after it then say what is there. */
const until = async (fn, arg) => {
  for (const end = Date.now() + 8000; Date.now() < end; ) if (await page.evaluate(fn, arg)) return true;
  return false;
};
await until(() => document.querySelector('.px-tile[data-index="0"] img')?.complete);
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const shown = () => page.evaluate(() => document.querySelector(".px-viewer")?.getAttribute("aria-label") ?? null);
const box = (s) => page.locator(s).first().evaluate((e) => { const r = e.getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height, r: r.right, b: r.bottom }; });

// ---- the viewer ----
await page.locator('.px-tile[data-index="0"]').dblclick();
await until(() => document.querySelector(".px-viewer")?.getAttribute("aria-label") === "IMG_0.JPG");
ok((await shown()) === "IMG_0.JPG", "a double-click opens the photo");
ok((await page.evaluate(() => document.getAnimations().filter((a) => a.playState === "running").length)) === 0,
  "and it is open at once — nothing animates");
await page.keyboard.press("ArrowRight");
ok((await shown()) === "IMG_1.JPG", "→ goes to the next photo");

// Space on a Live Photo plays its moving part.
await page.keyboard.press(" ");
await until(() => document.querySelectorAll(".px-live").length === 1);
ok((await page.locator(".px-live").count()) === 1 && (await shown()) === "IMG_1.JPG", "Space plays a Live Photo's moving part");
await page.keyboard.press(" ");
await until(() => document.querySelectorAll(".px-live").length === 0);
ok((await page.locator(".px-live").count()) === 0, "and Space again stops it");

await page.keyboard.press("ArrowRight");
await page.keyboard.press("ArrowLeft");
await page.keyboard.press("ArrowRight");
ok((await shown()) === "IMG_2.JPG", "← and → step either way");

await page.keyboard.press("f");
await until(() => window.__flags?.at(-1)?.flag === "favorite" && !!document.querySelector('.px-viewer-bar [aria-label="Unfavorite"]'));
ok((await page.evaluate(() => window.__flags?.at(-1))).flag === "favorite" &&
  (await page.locator('.px-viewer-bar [aria-label="Unfavorite"]').count()) === 1, "F favorites it, and the heart fills at once");

// I: Info beside the photo, never over it, and never over the next arrow.
await page.keyboard.press("i");
// Info open, and the photo moved aside for it (its box settled over two frames).
await until(async () => {
  const info = document.querySelector(".px-info"), pic = document.querySelector(".px-shown");
  if (!info || !pic) return false;
  const a = pic.getBoundingClientRect().right;
  await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  return pic.getBoundingClientRect().right === a;
});
ok((await page.locator(".px-info").count()) === 1, "I shows Info");
const info = await box(".px-info"), pic = await box(".px-shown"), next = await box(".px-nav.is-next");
ok(pic.r <= info.x && next.r <= info.x, `the photo and its next arrow make room for Info  (photo ends ${Math.round(pic.r)}, arrow ${Math.round(next.r)}, Info starts ${Math.round(info.x)})`);
await page.keyboard.press("i");
await until(() => !document.querySelector(".px-info"));
ok((await page.locator(".px-info").count()) === 0, "and I again hides it");

// Ctrl+Backspace: asked first, then gone, and the viewer goes on to the next photo.
await page.keyboard.press("Control+Backspace");
await until(() => window.__flags?.at(-1)?.flag === "deleted" && document.querySelector(".px-viewer")?.getAttribute("aria-label") === "IMG_3.JPG");
ok((await page.evaluate(() => window.__asked?.at(-1))) === "Delete this photo?" &&
  (await page.evaluate(() => window.__flags?.at(-1))).flag === "deleted", "Ctrl+Backspace deletes it, asked first");
ok((await shown()) === "IMG_3.JPG", "and the viewer goes on to the next");

// Through to a photo further on, then Escape: that photo is the one chosen, in view, focused.
for (let i = 0; i < 40; i++) await page.keyboard.press("ArrowRight");
// Settled on a photo: the same one shown over a few frames.
await until(async () => {
  const at = () => document.querySelector(".px-viewer")?.getAttribute("aria-label");
  const a = at();
  for (let i = 0; i < 6; i++) await new Promise((r) => requestAnimationFrame(r));
  return a === at();
});
const far = await shown();
await page.keyboard.press("Escape");
await until(() => !document.querySelector(".px-viewer") && document.activeElement?.matches('.px-tile[aria-selected="true"]'));
ok((await page.locator(".px-viewer").count()) === 0, "Escape closes it");
const back = await page.evaluate((name) => {
  const t = document.querySelector('.px-tile[aria-selected="true"]');
  const s = document.querySelector(".px-scroll").getBoundingClientRect();
  const r = t?.getBoundingClientRect();
  return { name: t?.getAttribute("aria-label")?.startsWith(name + ","), focused: document.activeElement === t, inView: !!r && r.top >= s.top && r.bottom <= s.bottom };
}, far);
ok(back.name && back.focused && back.inView, `the photo last shown (${far}) is chosen, focused and in view  ${JSON.stringify(back)}`);

// Space on a still takes it back to the library, as Photos.
await page.keyboard.press(" ");
await until((far) => document.querySelector(".px-viewer")?.getAttribute("aria-label") === far, far);
ok((await shown()) === far, "Space opens the chosen photo");
await page.keyboard.press(" ");
await until(() => !document.querySelector(".px-viewer"));
ok((await page.locator(".px-viewer").count()) === 0, "and Space on a still closes it again");

// ---- the library ----
/** The grid fewer across than `n` (or as many, `same`), the chosen photo drawn in it and still over a few frames. */
const zoomedTo = (n, same = false) => until(async ([n, same]) => {
  const across = () => { const g = document.querySelector(".px-grid"); return g ? getComputedStyle(g).gridTemplateColumns.split(" ").length : 0; };
  const want = () => (same ? across() === n : across() > 0 && across() < n);
  const top = () => document.querySelector('.px-tile[aria-selected="true"]')?.getBoundingClientRect().top;
  if (!want()) return false;
  const a = top();
  for (let i = 0; i < 4; i++) await new Promise((r) => requestAnimationFrame(r));
  return a !== undefined && a === top() && want();
}, [n, same]);
const across = () => page.locator(".px-grid").first().evaluate((g) => getComputedStyle(g).gridTemplateColumns.split(" ").length);
const where = () => page.evaluate(() => {
  const t = document.querySelector('.px-tile[aria-selected="true"]');
  return t ? Math.round(t.getBoundingClientRect().top - document.querySelector(".px-scroll").getBoundingClientRect().top) : NaN;
});
const was = await where(), n0 = await across();
await page.keyboard.press("Control+=");
await zoomedTo(n0);
const n1 = await across(), now = await where();
ok(n1 < n0, `Ctrl + makes the photos bigger  (${n0} → ${n1} across)`);
ok(Math.abs(now - was) < 40, `and the chosen photo stays where it was on the screen  (${was} → ${now}px)`);
const titles = await page.locator(".px-month-title").allInnerTexts();
ok(titles.length > 0 && titles.every((t) => !/^\w+ \d{4}$/.test(t)) && /day/.test(titles[0]), `five across, the library is grouped by day  (${titles.slice(0, 2).join(" | ")})`);
await page.keyboard.press("Control+-");
await zoomedTo(n0, true);
ok((await across()) === n0 && /^\w+ \d{4}$/.test((await page.locator(".px-month-title").first().innerText())), "Ctrl − back, by month again");

// The month picker still lists months, once each, by day as well as by month.
await page.keyboard.press("Control+=");
await zoomedTo(n0);
await page.getByRole("button", { name: "Go to a Month" }).click();
await until(() => document.querySelectorAll(".px-jump-month span").length >= 3);
const listed = await page.locator(".px-jump-month span").allInnerTexts();
ok(listed.length >= 3 && new Set(listed).size === listed.length, `by day, the month picker lists each month once  (${listed.join(", ")})`);
await page.keyboard.press("Escape");
await page.keyboard.press("Control+-");
await zoomedTo(n0, true);

// A sweep from the space beside the first month's heading down over two rows.
await page.locator(".px-scroll").evaluate((e) => (e.scrollTop = 0));
await until(async () => {
  const at = () => document.querySelector('.px-tile[data-index="9"]')?.getBoundingClientRect().top;
  const a = at();
  for (let i = 0; i < 4; i++) await new Promise((r) => requestAnimationFrame(r));
  return a !== undefined && a === at() && document.querySelector(".px-scroll").scrollTop === 0;
});
const title = await box(".px-month-title");
const t9 = await box('.px-tile[data-index="9"]');
await page.mouse.move(title.r - 8, title.y + 6);
await page.mouse.down();
await page.mouse.move(t9.x + t9.w / 2, t9.y + t9.h / 2, { steps: 6 });
await until(() => document.querySelector(".px-sweep")?.hidden === false);
const during = await page.locator(".px-sweep").evaluate((e) => !e.hidden);
await page.mouse.up();
await until(async () => {
  const chosen = () => document.querySelectorAll('.px-tile[aria-selected="true"]').length;
  const n = chosen();
  for (let i = 0; i < 4; i++) await new Promise((r) => requestAnimationFrame(r));
  return document.querySelector(".px-sweep")?.hidden === true && n > 0 && n === chosen();
});
const picked = await page.locator('.px-tile[aria-selected="true"]').evaluateAll((ts) => ts.map((t) => +t.dataset.index).sort((a, b) => a - b));
// From the last column down to the middle of photo 9 (second row): those columns of both rows.
const n = await across();
const col9 = 9 % n;
const want = [0, 1].flatMap((r) => Array.from({ length: n - col9 }, (_, c) => r * n + col9 + c));
ok(during, "sweeping draws the rectangle");
ok(picked.join() === want.join(), `a sweep selects exactly the photos it touches  (${picked.join(",")}; want ${want.join(",")})`);
ok((await page.locator(".px-sweep").evaluate((e) => e.hidden)), "and the rectangle goes when the button is let go");
const kept = picked.length;
await page.waitForTimeout(100);
ok((await page.locator('.px-tile[aria-selected="true"]').count()) === kept, "the click that ends a sweep does not let the selection go");

// Shift + click extends; Ctrl + click adds and takes away.
await page.locator('.px-tile[data-index="20"]').click();
await page.locator('.px-tile[data-index="24"]').click({ modifiers: ["Shift"] });
ok((await page.locator('.px-tile[aria-selected="true"]').count()) === 5, "Shift-click selects the run between");
await page.locator('.px-tile[data-index="30"]').click({ modifiers: ["Control"] });
await page.locator('.px-tile[data-index="22"]').click({ modifiers: ["Control"] });
ok((await page.locator('.px-tile[aria-selected="true"]').count()) === 5, "Ctrl-click adds one and takes one away");
ok((await page.locator(".px-bar .px-span").innerText()) === "5 Photos Selected", "the header counts them");

await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
