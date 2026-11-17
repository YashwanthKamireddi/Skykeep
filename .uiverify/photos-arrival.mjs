// A photo taken on the phone arrives in the open library in place, and nothing else moves: the
// tiles already drawn keep their elements and their pictures (no grey flash, no grid swapped for
// its placeholder and back), a month that gains a photo draws it rather than losing its last
// row, a new month draws no other month again, and the view scrolled down stays on the same
// photos when photos land above it. Only the new tile fades in, briefly — not for a batch, not
// for the person's own upload, not with reduced motion, not on opening or switching views.
//
//   URL=http://localhost:4173 node .uiverify/photos-arrival.mjs   (ENGINE=webkit for WebKit)
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
import { PICS, instrument, land, servePictures } from "./arrival.mjs";

const URL = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

// Six hundred photos over ten months, newest first, twelve hours apart.
function library() {
  const P = (id, takenAt, extra = {}) => ({ id, master: `m${id}`, takenAt, addedAt: takenAt, kind: "image",
    width: 4032, height: 3024, duration: null, filename: `IMG_${id}.JPG`, size: 2400000, uti: "public.jpeg",
    favorite: false, hidden: false, deleted: false, deletedAt: null, subtype: 0, changeTag: null, ...extra });
  window.__photos = Array.from({ length: 600 }, (_, i) => P(`p${i + 1}`, 1792500000 - i * 43200));
  window.__mkPhoto = P;
  window.__arrive = (photos) => { window.__photos = [...photos, ...window.__photos].sort((a, b) => b.takenAt - a.takenAt); };
}
const cfg = { items: ".px-tile", idAttr: "data-id", pictures: true, frames: 50 };
const newest = 1792500000;

async function open({ reduced = false } = {}) {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 800 }, reducedMotion: reduced ? "reduce" : "no-preference" });
  await ctx.addInitScript(installStub, { photosNewestFirst: true, screen: "library", services: SERVICES, twoFactor: null });
  await ctx.addInitScript(instrument);
  await ctx.addInitScript(library);
  await servePictures(ctx);
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(URL, { waitUntil: "networkidle" });
  await openScreen(page, "Photos");
  await page.locator(".px-tile").first().waitFor();
  await page.waitForTimeout(1200);
  return { ctx, page };
}
const arrive = (photos) => `window.__arrive(${photos})`;
const one = (id, at, extra = "{}") => `[window.__mkPhoto(${JSON.stringify(id)}, ${at}, ${extra})]`;
const many = (n, from, tag = "pN") => `Array.from({ length: ${n} }, (_, i) => window.__mkPhoto("${tag}" + i, ${from} + (${n} - i) * 60))`;
const say = (r) => `remade ${r.remade}, blank ${r.blank}, lost ${r.lost}, moved ${r.moved}, fresh ${r.fresh}, faded ${r.fadedFresh}+[${r.fadedOther.join(",")}], old pictures loaded ${r.oldLoads}`;

