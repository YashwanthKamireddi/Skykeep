// A sweep with a key held, as Finder's icon view and File Explorer have it (#226): Shift adds
// what it touches to what was chosen; Ctrl (⌘ on a Mac) turns each photo it touches over, so a
// chosen one is let go. Both are measured against the choice as the sweep began: a sweep
// pulled back off a photo gives it back as it was. Before, both keys only ever added.
//
// In Whole photos, where a sweep starts in the room beside a photo (#225).
//
//   URL=http://localhost:4173 node .uiverify/photos-sweep-keys.mjs   (ENGINE=webkit for WebKit)
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

// The first row's first two photos: each one's cell and its own frame.
const [one, two] = await page.evaluate(() => {
  const tiles = [...document.querySelectorAll(".px-tile[data-id]")];
  const top = tiles[0].getBoundingClientRect().top;
  return tiles.filter((t) => Math.abs(t.getBoundingClientRect().top - top) < 1).slice(0, 2).map((t) => {
    const c = t.getBoundingClientRect(), p = t.querySelector(".px-pic").getBoundingClientRect();
    return { id: t.dataset.id, cell: { x: c.x, y: c.y, w: c.width, h: c.height }, pic: { x: p.x, y: p.y, w: p.width, h: p.height } };
  });
});
const chosen = async () => (await page.evaluate(() => [...document.querySelectorAll('.px-tile[aria-selected="true"]')].map((t) => t.dataset.id))).sort().join();
const both = [one.id, two.id].sort().join();
// From the room above a photo to a point on a photo (or back off it), with `key` held.
const sweep = async (from, to, key, back) => {
  const at = { x: from.pic.x + from.pic.w / 2, y: from.cell.y + (from.pic.y - from.cell.y) / 2 };
  if (key) await page.keyboard.down(key);
  await page.mouse.move(at.x, at.y);
  await page.mouse.down();
  const end = { x: to.pic.x + to.pic.w / 2, y: to.pic.y + to.pic.h / 2 };
  for (let i = 1; i <= 6; i++) await page.mouse.move(at.x + (i * (end.x - at.x)) / 6, at.y + (i * (end.y - at.y)) / 6);
  await page.waitForTimeout(60);
  if (back) {
    // Pulled back up into the room above, off the photos again.
    for (let i = 1; i <= 6; i++) await page.mouse.move(end.x, end.y + (i * (at.y + 4 - end.y)) / 6);
    await page.waitForTimeout(60);
  }
  await page.mouse.up();
  if (key) await page.keyboard.up(key);
  await page.waitForTimeout(120);
};

await sweep(one, two);
ok((await chosen()) === both, `a sweep over two photos chooses both  (${await chosen()})`);

await sweep(two, two, "Control");
ok((await chosen()) === one.id, `Ctrl and a sweep over a chosen photo lets it go, the other stays  (${await chosen()})`);

await sweep(two, two, "Control");
ok((await chosen()) === both, `Ctrl and a sweep over it again chooses it again  (${await chosen()})`);

await sweep(two, two, "Control", true);
ok((await chosen()) === both, `a Ctrl sweep pulled back off a photo gives it back as it was  (${await chosen()})`);

await sweep(two, two, "Control");
await sweep(one, two, "Shift");
ok((await chosen()) === both, `Shift and a sweep adds, and never lets a chosen photo go  (${await chosen()})`);

await sweep(two, two);
ok((await chosen()) === two.id, `a sweep with no key starts a new choice  (${await chosen()})`);

await b.close();
process.exitCode = failures ? 1 : 0;
