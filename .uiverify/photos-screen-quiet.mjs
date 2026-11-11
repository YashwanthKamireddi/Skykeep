// Photos draws only what changed, and the viewer's keys stay with what is in front of it.
//
// Pages of photos landing while the grid scrolls draw the months they land in, never the whole
// screen (header, sidebar, toolbar and every month's comparison): subscribed to the store's
// version, the 4,000-line screen drew again for every page, several times a second, in the
// scroll's own frames. A sync that changes nothing in a month's photos draws that month no
// more. Upload events are laid in once a frame and drawn by their panel alone.
//
// With a sheet or menu over the viewer (Adjust Date, the Delete confirm), F, H, Delete and the
// arrows belong to the sheet: they favorited, hid or deleted the photo behind it.
//
//   BASE=http://localhost:4173 node .uiverify/photos-screen-quiet.mjs   (ENGINE=webkit for WebKit)
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const BASE = process.env.BASE ?? process.env.URL ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

// Counts the renders of the component that draws an element (`window.__countOf(selector)`), as
// React's developer tools see them: a component drawn again gets a new list of hook states.
function counter() {
  let seen = new Map();
  window.__renders = {};
  const fiberOf = (el) => {
    const key = el && Object.keys(el).find((k) => k.startsWith("__reactFiber$"));
    return key ? el[key] : null;
  };
  const ownerOf = (f) => {
    for (let x = f?.return; x; x = x.return) if (x.tag === 0 || x.tag === 14 || x.tag === 15) return x;
    return null;
  };
  const rootOf = (f) => {
    let x = f;
    while (x?.return) x = x.return;
    return x;
  };
  window.__REACT_DEVTOOLS_GLOBAL_HOOK__ = {
    supportsFiber: true,
    renderers: new Map(),
    inject() { return 1; },
    checkDCE() {},
    onScheduleFiberRoot() {},
    onPostCommitFiberRoot() {},
    onCommitFiberUnmount() {},
    onCommitFiberRoot(_id, root) {
      for (const [name, selector] of Object.entries(window.__watch ?? {})) {
        const owner = ownerOf(fiberOf(document.querySelector(selector)));
        if (!owner) continue;
        const current = rootOf(owner) === root.current ? owner : owner.alternate;
        if (!current) continue;
        if (seen.get(name) !== current.memoizedState) {
          if (seen.has(name)) window.__renders[name] = (window.__renders[name] ?? 0) + 1;
          seen.set(name, current.memoizedState);
        }
      }
    },
  };
  window.__resetRenders = () => { window.__renders = {}; };
}

// Five thousand photos an hour apart, newest first: seven months of them.
function library() {
  const P = (i) => ({ id: `p${i + 1}`, master: `mp${i + 1}`, takenAt: 1792500000 - i * 3600, addedAt: 1792500000 - i * 3600, kind: "image",
    width: 4032, height: 3024, duration: null, filename: `IMG_${i + 1}.JPG`, size: 2400000, uti: "public.jpeg",
    favorite: false, hidden: false, deleted: false, deletedAt: null, subtype: 0, changeTag: null });
  window.__photos = Array.from({ length: 5000 }, (_, i) => P(i));
  // Pages held while `__hold` is set, let go by `__release()`.
  window.__held = [];
  window.__release = () => { window.__hold = false; window.__held.splice(0).forEach((r) => r()); };
  const wrap = () => {
    const t = window.__TAURI_INTERNALS__;
    if (!t || t.__held) return;
    const inner = t.invoke;
    t.invoke = async (cmd, args, o) => {
      if (cmd === "photos_get" && window.__hold) {
        await new Promise((r) => window.__held.push(r));
        (window.__landed ??= []).push(args.ids.length);
      }
      return inner(cmd, args, o);
    };
    t.__held = true;
  };
  wrap();
  document.addEventListener("DOMContentLoaded", wrap);
}

async function open() {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
  await ctx.addInitScript(counter);
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
  await ctx.addInitScript(library);
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(BASE, { waitUntil: "networkidle" });
  await openScreen(page, "Photos");
  await page.locator(".px-tile").first().waitFor();
  await page.waitForTimeout(800);
  return { ctx, page };
}

