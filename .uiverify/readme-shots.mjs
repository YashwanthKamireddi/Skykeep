// The README's screenshots and hero animation, made from the real interface with an invented
// library (readme-fixture.mjs) and drawn pictures (readme-pictures.mjs).
//
//   node .uiverify/readme-shots.mjs             everything, into docs/media/
//   node .uiverify/readme-shots.mjs notes hero  only those scenes
//
// It builds the interface and previews it on PORT (4220) unless something already answers
// there. Needs ImageMagick (`magick`) and ffmpeg (with libwebp). Stills and the loop are written
// as WebP. Every picture is drawn; no photo file is read.
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { chromium } from "playwright";
import { SERVICES, installStub, openScreen, openSettings } from "./stub.mjs";
import { installDemo, photoLibrary } from "./readme-fixture.mjs";
import { drawPictures } from "./readme-pictures.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const OUT = process.env.OUT ?? path.join(ROOT, "docs/media");
const PORT = Number(process.env.PORT ?? 4220);
const WORK = process.env.WORK ?? path.join(os.tmpdir(), "skykeep-readme-shots");
const URL_ = `http://localhost:${PORT}`;
const PICS = `${URL_}/__pics`;
const ONLY = new Set(process.argv.slice(2));
const want = (name) => ONLY.size === 0 || ONLY.has(name);
// Wednesday, 9:41 in the morning, in a zone the fixture's events are written in.
const TZ = "America/Los_Angeles";
const NOW = new Date("2026-10-21T09:41:00-07:00");
const LIBRARY = photoLibrary();
// The window: small enough that its text reads at the README's 880 px, wide enough for every
// app's columns. Drawn at 2x, then each still is scaled to WIDTH, twice the README's width.
const WIN = { width: 1200, height: 760 };
const PAD = [44, 40];
const WIDTH = 1760;

fs.mkdirSync(OUT, { recursive: true });
fs.mkdirSync(WORK, { recursive: true });
const sh = (cmd, args) => execFileSync(cmd, args, { stdio: ["ignore", "ignore", "inherit"] });

// ---- The interface, served.
let server = null;
const up = async () => { try { return (await fetch(URL_)).ok; } catch { return false; } };
if (!(await up())) {
  const dist = path.join(WORK, "dist");
  execFileSync("npx", ["vite", "build", "--outDir", dist, "--emptyOutDir"], { cwd: ROOT, stdio: "ignore" });
  server = spawn("npx", ["vite", "preview", "--outDir", dist, "--port", String(PORT), "--strictPort"], { cwd: ROOT, stdio: "ignore", detached: true });
  process.on("exit", () => { try { process.kill(-server.pid); } catch {} });
  for (let i = 0; i < 100 && !(await up()); i++) await new Promise((r) => setTimeout(r, 200));
}

const browser = await chromium.launch();

// ---- Pictures, all drawn: the photo library, Drive's thumbnails, a note's picture, faces.
const pictures = new Map();
{
  const page = await browser.newPage();
  const specs = LIBRARY.map((p) => ({ name: p.id, w: p.w, h: p.h, scene: p.scene, seed: p.seed }));
  const L = [1200, 900], P = [900, 1200];
  for (const [name, [w, h], scene, seed] of [
    ["lakedawn", L, "lake", 5], ["ridge", L, "landscape", 2], ["fire", L, "fire", 1], ["aurora", L, "aurora", 2],
    ["stars", L, "night", 8], ["mist", L, "landscape", 4], ["pines", L, "forest", 3], ["porch", L, "coffee", 2], ["shore", L, "sea", 6],
    ["map", P, "map", 4], ["plan", P, "page", 6], ["packing", P, "page", 9], ["costs", P, "sheet", 3], ["slides", [1280, 720], "slides", 1],
    ["budget", P, "sheet", 5], ["portfolio", P, "page", 2],
  ]) specs.push({ name: `drive/${name}`, w, h, scene, seed });
  specs.push({ name: "note/M1", w: 1600, h: 560, scene: "lake", seed: 13 });
  // Ruth (c10) is the one with grey hair.
  for (let i = 1; i <= 25; i++) specs.push({ name: `face/${i}`, w: 400, h: 400, scene: "portrait", seed: i, grey: i === 10 });
  for (const [k, v] of Object.entries(await page.evaluate(drawPictures, specs))) pictures.set(k, Buffer.from(v, "base64"));
  await page.close();
}
// pic/<id>/<size>, note/<id>/<part>, drive/<name>, photo/<contact>?v=…
const pictureFor = (p) => {
  const parts = p.replace(/\?.*$/, "").split("/");
  if (parts[0] === "pic") return pictures.get(parts[1]);
  if (parts[0] === "note") return pictures.get(`note/${parts[1]}`);
  if (parts[0] === "drive") return pictures.get(p);
  // A contact's face: the portrait numbered as the card is (c13, face/13).
  if (parts[0] === "photo") return pictures.get(`face/${parts[1].replace(/^c/, "")}`);
  return null;
};

