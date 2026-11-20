// Whole photos: only the photo is the photo (#225), as on icloud.com. Each photo sits in its
// own shape in a square cell; the room around it in the cell is the grid's. A sweep starts
// there as it does between cells, and the sweep chooses a photo only once it touches the photo
// itself, not the room beside it. The photo itself is still clicked, chosen and dragged out.
//
// Before: the whole cell was the tile, so a press beside a photo did nothing (it was the
// tile's) and a sweep over that room chose the photo early.
//
//   URL=http://localhost:4173 node .uiverify/photos-sweep-whole.mjs   (ENGINE=webkit for WebKit)
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
const page = await ctx.newPage();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(400);
await page.getByRole("button", { name: "Show Whole Photos" }).click();
await page.waitForTimeout(250);
await page.evaluate(() => {
  window.__drags = 0;
  document.addEventListener("dragstart", () => window.__drags++, true);
});

// Two photos side by side on the first row: each one's cell and its own (4:3, landscape) frame.
const boxes = async () => page.evaluate(() => {
  const tiles = [...document.querySelectorAll(".px-tile[data-id]")];
  const top = tiles[0].getBoundingClientRect().top;
  return tiles.filter((t) => Math.abs(t.getBoundingClientRect().top - top) < 1).slice(0, 2).map((t) => {
    const c = t.getBoundingClientRect(), p = t.querySelector(".px-pic").getBoundingClientRect();
    return { id: t.dataset.id, cell: { x: c.x, y: c.y, w: c.width, h: c.height }, pic: { x: p.x, y: p.y, w: p.width, h: p.height } };
  });
});
const [one, two] = await boxes();
ok(one && two && one.pic.h < one.cell.h - 20, `a landscape photo leaves room above and below it in its cell (${one?.pic.h.toFixed(0)} of ${one?.cell.h.toFixed(0)})`);
const chosen = () => page.evaluate(() => [...document.querySelectorAll('.px-tile[aria-selected="true"]')].map((t) => t.dataset.id));
// Let go of what is chosen as a person does: a click in the gap between two photos' cells.
const clear = async () => {
  await page.mouse.click((one.cell.x + one.cell.w + two.cell.x) / 2, one.cell.y + one.cell.h / 2);
  await page.waitForTimeout(120);
};
// Above the photo, inside its cell.
const above = (t) => ({ x: t.pic.x + t.pic.w / 2, y: t.cell.y + (t.pic.y - t.cell.y) / 2 });

// What a press there lands on: the grid, not the tile.
const hit = await page.evaluate(({ x, y }) => !!document.elementFromPoint(x, y)?.closest(".px-tile"), above(one));
ok(!hit, "the room above a photo in its cell is not the photo");

// 1. A sweep starts in the room above a photo and chooses the photo once it reaches it.
await clear();
let at = above(one);
await page.mouse.move(at.x, at.y);
await page.mouse.down();
for (let i = 1; i <= 6; i++) await page.mouse.move(at.x + (i * 10), at.y + (i * (one.pic.h / 2 + (one.pic.y - at.y))) / 6);
await page.waitForTimeout(80);
const sweepShown = await page.evaluate(() => { const s = document.querySelector(".px-sweep"); return !!s && !s.hidden && s.getBoundingClientRect().height > 5; });
ok(sweepShown, "a sweep starts in the room beside a photo, as it does between cells");
await page.mouse.up();
await page.waitForTimeout(120);
ok((await chosen()).join() === one.id, `and chooses the photo it reaches  ${JSON.stringify(await chosen())}`);
ok((await page.evaluate(() => window.__drags)) === 0, "and drags no photo out");

// 2. A thin sweep across only the room above two photos chooses neither.
await clear();
at = { x: one.pic.x + 4, y: one.cell.y + 3 };
await page.mouse.move(at.x, at.y);
await page.mouse.down();
for (let i = 1; i <= 6; i++) await page.mouse.move(at.x + (i * (two.pic.x + two.pic.w / 2 - at.x)) / 6, at.y + i);
await page.waitForTimeout(80);
await page.mouse.up();
await page.waitForTimeout(120);
ok((await chosen()).length === 0, `a sweep over only the room beside photos chooses none  ${JSON.stringify(await chosen())}`);

// 3. A click in that room chooses nothing; a click on the photo chooses it.
await clear();
await page.mouse.click(above(two).x, above(two).y);
await page.waitForTimeout(150);
ok((await chosen()).length === 0, "a click beside a photo chooses nothing");
await page.mouse.click(two.pic.x + two.pic.w / 2, two.pic.y + two.pic.h / 2);
await page.waitForTimeout(150);
ok((await chosen()).join() === two.id, "a click on the photo chooses it");

// 4. The photo itself is still dragged out.
await page.mouse.move(two.pic.x + two.pic.w / 2, two.pic.y + two.pic.h / 2);
await page.mouse.down();
for (let i = 1; i <= 8; i++) await page.mouse.move(two.pic.x + two.pic.w / 2 + i * 12, two.pic.y + two.pic.h / 2 + i * 6);
await page.waitForTimeout(80);
await page.mouse.up();
ok((await page.evaluate(() => window.__drags)) === 1, `the photo itself still drags out (${await page.evaluate(() => window.__drags)} drags)`);

await b.close();
process.exitCode = failures ? 1 : 0;