// Pages landing while scrolled: the months draw them, the screen does not draw again.
{
  const { ctx, page } = await open();
  await page.evaluate(() => { window.__watch = { screen: ".px-bar" }; window.__hold = true; window.__landed = []; });
  // Down past what is held, a step at a time as a wheel goes, then still: the pages it asks for wait.
  for (let i = 1; i <= 12; i++) {
    await page.evaluate((y) => { document.querySelector(".px-scroll").scrollTop = y; }, i * 7000);
    await page.waitForTimeout(60);
  }
  await page.waitForTimeout(1000);
  const held = await page.evaluate(() => window.__held.length);
  ok(held > 0, `scrolling down asks for pages (${held} held)`);
  await page.evaluate(() => window.__resetRenders());
  await page.evaluate(() => window.__release());
  await page.waitForTimeout(1200);
  const landed = await page.evaluate(() => window.__landed.length);
  const r = await page.evaluate(() => window.__renders);
  ok(landed > 0 && (await page.locator(".px-tile").count()) > 0, `the pages land and their tiles are drawn (${landed} pages)`);
  ok((r.screen ?? 0) === 0, `the screen is not drawn again for pages landing (${r.screen ?? 0} draws for ${landed} pages)`);

  // An upload's events: laid in once a frame, and the screen is not drawn again for them.
  await page.evaluate(() => window.__resetRenders());
  await page.evaluate(() => {
    for (let i = 0; i < 300; i++) window.__emit("quince://photo-upload", { id: `u${i}`, batch: "b1", name: `IMG_U${i}.JPG`, within: "", sent: 0, total: 1000, folder: false, error: null, state: "waiting" });
    for (let i = 0; i < 300; i++) window.__emit("quince://photo-upload", { id: `u${i}`, batch: "b1", name: `IMG_U${i}.JPG`, within: "", sent: 500, total: 1000, folder: false, error: null, state: "sending" });
  });
  await page.waitForTimeout(400);
  const rows = await page.locator(".px-up-row").count();
  const ur = await page.evaluate(() => window.__renders);
  ok(rows === 300, `every file on its way is listed (${rows})`);
  ok((ur.screen ?? 0) === 0, `six hundred upload events draw the screen no more (${ur.screen ?? 0})`);
  // A finished file never goes back, even when an older event comes after it.
  await page.evaluate(() => {
    window.__emit("quince://photo-upload", { id: "u0", batch: "b1", name: "IMG_U0.JPG", within: "", sent: 1000, total: 1000, folder: false, error: null, state: "done" });
    window.__emit("quince://photo-upload", { id: "u0", batch: "b1", name: "IMG_U0.JPG", within: "", sent: 700, total: 1000, folder: false, error: null, state: "sending" });
  });
  await page.waitForTimeout(200);
  ok((await page.locator(".px-up-row.is-done").count()) === 1, "a finished upload stays finished when an older event comes after it");
  await ctx.close();
}

// The viewer's keys while a sheet is over it.
{
  const { ctx, page } = await open();
  await page.locator(".px-tile").nth(1).dblclick();
  await page.locator(".px-viewer").waitFor();
  await page.waitForTimeout(400);
  const name = async () => page.locator(".px-viewer").getAttribute("aria-label");
  const first = await name();
  await page.keyboard.press("i");
  await page.locator(".px-adjust").click();
  const sheet = page.getByRole("dialog", { name: "Adjust Date and Time" });
  await sheet.waitFor();
  await sheet.locator(".sheet-close").focus();
  await page.evaluate(() => { window.__flags = []; });
  for (const key of ["f", "h", "ArrowRight", "Delete"]) await page.keyboard.press(key);
  await page.waitForTimeout(300);
  ok((await page.evaluate(() => window.__flags.length)) === 0, `F, H and Delete in Adjust Date change nothing behind it (${JSON.stringify(await page.evaluate(() => window.__flags))})`);
  ok((await page.locator(".modal-sheet.confirm").count()) === 0, "…and ask nothing");
  ok((await name()) === first, `…and the arrows leave the photo where it was (${first} → ${await name()})`);
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);
  ok((await sheet.count()) === 0 && (await page.locator(".px-viewer").count()) === 1, "Escape closes the sheet, the photo stays open");

  // The Delete confirm: F and the arrows there leave the photo alone.
  await page.locator(".px-viewer").click({ position: { x: 640, y: 400 } }).catch(() => {});
  await page.evaluate(() => document.activeElement?.blur());
  await page.keyboard.press("Delete");
  const confirm = page.locator(".modal-sheet.confirm");
  await confirm.waitFor();
  for (const key of ["f", "h", "ArrowRight"]) await page.keyboard.press(key);
  await page.waitForTimeout(300);
  ok((await page.evaluate(() => window.__flags.length)) === 0, `F and H in the Delete confirm change nothing (${JSON.stringify(await page.evaluate(() => window.__flags))})`);
  ok((await name()) === first, `…and the arrows leave the photo where it was (${await name()})`);
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);
  ok((await confirm.count()) === 0 && (await page.evaluate(() => window.__flags.length)) === 0, "Escape keeps the photo");
  // With nothing over it, the keys are the viewer's again.
  await page.evaluate(() => document.activeElement?.blur());
  await page.keyboard.press("f");
  await page.waitForTimeout(200);
  ok((await page.evaluate(() => window.__flags.filter((f) => f.flag === "favorite").length)) === 1, "with nothing over the viewer, F favorites the photo");
  await ctx.close();
}

await b.close();
console.log(failures ? `\n${failures} failed` : "\nall passed");
process.exitCode = failures ? 1 : 0;
