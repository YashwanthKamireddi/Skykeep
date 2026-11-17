// Coming back to a screen is coming back to it as it was: nothing on it plays again. Screens
// stay loaded out of sight, and a CSS animation starts over whenever its element comes back
// into view — every face in Contacts faded in anew, every ticked reminder sprang in again.
// Also: a toggle whose glyph says its state wears no fill that reads as a stuck hover.
import zlib from "node:zlib";
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const png = (() => {
  const { deflateSync, crc32 } = zlib;
  const w = 64, h = 64, raw = Buffer.alloc((w * 3 + 1) * h, 120);
  for (let y = 0; y < h; y++) raw[y * (w * 3 + 1)] = 0;
  const chunk = (t, d) => { const l = Buffer.alloc(4); l.writeUInt32BE(d.length); const td = Buffer.concat([Buffer.from(t), d]); const c = Buffer.alloc(4); c.writeUInt32BE(crc32(td)); return Buffer.concat([l, td, c]); };
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 2;
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", ihdr), chunk("IDAT", deflateSync(raw)), chunk("IEND", Buffer.alloc(0))]);
})();
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(() => {
  window.__picBase = "http://pics.test";
  // Every contact with a picture of their own.
  const add = () => {
    const internals = window.__TAURI_INTERNALS__;
    if (!internals || internals.__faces) return;
    internals.__faces = true;
    const invoke = internals.invoke;
    internals.invoke = async (cmd, args) => {
      const answer = await invoke(cmd, args);
      return cmd === "contacts_list" && Array.isArray(answer) ? answer.map((c) => ({ ...c, photoUrl: "https://example.test/p" })) : answer;
    };
  };
  window.__afterStub = add;
});
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, contactsFixture: true, remindersFixture: true, notesFixture: true });
await ctx.addInitScript(() => window.__afterStub?.());
await ctx.route("http://pics.test/**", (r) => r.fulfill({ status: 200, contentType: "image/png", body: png }));
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const playing = (sel) => page.evaluate((s) => [...document.querySelectorAll(s)].flatMap((e) => e.getAnimations({ subtree: true })).filter((a) => a.playState === "running").length, sel);

await openScreen(page, "Contacts");
// Every face drawn: as many pictures loaded as there are faces.
await page.waitForFunction(() => {
  const f = [...document.querySelectorAll(".face-photo")];
  return f.length > 0 && f.every((i) => !(i instanceof HTMLImageElement) || (i.complete && i.naturalWidth > 0));
}, null, { timeout: 8000 }).catch(() => {});
const faces = await page.locator(".face-photo").count();
ok(faces > 0, `contacts show their pictures (${faces})`);
ok((await playing(".face-photo")) === 0, "drawn without a fade");
await openScreen(page, "Notes");
await page.waitForTimeout(300);
await openScreen(page, "Contacts");
ok((await playing(".face-photo")) === 0, "back to Contacts: no face fades in again");

// Reminders: a tick's fill fades in on the press, and only then (drawn by the page, frame by
// frame — lib/fade — so it is measured as the fill's opacity rising, not as an animation).
await openScreen(page, "Reminders");
await page.locator(".rx-list", { hasText: "Daily" }).click();
await page.evaluate(() => { window.__doneDelay = 4000; });
const tick = page.locator(".rx-row", { has: page.getByLabel("Rename Pick up the parcel") }).locator(".rx-tick");
await tick.waitFor();
// This row's own fill, each frame from the press until it is whole (or two seconds): another
// row ticked before stays whole throughout and is not this one.
const rising = page.evaluate(() => new Promise((done) => {
  const seen = [];
  const t0 = performance.now();
  const sample = () => {
    const row = [...document.querySelectorAll(".rx-row")].find((r) => r.querySelector('[aria-label="Rename Pick up the parcel"]'));
    const fill = row?.querySelector(".rx-tick-fill");
    if (fill) seen.push(Number(getComputedStyle(fill).opacity));
    if (performance.now() - t0 < 2000 && !(seen.some((o) => o < 0.5) && seen.at(-1) >= 0.9)) requestAnimationFrame(sample); else done(seen);
  };
  requestAnimationFrame(sample);
}));
await tick.click();
const seen = await rising;
// Faded in: from faint to (nearly) whole over a few frames, never whole at once.
const low = seen.findIndex((o) => o < 0.5);
ok(low >= 0 && seen.slice(low).some((o) => o > 0 && o < 0.9) && seen.slice(low).some((o) => o >= 0.9), `a tick pressed fades in  [${seen.map((o) => o.toFixed(2)).join(" ")}]`);
await page.waitForTimeout(400);
await openScreen(page, "Notes");
await page.waitForTimeout(200);
await openScreen(page, "Reminders");
const after = await page.evaluate(() => [...document.querySelectorAll(".rx-tick-fill")].map((f) => Number(getComputedStyle(f).opacity)));
ok((await playing(".rx-tick")) === 0 && after.every((o) => o === 0 || o === 1), "back to Reminders: no tick fades in again");

// Photos: the square/whole toggle wears no fill until it is hovered.
await openScreen(page, "Photos");
await page.waitForTimeout(600);
const toggle = page.locator(".px-cap.is-glyph-state");
if (await toggle.count()) {
  await page.mouse.move(5, 5);
  await toggle.click();
  await page.mouse.move(5, 5);
  await page.waitForTimeout(200);
  const bg = await toggle.evaluate((e) => getComputedStyle(e).backgroundColor);
  ok(bg === "rgba(0, 0, 0, 0)", `the square/whole toggle shows its state by its glyph, no fill  (${bg})`);
} else ok(true, "no photos toggle in this fixture");
await b.close();
process.exitCode = failures ? 1 : 0;