/** A window of the app on the demo library, at `scale`, in `scheme`. */
async function open({ scheme = "light", scale = 2, width = WIN.width, height = WIN.height, screen = "library", cursor = false, storage = {} } = {}) {
  const ctx = await browser.newContext({ viewport: { width, height }, deviceScaleFactor: scale, colorScheme: scheme,
    timezoneId: TZ, locale: "en-US", reducedMotion: "no-preference" });
  await ctx.clock.setFixedTime(NOW);
  await ctx.route(`${PICS}/**`, (route) => {
    const body = pictureFor(decodeURIComponent(route.request().url().slice(PICS.length + 1)));
    return body ? route.fulfill({ body, contentType: "image/jpeg" }) : route.fulfill({ status: 404, body: "" });
  });
  await ctx.addInitScript(installDemo, { picBase: PICS, tz: TZ, library: LIBRARY });
  await ctx.addInitScript(installStub, { screen, services: SERVICES, twoFactor: null, notesFixture: true, calendarFixture: true,
    remindersFixture: true, contactsFixture: true, driveFixture: true, demoFixture: true });
  await ctx.addInitScript((s) => { try { for (const [k, v] of Object.entries(s)) localStorage.setItem(k, v); } catch {} }, storage);
  if (cursor) await ctx.addInitScript(fakeCursor);
  const page = await ctx.newPage();
  page.on("pageerror", (e) => console.log(`page error: ${e.message}`));
  await page.goto(`${URL_}/`, { waitUntil: "networkidle" });
  await page.waitForTimeout(600);
  return { ctx, page };
}

/** Until every picture in view is drawn and the fonts are in: never a half-loaded still. */
async function settled(page, ms = 400) {
  await page.evaluate(() => document.fonts.ready);
  await page.waitForFunction(() => [...document.querySelectorAll("img")].every((i) => {
    const r = i.getBoundingClientRect();
    return r.width === 0 || r.bottom < 0 || r.top > innerHeight || (i.complete && i.naturalWidth > 0);
  }), null, { timeout: 15000 }).catch(() => console.log("some pictures still loading"));
  await page.waitForTimeout(ms);
}

/** A pointer drawn into the page, for the animation: a screencast shows no cursor of its own. */
function fakeCursor() {
  addEventListener("DOMContentLoaded", () => {
    const c = document.createElement("div");
    c.innerHTML = `<svg width="22" height="28" viewBox="0 0 22 28"><path d="M2 2v20l5.2-4.6 3.4 8 3.6-1.6-3.4-7.7H18z" fill="#111" stroke="#fff" stroke-width="1.6" stroke-linejoin="round"/></svg>`;
    Object.assign(c.style, { position: "fixed", left: "0", top: "0", zIndex: 2147483647, pointerEvents: "none", transform: "translate(-200px,-200px)", filter: "drop-shadow(0 1px 1.5px rgba(0,0,0,.35))" });
    document.body.appendChild(c);
    addEventListener("mousemove", (e) => { c.style.transform = `translate(${e.clientX - 2}px,${e.clientY - 2}px)`; }, true);
  });
}

// ---- Scenes: each leaves the window as the screenshot should show it.
const SCENES = {
  notes: async (page) => {
    await openScreen(page, "Notes");
    await page.locator(".note-rows").getByText("Cabin weekend").first().click();
    await page.waitForTimeout(900);
  },
  reminders: async (page) => {
    await openScreen(page, "Reminders");
    await page.waitForTimeout(500);
    await page.locator(".rx").getByText("Cabin Weekend", { exact: true }).first().click();
    await page.waitForTimeout(700);
  },
  calendar: async (page) => {
    await openScreen(page, "Calendar");
    await page.getByRole("button", { name: "Week", exact: true }).click();
    await page.waitForTimeout(500);
    // The working day in view.
    await page.evaluate(() => {
      const s = [...document.querySelectorAll(".cx *")].find((e) => e.scrollHeight > e.clientHeight + 200 && getComputedStyle(e).overflowY !== "visible");
      if (s) s.scrollTop = s.scrollHeight * (7.6 / 24);
    });
    await page.waitForTimeout(400);
  },
  contacts: async (page) => {
    await openScreen(page, "Contacts");
    await page.locator(".contacts").getByText("Maya Chen", { exact: true }).first().click();
    await page.waitForTimeout(700);
  },
  drive: async (page) => {
    await openScreen(page, "Drive");
    await page.waitForTimeout(400);
    await page.locator(".drive").getByText("Summer at the Lake", { exact: true }).first().dblclick();
    await page.waitForTimeout(1200);
    await page.mouse.click(5, WIN.height - 5);
  },
  photos: async (page) => {
    await openScreen(page, "Photos");
    await page.waitForTimeout(1800);
    await settled(page, 3000);
  },
  settings: async (page) => {
    await openSettings(page);
    await page.waitForSelector(".settings");
    await page.waitForTimeout(800);
  },
};

