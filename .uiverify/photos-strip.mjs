// The viewer's filmstrip, as icloud.com's: the photos either side as slips, the shown one wider
// in the middle; a click goes to it and the strip glides to keep it in the middle; in a library
// of thousands only the nearest are built.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
await ctx.addInitScript(() => {
  const P = (id, t) => ({ id, master: `m${id}`, takenAt: t, addedAt: t, kind: "image", width: 4032, height: 3024, duration: null,
    filename: `IMG_${id}.JPG`, size: 1, uti: "public.jpeg", favorite: false, hidden: false, deleted: false, deletedAt: null, subtype: 0, changeTag: null });
  window.__photos = Array.from({ length: 2000 }, (_, i) => P(`q${i}`, 1792600000 - i * 40000));
});
await ctx.addInitScript(() => { window.__picBase = "http://pics.test"; });
const page = await ctx.newPage();
// Real pictures throughout, each a different colour, the screen-sized ones slow to come (as a
// photo not yet made at that size is), so a step has to wait for its picture.
await page.route("http://pics.test/**", async (route) => {
  const path = decodeURIComponent(route.request().url().split("/").pop());
  const n = Number(/q(\d+)/.exec(path)?.[1] ?? 0);
  if (/fit(\d+)/.test(path) && Number(/fit(\d+)/.exec(path)[1]) > 256) await new Promise((r) => setTimeout(r, 700));
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="1600" height="1200"><rect width="1600" height="1200" fill="hsl(${(n * 47) % 360} 60% 50%)"/></svg>`;
  await route.fulfill({ status: 200, contentType: "image/svg+xml", body: svg });
});

page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(500);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const centred = () => page.evaluate(() => {
  const s = document.querySelector(".px-slip.is-shown").getBoundingClientRect(), st = document.querySelector(".px-strip").getBoundingClientRect();
  return Math.abs(s.left + s.width / 2 - (st.left + st.width / 2));
});

await page.locator(".px-tile").nth(40).dblclick();
await page.waitForTimeout(800);
ok((await page.locator(".px-slip").count()) <= 2 * Math.max(30, Math.ceil(1280 / 28 / 2) + 12) + 1 && (await page.locator(".px-slip").count()) > 20, `only the nearest slips are built  (${await page.locator(".px-slip").count()} of 2000)`);
ok((await centred()) <= 1, "the shown photo's slip sits in the middle");
await page.locator(".px-slip").nth(25).click();
await page.waitForTimeout(700);
const shown = await page.locator(".px-shown").getAttribute("src");
const slip = await page.locator(".px-slip.is-shown img").getAttribute("src");
ok(shown.split("pic%2F")[1]?.split("%2F")[0] === slip.split("pic%2F")[1]?.split("%2F")[0], "a click on a slip goes to that photo");
ok((await centred()) <= 1, "and the strip glides to keep it in the middle");
for (let i = 0; i < 40; i++) await page.keyboard.press("ArrowRight");
await page.waitForTimeout(700);
ok((await centred()) <= 1 && (await page.locator(".px-slip").count()) <= 2 * Math.max(30, Math.ceil(1280 / 28 / 2) + 12) + 1, "stepping far along, still in the middle and still few");
// A wheel or a two-finger swipe over the strip skims: four notches of 40 go four photos on,
// and the slips glide on the compositor, never jump.
const which = async () => (await page.locator(".px-slip.is-shown img").getAttribute("src")).split("pic%2F")[1].split("%2F")[0].split("~")[0];
const before = await which();
const strip = await page.locator(".px-strip").boundingBox();
await page.mouse.move(strip.x + strip.width / 2, strip.y + strip.height / 2);
for (let i = 0; i < 4; i++) { await page.mouse.wheel(0, 40); await page.waitForTimeout(60); }
await page.waitForTimeout(700);
const after = await which();
const ids = await page.evaluate(() => window.__photos.map((p) => p.id));
ok(ids.indexOf(after) - ids.indexOf(before) === 4, `scrolling over the strip skims through the photos  (${ids.indexOf(before)} → ${ids.indexOf(after)})`);
ok((await centred()) <= 1, "and the strip keeps the shown one in the middle");
ok(await page.evaluate(() => [...document.querySelectorAll(".px-slip, .px-strip-run")].every((e) => {
  const cs = getComputedStyle(e);
  return cs.transitionDuration.split(",").every((d) => parseFloat(d) === 0);
}) && document.getAnimations().filter((a) => a.playState === "running").length === 0),
  "the strip and its slips move at once — nothing in it animates (animated, it flickered)");
ok((await page.locator(".px-slip img[loading=lazy]").count()) === 0, "and every slip's picture is fetched at once, never lazily");
// Stepping fast, frame by frame: every frame has a picture on the stage — the new one decoded,
// or the last one painted held under it — and a step never shows the soft thumbnail.
const seen = await page.evaluate(async () => {
  const bad = [];
  let frames = 0, held = 0;
  const look = () => {
    frames++;
    const top = document.querySelector(".px-pan > .px-shown");
    const under = document.querySelector(".px-stage > .px-under");
    const underOn = under && under.style.visibility !== "hidden";
    if (underOn) held++;
    if (underOn && getComputedStyle(top).backgroundImage !== "none") bad.push("thumbnail under a step");
    if (!underOn && !(top.complete && top.naturalWidth > 0) && getComputedStyle(top).backgroundImage === "none") bad.push("nothing on the stage");
  };
  for (let i = 0; i < 12; i++) {
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    for (let f = 0; f < 4; f++) { await new Promise((r) => requestAnimationFrame(r)); look(); }
  }
  // No neighbour is pre-decoded now, so the stepped-to screen-sized picture is made on the
  // step itself: wait for it to land rather than for a fixed while.
  for (let i = 0; i < 200; i++) {
    const u = document.querySelector(".px-stage > .px-under");
    if (!u || u.style.visibility === "hidden") break;
    await new Promise((r) => setTimeout(r, 16));
  }
  const under = document.querySelector(".px-stage > .px-under");
  return { bad: [...new Set(bad)], frames, held, left: !!under && under.style.visibility !== "hidden" };
});
ok(seen.bad.length === 0 && seen.held > 0, `stepping, every frame has a picture and never the thumbnail  (${seen.frames} frames, ${seen.held} holding the last; ${seen.bad.join(", ")})`);
ok(!seen.left, "and once the new picture is decoded, the one held under it is gone");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