{
  const { ctx, page } = await open();
  // F5: a tile's picture is addressed with its version, so the engine may keep it for good.
  const src = await page.locator(".px-tile img").first().getAttribute("src");
  ok(/pic%2Fp1~[0-9a-z]+%2Ffit|pic\/p1~[0-9a-z]+\/fit/.test(src ?? ""), `a tile's picture carries its version (${src?.slice(-40)})`);

  // Nothing new: nothing drawn again, nothing fetched again.
  const hits = ctx.__pictureHits.length;
  let r = await land(page, cfg, { changed: ["photos"] });
  ok(r.remade === 0 && r.blank === 0 && r.loads === 0 && Object.keys(r.removed).length === 0, `a landing with nothing new changes nothing (${say(r)}, removed ${JSON.stringify(r.removed)})`);
  ok(ctx.__pictureHits.length === hits, `…and asks for no picture (${ctx.__pictureHits.length - hits})`);

  // A heart given on the phone: the tile shows it, its picture is not fetched again.
  r = await land(page, cfg, { changed: ["photos"], mutate: `window.__photos = window.__photos.map((p) => p.id === "p2" ? { ...p, favorite: true, changeTag: "t2" } : p)` });
  ok((await page.locator('.px-tile[data-id="p2"] .px-heart').count()) === 1, "a heart given on the phone shows on its tile");
  ok(r.remade === 0 && r.srcChanges === 0 && r.oldLoads === 0 && r.fadedFresh === 0 && r.fadedOther.length === 0, `…which is not drawn again, fetched again or faded (${say(r)}, src changes ${r.srcChanges})`);

  // One new photo at the top of the newest month, the grid at the top.
  r = await land(page, cfg, { changed: ["photos"], mutate: arrive(one("pA", newest + 600)) });
  ok(r.remade === 0, `a photo arriving draws no other tile again (${say(r)})`);
  ok(r.blank === 0, `no tile already showing goes blank, not for a frame (${r.blank})`);
  ok(r.fresh === 1 && (await page.locator('.px-tile[data-id="pA"]').count()) === 1, "the new photo is in the grid");
  ok(r.fadedFresh === 1 && r.fadedOther.length === 0, `only the new tile fades in (${r.fadedFresh} new, others [${r.fadedOther.join(",")}])`);
  ok(r.oldLoads === 0, `no picture already shown is fetched again (${r.oldLoads})`);

  // A month that is built whole gains a photo in its middle: it draws all of its photos.
  const month = await page.evaluate(() => {
    const sec = document.querySelectorAll(".px-month")[1];
    const ids = [...sec.querySelectorAll(".px-tile")].map((t) => t.dataset.id);
    return { label: sec.getAttribute("aria-label"), ids };
  });
  const after = await page.evaluate((id) => window.__photos.find((p) => p.id === id).takenAt, month.ids[1]);
  // Frame by frame: the month's tiles, and whether any of it went back to its placeholder block.
  const inMonth = `(() => { const sec = document.querySelector('.px-month[aria-label="${month.label}"]'); return { monthTiles: sec ? sec.querySelectorAll(".px-tile").length : -1, monthLater: sec ? sec.querySelectorAll(".px-later").length : -1 }; })()`;
  r = await land(page, { ...cfg, probe: inMonth }, { changed: ["photos"], mutate: arrive(one("pB", after + 30)) });
  const fewest = Math.min(...r.frames.map((f) => f.monthTiles));
  const blocks = Math.max(...r.frames.map((f) => f.monthLater));
  ok(r.last.monthTiles === month.ids.length + 1 && blocks === 0 && fewest >= month.ids.length, `a whole month that gains a photo draws every one of them, in every frame (${r.last.monthTiles} of ${month.ids.length + 1}, fewest ${fewest}, ${blocks} placeholder blocks)`);
  ok(r.remade === 0 && r.blank === 0, `…drawing none of them again (${say(r)})`);

  // A photo from a month newer than any: a new month at the top, and no other month built again.
  r = await land(page, cfg, { changed: ["photos"], mutate: arrive(one("pC", newest + 40 * 86400)) });
  ok(r.remade === 0 && r.blank === 0, `a new month draws no other month again (${say(r)})`);
  ok(r.fresh === 1 && r.fadedFresh === 1 && r.fadedOther.length === 0, `…and its one photo fades in alone (${say(r)})`);

  // Thirty at once: they appear, none fades.
  r = await land(page, cfg, { changed: ["photos"], mutate: arrive(many(30, newest + 50 * 86400)) });
  ok(r.remade === 0 && r.blank === 0, `thirty arriving at once draw no other tile again (${say(r)})`);
  ok(r.fadedFresh === 0 && r.fadedOther.length === 0, `…and none of them fades in (${r.fadedFresh})`);

  // The person's own upload coming back: it simply appears.
  await page.evaluate(() => window.__emit("quince://photo-upload", { id: "u1", batch: "b1", name: "IMG_pOWN.JPG", within: "", sent: 1000, total: 1000, folder: false, error: null, state: "done" }));
  await page.waitForTimeout(200);
  r = await land(page, cfg, { changed: ["photos"], mutate: arrive(one("pOWN", newest + 60 * 86400)) });
  ok(r.fresh === 1 && r.fadedFresh === 0, `a photo uploaded from here appears without a fade (${say(r)})`);

  // Scrolled down: photos landing above leave the view on the same photos.
  await page.evaluate(() => { document.querySelector(".px-scroll").scrollTop = 2500; });
  await page.waitForTimeout(1200);
  r = await land(page, cfg, { changed: ["photos"], mutate: arrive(many(5, newest + 70 * 86400, "pU")) });
  ok(r.moved <= 1 && r.lost === 0, `photos landing above the view move nothing in it (moved ${r.moved}px, lost ${r.lost})`);
  ok(r.remade === 0 && r.blank === 0, `…and draw nothing in it again (${say(r)})`);
  // And another month built above: still in place.
  r = await land(page, cfg, { changed: ["photos"], mutate: arrive(one("pD", newest + 120 * 86400)) });
  ok(r.moved <= 1 && r.lost === 0 && r.remade === 0, `a new month above the view moves nothing in it (${say(r)})`);
  await ctx.close();
}

// Reduced motion: the photo is there at once.
{
  const { ctx, page } = await open({ reduced: true });
  const r = await land(page, cfg, { changed: ["photos"], mutate: arrive(one("pR", newest + 600)) });
  ok(r.fresh === 1 && r.fadedFresh === 0 && r.fadedOther.length === 0, `with reduced motion a new photo is there at once (${say(r)})`);
  await ctx.close();
}