/** The window, framed: rounded, shadowed, on a soft wash of Skykeep's blue. */
async function frame(rawPng, scheme, { pad = PAD } = {}) {
  const meta = await sizeOf(rawPng);
  const scale = 2;
  const w = meta.w / scale, h = meta.h / scale;
  const page = await browser.newPage({ viewport: { width: w + pad[0] * 2, height: h + pad[1] * 2 }, deviceScaleFactor: scale });
  await page.setContent(frameHtml(scheme, w, h, pad, `data:image/png;base64,${fs.readFileSync(rawPng).toString("base64")}`));
  await page.waitForLoadState("load");
  const out = rawPng.replace(/\.raw\.png$/, ".framed.png");
  await page.screenshot({ path: out, omitBackground: false });
  await page.close();
  return out;
}

function frameHtml(scheme, w, h, pad, src, { hole = false } = {}) {
  const dark = scheme === "dark";
  const bg = dark
    ? `radial-gradient(70% 90% at 88% 100%, rgba(52,128,216,.42), transparent 70%),
       radial-gradient(60% 70% at 8% 0%, rgba(94,185,242,.20), transparent 70%),
       linear-gradient(160deg, #0f2350 0%, #0a1838 55%, #07112a 100%)`
    : `radial-gradient(70% 90% at 90% 100%, rgba(52,128,216,.30), transparent 70%),
       radial-gradient(60% 70% at 6% 0%, rgba(94,185,242,.28), transparent 70%),
       linear-gradient(160deg, #f1f7fe 0%, #dcebfb 55%, #c3dcf7 100%)`;
  const shadow = dark
    ? "0 0 0 1px rgba(255,255,255,.10), 0 2px 6px rgba(0,0,0,.35), 0 24px 48px -12px rgba(0,0,0,.55), 0 60px 110px -40px rgba(0,10,40,.8)"
    : "0 0 0 1px rgba(16,40,90,.10), 0 2px 6px rgba(16,40,90,.08), 0 24px 48px -14px rgba(18,62,157,.28), 0 60px 110px -40px rgba(18,62,157,.40)";
  return `<!doctype html><html><head><style>
    html,body{margin:0;width:${w + pad[0] * 2}px;height:${h + pad[1] * 2}px;background:${hole ? "transparent" : bg};overflow:hidden}
    .win{position:absolute;left:${pad[0]}px;top:${pad[1]}px;width:${w}px;height:${h}px;border-radius:12px;overflow:hidden;
      box-shadow:${shadow};background:${hole ? "#fff" : dark ? "#1c1c1e" : "#f5f5f7"}}
    .win img{display:block;width:${w}px;height:${h}px}
  </style></head><body><div class="win">${src ? `<img src="${src}">` : ""}</div></body></html>`;
}

async function sizeOf(png) {
  const [w, h] = execFileSync("magick", ["identify", "-format", "%w %h", png]).toString().split(" ").map(Number);
  return { w, h };
}

/** A WebP for the README, WIDTH px wide (sharp at the README's 880 on a 2x screen), stripped.
 *  At quality 90 the interface's text is indistinguishable from the PNG at half the bytes,
 *  and the README loads that much sooner. */
function publish(framed, name) {
  const webp = path.join(OUT, `${name}.webp`);
  sh("magick", [framed, "-strip", "-filter", "Lanczos", "-resize", `${WIDTH}x`, "-quality", "90", "-define", "webp:method=6", webp]);
  console.log(`${path.relative(ROOT, webp)}  ${Math.round(fs.statSync(webp).size / 1024)} KB`);
}

const has = (cmd) => { try { execFileSync("sh", ["-c", `command -v ${cmd}`], { stdio: "ignore" }); return true; } catch { return false; } };

