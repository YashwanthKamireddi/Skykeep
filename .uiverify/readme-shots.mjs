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
import { SERVICES, installStub, openScreen } from "./stub.mjs";
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
    await page.locator(".nav-item", { hasText: "Photos" }).click();
    await page.waitForTimeout(1800);
    await settled(page, 3000);
  },
  settings: async (page) => {
    await page.locator(".sidebar-account").click();
    await page.waitForSelector(".settings");
    await page.waitForTimeout(800);
  },
  // Back Up iCloud part way through: Photos and iCloud Drive into a drive of the person's own.
  backup: async (page) => {
    await page.evaluate(() => {
      window.__backup = { folder: "/media/Backup Drive", hidden: false, drive: true, driveRunning: true, driveNote: null,
        copied: 1486, total: 2140, running: true, problem: null, photosBytes: 5_120_000_000, photosBytesTotal: 7_340_000_000,
        driveFiles: 2930, driveFilesTotal: 4120, driveBytes: 2_870_000_000, driveBytesTotal: 4_100_000_000 };
    });
    await page.locator(".sidebar-account").click();
    await page.waitForSelector(".settings");
    await page.locator(".backup").first().scrollIntoViewIfNeeded();
    await page.locator(".backup").first().evaluate((el) => el.scrollIntoView({ block: "center" }));
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

// ---- Stills.
const stills = { notes: ["light", "dark"], reminders: ["light", "dark"], calendar: ["light", "dark"], contacts: ["light", "dark"],
  drive: ["light", "dark"], photos: ["light", "dark"], settings: ["light", "dark"], backup: ["light", "dark"] };
for (const [scene, schemes] of Object.entries(stills)) {
  if (!want(scene)) continue;
  for (const scheme of schemes) {
    const { ctx, page } = await open({ scheme, storage: { "quince.drive.view": "grid" } });
    await SCENES[scene](page);
    await settled(page);
    const raw = path.join(WORK, `${scene}-${scheme}.raw.png`);
    await page.screenshot({ path: raw });
    await ctx.close();
    publish(await frame(raw, scheme), `${scene}-${scheme}`);
  }
}
if (want("signin")) {
  for (const scheme of ["light", "dark"]) {
    const { ctx, page } = await open({ scheme, screen: "signin" });
    await page.getByLabel("Apple Account").fill("jamie.rivera@example.com");
    await page.getByLabel("Password").fill("demo-password");
    await page.locator("body").click({ position: { x: 5, y: 5 } });
    await page.waitForTimeout(400);
    const raw = path.join(WORK, `signin-${scheme}.raw.png`);
    await page.screenshot({ path: raw });
    await ctx.close();
    publish(await frame(raw, scheme), `signin-${scheme}`);
  }
}

// ---- The hero: switch apps, tick a reminder, watch a note arrive from the phone.
if (want("hero")) {
  const W = 1120, H = 700, PAD = [40, 36];
  const { ctx, page } = await open({ scheme: "light", scale: 1, width: W, height: H, cursor: true });
  await openScreen(page, "Reminders");
  await page.mouse.move(W - 40, H - 60);
  await page.waitForTimeout(500);
  // Each frame goes to disk as it arrives and only its time is kept: held in memory, a dozen
  // seconds of frames (and their copies on the way to files) filled a 16 GB machine.
  const dir = path.join(WORK, "hero");
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
  await cdp.send("Page.startScreencast", { format: "png", everyNthFrame: 1, maxWidth: W, maxHeight: H });
  const glide = async (loc, { click = "single", hold = 0, offset } = {}) => {
    const box = await loc.boundingBox();
    const x = box.x + (offset?.x ?? box.width / 2), y = box.y + (offset?.y ?? box.height / 2);
    await page.mouse.move(x, y, { steps: 18 });
    await page.waitForTimeout(160);
    if (click === "single") await page.mouse.click(x, y);
    if (click === "double") await page.mouse.dblclick(x, y);
    if (hold) await page.waitForTimeout(hold);
  };
  await page.waitForTimeout(1100);
  // Tick a reminder.
  await glide(page.getByRole("checkbox", { name: "Send the Q4 slides to Jordan" }), { hold: 1400 });
  // To Calendar, and its week.
  await glide(page.locator(".nav-item", { hasText: "Calendar" }), { hold: 600 });
  await glide(page.getByRole("button", { name: "Week", exact: true }), { hold: 1500 });
  // To Notes; a note arrives from the phone.
  await glide(page.locator(".nav-item", { hasText: "Notes" }), { hold: 900 });
  await page.mouse.move(W * 0.62, H * 0.55, { steps: 16 });
  await page.evaluate(() => { window.__demoArrive(); const now = Math.floor(Date.now() / 1000);
    // As a real pass reports it: Notes synced now and changed — with no services named, the
    // footer read "Not synced yet" as the note arrived.
    const apps = [["notes", "Notes"], ["reminders", "Reminders"], ["calendar", "Calendar"], ["contacts", "Contacts"], ["drive", "iCloud Drive"], ["photos", "Photos"]];
    window.__emit?.("quince://synced", { services: apps.map(([service, label]) => ({ service, label, outcome: "ok", detail: null, lastSyncedAt: now })),
      asOf: now, tookMs: 5, changed: ["notes"] }); });
  await page.waitForTimeout(1200);
  await glide(page.locator(".note-rows").getByText("Farmers market").first(), { hold: 1800 });
  // The other three apps: a contact's card, a Drive folder, the photo library.
  await glide(page.locator(".nav-item", { hasText: "Contacts" }), { hold: 500 });
  await glide(page.locator(".contacts").getByText("Maya Chen", { exact: true }).first(), { hold: 1500 });
  await glide(page.locator(".nav-item", { hasText: "Drive" }), { hold: 500 });
  await glide(page.locator(".drive").getByText("Summer at the Lake", { exact: true }).first(), { click: "double", hold: 1600 });
  await glide(page.locator(".nav-item", { hasText: "Photos" }), { hold: 2400 });
  await cdp.send("Page.stopScreencast");
  await ctx.close();

  // Frames to a GIF, framed like the stills.
  const end = frames[frames.length - 1].t + 1.2;
  let list = "";
  frames.forEach((f, i) => {
    // A still stretch plays for at most 0.9 s: the screencast sends no frame while nothing
    // moves, and those gaps made the loop twice as long as anyone watches.
    list += `file '${frameFile(i)}'\nduration ${Math.min(0.9, Math.max(0.004, (frames[i + 1]?.t ?? end) - f.t)).toFixed(3)}\n`;
  });
  list += `file '${frameFile(frames.length - 1)}'\n`;
  fs.writeFileSync(path.join(dir, "list.txt"), list);
  // The backdrop and the window's rounded shape, from the same frame as the stills.
  const bgPage = await browser.newPage({ viewport: { width: W + PAD[0] * 2, height: H + PAD[1] * 2 }, deviceScaleFactor: 1 });
  await bgPage.setContent(frameHtml("light", W, H, PAD, ""));
  await bgPage.screenshot({ path: path.join(dir, "bg.png") });
  await bgPage.close();
  sh("magick", ["-size", `${W}x${H}`, "xc:black", "-fill", "white", "-draw", `roundrectangle 0,0 ${W - 1},${H - 1} 12,12`, path.join(dir, "mask.png")]);
  const gif = path.join(OUT, "hero.gif");
  // Two passes, each bounded to the clip's length: the palette first, then the frames through
  // it. In one pass the palette waited on every frame held in memory, and the looped backdrop
  // and mask never ended.
  const dur = frames.reduce((n, f, i) => n + Math.min(0.9, Math.max(0.004, (frames[i + 1]?.t ?? end) - f.t)), 0).toFixed(2);
  const inputs = ["-f", "concat", "-safe", "0", "-i", path.join(dir, "list.txt"), "-loop", "1", "-t", dur, "-i", path.join(dir, "mask.png"), "-loop", "1", "-t", dur, "-i", path.join(dir, "bg.png")];
  const comp = `[0:v]fps=15,scale=${W}:${H},format=rgba[v];[1:v]format=gray[m];[v][m]alphamerge[w];[2:v][w]overlay=${PAD[0]}:${PAD[1]}:shortest=1`;
  const palette = path.join(dir, "palette.png");
  sh("ffmpeg", ["-y", "-loglevel", "error", ...inputs, "-filter_complex", `${comp},palettegen=max_colors=192:stats_mode=diff`, "-update", "1", "-frames:v", "1", palette]);
  sh("ffmpeg", ["-y", "-loglevel", "error", ...inputs, "-i", palette, "-filter_complex",
    `${comp}[c];[c][3:v]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle`, "-t", dur, "-loop", "0", gif]);
  // The README shows it as animated WebP: the same frames at about half the GIF's size.
  const webp = path.join(OUT, "hero.webp");
  sh("ffmpeg", ["-y", "-loglevel", "error", "-i", gif, "-c:v", "libwebp_anim", "-lossless", "0", "-q:v", "80", "-compression_level", "6", "-loop", "0", webp]);
  fs.rmSync(gif);
  console.log(`${path.relative(ROOT, webp)}  ${Math.round(fs.statSync(webp).size / 1024)} KB`);
}

await browser.close();
process.exit(0);