// A batch from another device is said under the title — "Receiving 30 photos" — while its
// pictures come in, then the line goes; one photo, or the person's own upload, says nothing.
{
  const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
  await ctx.addInitScript(installStub, { photosNewestFirst: true, screen: "library", services: SERVICES, twoFactor: null });
  await ctx.addInitScript(instrument);
  await ctx.addInitScript(library);
  await ctx.addInitScript(() => {
    window.__said = [];
    new MutationObserver(() => {
      const t = document.querySelector(".px-receiving")?.textContent;
      if (t && window.__said.at(-1) !== t) window.__said.push(t);
    }).observe(document, { subtree: true, childList: true, characterData: true });
  });
  // The first screen's pictures from the disk; the batch's a while in coming, as from the
  // network (`slow`, set once the grid is drawn): the line waits for them. Held back by a route
  // of its own over servePictures' — the later route is asked first, and passes it on.
  await servePictures(ctx);
  let slow = 0;
  await ctx.route(`${PICS}/**`, async (route) => {
    if (slow) await new Promise((r) => setTimeout(r, slow));
    await route.fallback();
  });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(URL, { waitUntil: "networkidle" });
  await openScreen(page, "Photos");
  await page.locator(".px-tile img[data-in]").first().waitFor({ timeout: 15000 });
  slow = 1500;
  const said = () => page.evaluate(() => window.__said.splice(0));
  const gone = () => page.waitForFunction(() => !document.querySelector(".px-receiving"), null, { timeout: 30000 }).then(() => true, () => false);

  await land(page, cfg, { changed: ["photos"], mutate: arrive(one("pS", newest + 600)) });
  ok((await said()).length === 0, "one photo arriving says nothing under the title");

  await land(page, cfg, { changed: ["photos"], mutate: arrive(many(30, newest + 50 * 86400, "pB")) });
  const words = await said();
  ok(words.includes("Receiving 30 photos"), `thirty arriving at once say "Receiving 30 photos" (${JSON.stringify(words)})`);
  ok(/·\s*Receiving 30 photos/.test(await page.locator(".px-span").innerText()), `…after the months shown, on the title's own line (${await page.locator(".px-span").innerText()})`);
  // Still there while their pictures are on their way, past the moment it always stays.
  await page.waitForTimeout(1900);
  const waiting = await page.evaluate(() => {
    const imgs = [...document.querySelectorAll('.px-tile[data-id^="pB"] img')];
    return { pending: imgs.some((i) => !i.dataset.in), line: !!document.querySelector(".px-receiving") };
  });
  ok(!waiting.pending || waiting.line, `…and stays while their pictures are still coming (${JSON.stringify(waiting)})`);
  slow = 0;
  ok(await gone(), "…and goes once they are in");

  await land(page, cfg, { changed: ["photos"], mutate: arrive(`[...${many(18, newest + 60 * 86400, "pV")}].map((p, i) => i < 3 ? { ...p, kind: "video", duration: 9 } : p)`) });
  ok((await said()).includes("Receiving 18 items"), "photos and videos together are items");
  await gone();

  // The person's own upload, landing as a batch: the upload panel already speaks for it.
  await page.evaluate(() => { for (let i = 0; i < 20; i++) window.__emit("quince://photo-upload", { id: `u${i}`, batch: "b1", name: `IMG_pO${i}.JPG`, within: "", sent: 1, total: 1, folder: false, error: null, state: "done" }); });
  await page.waitForTimeout(200);
  await land(page, cfg, { changed: ["photos"], mutate: arrive(many(20, newest + 70 * 86400, "pO")) });
  ok((await said()).length === 0, "the person's own upload landing says nothing under the title");
  await ctx.close();
}

// Opening Photos and switching views never fades a tile.
{
  const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
  await ctx.addInitScript(installStub, { photosNewestFirst: true, screen: "library", services: SERVICES, twoFactor: null });
  await ctx.addInitScript(library);
  await ctx.addInitScript(() => {
    window.__fades = 0;
    new MutationObserver((ms) => {
      for (const m of ms) {
        const o = m.target.style?.opacity;
        if (o && +o > 0 && +o < 1 && m.target.closest?.(".px-tile")) window.__fades++;
      }
    }).observe(document, { subtree: true, attributes: true, attributeFilter: ["style"] });
  });
  await servePictures(ctx);
  const page = await ctx.newPage();
  await page.goto(URL, { waitUntil: "networkidle" });
  await openScreen(page, "Photos");
  await page.locator(".px-tile").first().waitFor();
  await page.waitForTimeout(800);
  await page.locator(".source-row", { hasText: "Favorites" }).first().click().catch(() => {});
  await page.waitForTimeout(600);
  await page.locator(".source-row", { hasText: "Library" }).first().click().catch(() => {});
  await page.waitForTimeout(800);
  ok((await page.evaluate(() => window.__fades)) === 0, `opening Photos and switching views fades no tile (${await page.evaluate(() => window.__fades)})`);
  await ctx.close();
}

await b.close();
console.log(failures ? `${failures} failed` : "all passed");
process.exit(failures ? 1 : 0);