// ---- Stills. Those in STORE are also the app stores' screenshots, Skykeep's own window first.
const STORE = ["settings", "notes", "reminders", "calendar", "photos"];
const stills = { notes: ["light", "dark"], reminders: ["light", "dark"], calendar: ["light", "dark"], contacts: ["light", "dark"],
  drive: ["light", "dark"], photos: ["light", "dark"], settings: ["light"] };
for (const [scene, schemes] of Object.entries(stills)) {
  if (!want(scene)) continue;
  for (const scheme of schemes) {
    // Skykeep's own window is shown whole: the launcher and the three cards under it.
    const tall = scene === "settings" ? { height: 872 } : {};
    const { ctx, page } = await open({ scheme, ...tall, storage: { "quince.drive.view": "grid" } });
    await SCENES[scene](page);
    await settled(page);
    const raw = path.join(WORK, `${scene}-${scheme}.raw.png`);
    await page.screenshot({ path: raw });
    await ctx.close();
    const framed = await frame(raw, scheme);
    // Skykeep's own window is in the README's animation; its still is for the stores only.
    if (scene !== "settings") publish(framed, `${scene}-${scheme}`);
    // The app stores' screenshots (the metainfo's), lossless, light only.
    if (scheme === "light" && STORE.includes(scene)) {
      fs.mkdirSync(path.join(OUT, "screenshots"), { recursive: true });
      sh("magick", [framed, "-strip", "-filter", "Lanczos", "-resize", `${WIDTH}x`, path.join(OUT, "screenshots", `${scene === "settings" ? "skykeep" : scene}-light.png`)]);
    }
  }
}
// ---- The hero: Skykeep's window and its launcher; each app opens from its tile in a window of
// its own, over Skykeep's, as on a desktop. Every window is the real interface on the demo
// library, each at its own address (`?app=notes`), laid out on a stage page.
// Made in light and dark: the README shows the one that matches the reader's GitHub.
for (const scheme of want("hero") ? ["light", "dark"] : []) {
  // A small stage the windows fill, drawn at 2x: the README shows it at 880 px, so its text
  // reads at the size it has on a screen.
  const W = 1040, H = 640, SCALE = 2;
  // One window at a time, whole and centred: Skykeep's, then the app its tile opens.
  const SKY = { x: (W - 720) / 2, y: 0, w: 720, h: 0 };
  // An app fills the frame, as a window opened to work in does; Skykeep's stays its own size.
  const APP = { x: 20, y: 18, w: W - 40, h: H - 36 };
  const SHOWN = ["notes", "calendar", "photos"];
  const ctx = await browser.newContext({ viewport: { width: W, height: H }, deviceScaleFactor: SCALE, colorScheme: scheme,
    timezoneId: TZ, locale: "en-US", reducedMotion: "no-preference" });
  await ctx.clock.setFixedTime(NOW);
  await ctx.route(`${PICS}/**`, (route) => {
    const body = pictureFor(decodeURIComponent(route.request().url().slice(PICS.length + 1)));
    return body ? route.fulfill({ body, contentType: "image/jpeg" }) : route.fulfill({ status: 404, body: "" });
  });
  await ctx.addInitScript(installDemo, { picBase: PICS, tz: TZ, library: LIBRARY });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true, calendarFixture: true,
    remindersFixture: true, contactsFixture: true, driveFixture: true, demoFixture: true });
  // The stage: the backdrop the stills sit on, Skykeep's window, and each app's window, put away
  // until its tile is clicked. A pointer is drawn on top, since a screencast shows none.
  await ctx.route(`${URL_}/__stage`, (route) => route.fulfill({ contentType: "text/html", body: `<!doctype html><html><head><style>
    ${frameHtml(scheme, W, H, [0, 0], "").match(/<style>([\s\S]*)<\/style>/)[1].replace(/\.win\{[\s\S]*$/, "")}
    .w{position:absolute;border:0;border-radius:12px;overflow:hidden;background:${scheme === "dark" ? "#1c1c1e" : "#f5f5f7"};
      box-shadow:${scheme === "dark"
        ? "0 0 0 1px rgba(255,255,255,.10),0 2px 6px rgba(0,0,0,.35),0 24px 48px -12px rgba(0,0,0,.55),0 60px 110px -40px rgba(0,10,40,.8)"
        : "0 0 0 1px rgba(16,40,90,.10),0 2px 6px rgba(16,40,90,.08),0 24px 48px -14px rgba(18,62,157,.28),0 60px 110px -40px rgba(18,62,157,.40)"}}
    .w iframe{display:block;width:100%;height:100%;border:0}
    /* Put away but still painted, so a window shows on the very frame its tile is clicked. */
    .away{opacity:0;pointer-events:none}
    #cur{position:fixed;left:0;top:0;z-index:99;pointer-events:none;filter:drop-shadow(0 1px 1.5px rgba(0,0,0,.35))}
  </style></head><body>
    <div class="w" id="sky" style="left:${SKY.x}px;top:${SKY.y}px;width:${SKY.w}px;height:${H - 2 * SKY.y}px;z-index:1"><iframe src="/"></iframe></div>
    ${SHOWN.map((k) => `<div class="w away" id="${k}" style="left:${APP.x}px;top:${APP.y}px;width:${APP.w}px;height:${APP.h}px"><iframe src="/?app=${k}"></iframe></div>`).join("")}
    <svg id="cur" width="22" height="28" viewBox="0 0 22 28" style="transform:translate(${W - 24}px,${H - 24}px)"><path d="M2 2v20l5.2-4.6 3.4 8 3.6-1.6-3.4-7.7H18z" fill="#111" stroke="#fff" stroke-width="1.6" stroke-linejoin="round"/></svg>
  </body></html>` }));
  const page = await ctx.newPage();
  page.on("pageerror", (e) => console.log(`page error: ${e.message}`));
  await page.goto(`${URL_}/__stage`, { waitUntil: "networkidle" });
  const sky = page.frameLocator("#sky iframe");
  const app = (k) => page.frameLocator(`#${k} iframe`);
  await sky.locator(".launch-tile").first().waitFor();
  // Skykeep's window ends just under its launcher: a whole window, never one cut through a card.
  SKY.h = Math.ceil(await sky.locator(".launcher").evaluate((el) => el.getBoundingClientRect().bottom)) + 26;
  SKY.y = Math.round((H - SKY.h) / 2);
  await page.evaluate(([h, y]) => { const w = document.getElementById("sky"); w.style.height = `${h}px`; w.style.top = `${y}px`; }, [SKY.h, SKY.y]);
  for (const k of SHOWN) await app(k).locator({ notes: ".notes", calendar: ".cx", photos: ".px" }[k]).first().waitFor();
  // Calendar's week, the working day in view, before it is ever shown.
  await app("calendar").getByRole("button", { name: "Week", exact: true }).evaluate((b) => b.click());
  await page.frame({ url: /app=calendar/ }).evaluate(() => {
    const s = [...document.querySelectorAll(".cx *")].find((e) => e.scrollHeight > e.clientHeight + 200 && getComputedStyle(e).overflowY !== "visible");
    if (s) s.scrollTop = s.scrollHeight * (7.6 / 24);
  });
  // Every picture in every window drawn before the recording starts: a window shown on the frame
  // its tile is clicked must never show a grey box where its picture will be.
  for (const f of page.frames()) {
    await f.evaluate(() => document.fonts.ready).catch(() => {});
    await f.waitForFunction(() => [...document.querySelectorAll("img")].every((i) => i.complete && (i.naturalWidth > 0 || !i.currentSrc)),
      null, { timeout: 20000 }).catch(() => console.log("some pictures still loading"));
  }
  // Each app shown once before recording: a note's pictures load when they come into view, so
  // a window first shown on camera would show a grey box where its picture lands.
  for (const k of SHOWN) {
    await page.evaluate((id) => { for (const w of document.querySelectorAll(".w")) w.classList.toggle("away", w.id !== id); }, k);
    await page.waitForTimeout(1500);
  }
  await page.evaluate(() => { for (const w of document.querySelectorAll(".w")) w.classList.toggle("away", w.id !== "sky"); });
  await page.waitForTimeout(2500);

  const dir = path.join(WORK, `hero-${scheme}`);
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(dir, { recursive: true });
  const frames = [];
  const frameFile = (i) => path.join(dir, `f${String(i).padStart(4, "0")}.png`);
  const cdp = await ctx.newCDPSession(page);
  cdp.on("Page.screencastFrame", async (f) => {
    fs.writeFileSync(frameFile(frames.length), Buffer.from(f.data, "base64"));
    frames.push({ t: f.metadata.timestamp });
    await cdp.send("Page.screencastFrameAck", { sessionId: f.sessionId }).catch(() => {});
  });
  await cdp.send("Page.startScreencast", { format: "png", everyNthFrame: 1, maxWidth: W * SCALE, maxHeight: H * SCALE });
  let at = { x: W - 24, y: H - 24 };
  // The drawn pointer glides in the page at the screen's own rate; the real one follows it in
  // a few steps, for the hover each tile and row shows.
  const glide = async (loc, { click = "single", hold = 0 } = {}) => {
    const box = await loc.boundingBox();
    const to = { x: box.x + box.width / 2, y: box.y + box.height / 2 };
    const ms = Math.min(620, 260 + Math.hypot(to.x - at.x, to.y - at.y) * 0.45);
    await Promise.all([
      page.evaluate(([a, b, ms]) => new Promise((done) => {
        const cur = document.getElementById("cur");
        const t0 = performance.now();
        const step = (now) => {
          const k = Math.min(1, (now - t0) / ms), e = 1 - Math.pow(1 - k, 3);
          cur.style.transform = `translate(${a.x + (b.x - a.x) * e - 2}px,${a.y + (b.y - a.y) * e - 2}px)`;
          if (k < 1) requestAnimationFrame(step); else done();
        };
        requestAnimationFrame(step);
      }), [at, to, ms]),
      page.mouse.move(to.x, to.y, { steps: 6 }),
    ]);
    at = to;
    await page.waitForTimeout(70);
    if (click === "single") await page.mouse.click(to.x, to.y);
    if (click === "double") await page.mouse.dblclick(to.x, to.y);
    if (hold) await page.waitForTimeout(hold);
  };
  let z = 1;
  // Only the window asked for shows: an app's window, opened from its tile, stands alone.
  const raise = (id) => page.evaluate(([id, z]) => {
    for (const w of document.querySelectorAll(".w")) w.classList.toggle("away", w.id !== id);
    document.getElementById(id).style.zIndex = z;
  }, [id, ++z]);
  await page.waitForTimeout(1300);
  const names = { notes: "Notes", calendar: "Calendar", photos: "Photos" };
  for (const [i, k] of SHOWN.entries()) {
    if (i > 0) {
      // Back to Skykeep's window, as its icon in the dock or tray brings it.
      await raise("sky");
      await page.waitForTimeout(700);
    }
    await glide(sky.locator(".launch-tile", { hasText: names[k] }), { hold: 0 });
    await raise(k);
    if (k === "notes") {
      await page.waitForTimeout(700);
      // Another note than the one open, so the click is seen to open it.
      await glide(app("notes").locator(".note-rows").getByText("Sourdough starter").first(), { hold: 2000 });
    } else {
      await page.waitForTimeout(k === "photos" ? 2600 : 2000);
    }
  }
  await cdp.send("Page.stopScreencast");
  await ctx.close();

  // Frames straight to animated WebP, in full colour: through a GIF's 192-colour palette first,
  // the gradients and pictures came out banded and dithered.
  const end = frames[frames.length - 1].t + 2.4;
  const hold = (i) => Math.min(2.4, Math.max(0.004, (frames[i + 1]?.t ?? end) - frames[i].t));
  let list = "";
  // A still stretch plays for at most 2.4 s: long enough to read an app's window, never the
  // waits the recording itself had (the screencast sends no frame while nothing moves).
  frames.forEach((f, i) => { list += `file '${frameFile(i)}'\nduration ${hold(i).toFixed(3)}\n`; });
  list += `file '${frameFile(frames.length - 1)}'\n`;
  fs.writeFileSync(path.join(dir, "list.txt"), list);
  const webp = path.join(OUT, `hero-${scheme}.webp`);
  sh("ffmpeg", ["-y", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i", path.join(dir, "list.txt"),
    "-vf", `fps=24,scale=${WIDTH}:-2:flags=lanczos`, "-c:v", "libwebp_anim", "-lossless", "0", "-q:v", "82",
    "-compression_level", "6", "-loop", "0", webp]);
  console.log(`${path.relative(ROOT, webp)}  ${Math.round(fs.statSync(webp).size / 1024)} KB`);
}

// ---- Posters: a post image for Reddit and X (1600x900), GitHub's social preview (1280x640)
// and a wide banner for listings (1500x500). The six apps' own windows, whole, in a grid, each
// with its icon and name and alternately light and dark, under the headline; the brand's sky
// in Instrument Sans. Written to POSTERS, outside the repository: they are uploaded by hand.
if (want("posters")) {
  const DIR = process.env.POSTERS;
  if (!DIR) throw new Error("posters: set POSTERS to the folder they go in");
  const FONT = process.env.POSTER_FONT; // Instrument Sans, a variable .ttf
  fs.mkdirSync(DIR, { recursive: true });
  const b64 = (f) => fs.readFileSync(f).toString("base64");
  const LOGO = b64(path.join(ROOT, "packaging/icons/skykeep-1024.png"));
  const APPS = [["notes", "Notes", "light"], ["reminders", "Reminders", "dark"], ["calendar", "Calendar", "light"],
    ["contacts", "Contacts", "dark"], ["drive", "iCloud Drive", "light"], ["photos", "Photos", "dark"]];
  // Each app's window alone, as its launcher opens it.
  const shots = {};
  for (const [scene, , scheme] of APPS) {
    const { ctx, page } = await open({ scheme, storage: { "quince.drive.view": "grid" } });
    await SCENES[scene](page);
    await settled(page);
    const raw = path.join(WORK, `poster-${scene}-${scheme}.png`);
    await page.screenshot({ path: raw });
    await ctx.close();
    shots[scene] = b64(raw);
  }
  const icon = (k) => b64(path.join(ROOT, `packaging/icons/apps/${k}-256.png`));
  const GH = "github.com/YashwanthKamireddi/Skykeep";
  // Window sizes keep the screenshots' own shape (1200 by 760).
  const KINDS = {
    post: { W: 1600, H: 900, layout: "top", pad: 98, head: 58, h1: 74, p: 18, gap: 36, rowGap: 22, chip: 18 },
    social: { W: 1280, H: 640, layout: "top", inline: true, pad: 44, head: 26, h1: 52, p: 16, gap: 15, rowGap: 14, chip: 14 },
    banner: { W: 1500, H: 500, layout: "side", pad: 56, h1: 50, p: 18, gap: 14, rowGap: 30, chip: 12, text: 470 },
  };
  for (const [name, k] of Object.entries(KINDS)) {
    const side = k.layout === "side";
    const gridW = side ? k.W - k.pad * 2 - k.text - 40 : k.W - k.pad * 2;
    const winW = Math.floor((gridW - k.gap * 2) / 3);
    const winH = Math.round((winW * 760) / 1200);
    const cards = APPS.map(([key, label, scheme]) => `<figure>
        <img class="w ${scheme}" src="data:image/png;base64,${shots[key]}" style="width:${winW}px;height:${winH}px">
        <figcaption><span class="chip"><img src="data:image/png;base64,${icon(key)}" alt="">${label}</span><span class="mode">${scheme === "dark" ? "Dark" : "Light"}</span></figcaption>
      </figure>`).join("");
    const words = side
      ? `<div class="words"><div class="brand"><img src="data:image/png;base64,${LOGO}" alt="">Skykeep</div>
           <h1>Six apps.<br>Your whole iCloud.</h1>
           <p>Real desktop apps for Linux and Windows,<br>kept on your computer and in sync<br>with your iPhone. <b>Free and open source.</b></p></div>`
      : k.inline
      ? `<header><div class="row"><div class="brand big"><img src="data:image/png;base64,${LOGO}" alt=""><h1>Six apps. Your whole iCloud.</h1></div>
           <p><b>Skykeep</b> brings it to Linux and Windows,<br>in sync with your iPhone. <b>Free and open source.</b></p></div></header>`
      : `<header><div class="brand"><img src="data:image/png;base64,${LOGO}" alt="">Skykeep</div>
           <div class="row"><h1>Six apps. Your whole iCloud.</h1>
           <p>Real desktop apps for Linux and Windows,<br>kept on your computer and in sync with your iPhone.<br><b>Free and open source.</b> ${GH}</p></div></header>`;
    const html = `<!doctype html><html><head><meta charset="utf-8"><style>
      ${FONT ? `@font-face{font-family:IS;src:url(data:font/ttf;base64,${b64(FONT)});font-weight:400 700;font-stretch:75% 100%}` : ""}
      *{margin:0;padding:0;box-sizing:border-box}
      html,body{width:${k.W}px;height:${k.H}px;overflow:hidden}
      body{position:relative;font-family:IS,system-ui,sans-serif;color:#fff;
        background:radial-gradient(${k.W * 0.4}px ${k.H * 0.26}px at 82% 12%,rgba(255,255,255,.20),transparent 70%),
          radial-gradient(${k.W * 0.5}px ${k.H * 0.3}px at 10% 100%,rgba(255,255,255,.30),transparent 70%),
          radial-gradient(${k.W * 0.45}px ${k.H * 0.28}px at 72% 104%,rgba(255,255,255,.26),transparent 70%),
          linear-gradient(180deg,#1F7FE6 0%,#2F9BFF 46%,#78BEFF 82%,#A9D7FF 100%)}
      .brand{display:flex;align-items:center;gap:${Math.round(k.p * 0.7)}px;font-weight:660;font-size:${Math.round(k.p * 1.45)}px;letter-spacing:-.01em}
      .brand img{width:${Math.round(k.p * 2.3)}px;height:${Math.round(k.p * 2.3)}px;filter:drop-shadow(0 4px 12px rgba(11,42,74,.28))}
      h1{font-weight:660;font-stretch:80%;font-size:${k.h1}px;line-height:1;letter-spacing:-.028em;text-shadow:0 8px 30px rgba(11,42,74,.2)}
      p{font-size:${k.p}px;line-height:1.38;font-weight:480;color:rgba(255,255,255,.95)}
      p b{font-weight:660;color:#fff}
      header{position:absolute;left:${k.pad}px;right:${k.pad}px;top:${k.head}px}
      header .brand{margin-bottom:${Math.round(k.p * 0.6)}px}
      header .row{display:flex;justify-content:space-between;align-items:flex-end;gap:24px}
      header .row .brand.big{margin:0;align-items:center}
      header .row .brand.big img{width:${Math.round(k.h1 * 1.05)}px;height:${Math.round(k.h1 * 1.05)}px}
      header p{text-align:right;white-space:nowrap}
      header h1{white-space:nowrap}
      .words{position:absolute;left:${k.pad}px;top:50%;transform:translateY(-50%);width:${k.text}px}
      .words .brand{margin-bottom:${Math.round(k.p * 1.1)}px}
      .words p{margin-top:${Math.round(k.p * 1.1)}px}
      .g{position:absolute;${side ? `right:${k.pad}px;top:50%;transform:translateY(-50%)` : `left:50%;transform:translateX(-50%)`};
        display:grid;grid-template-columns:repeat(3,var(--w,${winW}px));column-gap:${k.gap}px;row-gap:${k.rowGap}px}
      .g .w{width:var(--w,${winW}px)!important;height:var(--h,${winH}px)!important}
      figure{position:relative}
      .w{display:block;border-radius:${Math.max(6, Math.round(winW / 50))}px;object-fit:cover;object-position:top left;
        box-shadow:0 0 0 1px rgba(255,255,255,.4),0 18px 40px -14px rgba(8,32,66,.55)}
      .w.dark{box-shadow:0 0 0 1px rgba(255,255,255,.18),0 18px 40px -14px rgba(8,32,66,.6)}
      figcaption{display:flex;justify-content:space-between;align-items:center;margin-top:${Math.round(k.chip * 0.55)}px}
      .chip{display:inline-flex;align-items:center;gap:${Math.round(k.chip * 0.45)}px;padding:${Math.round(k.chip * 0.28)}px ${Math.round(k.chip * 0.8)}px ${Math.round(k.chip * 0.28)}px ${Math.round(k.chip * 0.35)}px;border-radius:999px;
        background:rgba(255,255,255,.94);color:#0B2A4A;font-weight:660;font-size:${k.chip}px;box-shadow:0 6px 18px -6px rgba(8,32,66,.5)}
      .chip img{width:${Math.round(k.chip * 1.5)}px;height:${Math.round(k.chip * 1.5)}px}
      .mode{font-size:${Math.round(k.chip * 0.85)}px;font-weight:520;color:rgba(255,255,255,.85)}
    </style></head><body>${words}<div class="g">${cards}</div></body></html>`;
    const page = await browser.newPage({ viewport: { width: k.W, height: k.H }, deviceScaleFactor: 2 });
    await page.setContent(html, { waitUntil: "load" });
    await page.evaluate(() => document.fonts.ready);
    // Under a headline, the windows fill the room left below it: as wide as the width allows,
    // no taller than the height does. Measured, so no window ever lies over the words.
    if (!side) await page.evaluate(([W, H, pad, gap, rowGap, maxW]) => {
      const top = document.querySelector("header").getBoundingClientRect().bottom + pad * 0.42;
      const cap = document.querySelector("figcaption").getBoundingClientRect().height + 8;
      const room = H - top - pad * 0.36 - rowGap - cap * 2;
      const w = Math.floor(Math.min(maxW, ((room / 2) * 1200) / 760));
      const g = document.querySelector(".g");
      g.style.top = `${top}px`;
      g.style.setProperty("--w", `${w}px`);
      g.style.setProperty("--h", `${Math.round((w * 760) / 1200)}px`);
    }, [k.W, k.H, k.pad, k.gap, k.rowGap, winW]);
    await page.waitForTimeout(300);
    const raw = path.join(WORK, `poster-${name}.png`);
    await page.screenshot({ path: raw });
    await page.close();
    const out = path.join(DIR, `${name}-${k.W}x${k.H}.png`);
    // Drawn at 2x, saved at the size each place asks for, so the text stays crisp.
    sh("magick", [raw, "-strip", "-filter", "Lanczos", "-resize", `${k.W}x${k.H}`, out]);
    console.log(`${out}  ${Math.round(fs.statSync(out).size / 1024)} KB`);
  }
}

await browser.close();
process.exit(0);
