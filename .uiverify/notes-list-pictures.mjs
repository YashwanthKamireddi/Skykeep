// A note with a picture shows it small at the end of its row in the list, as Notes on the Mac
// does — only a picture this computer already holds (the backend answers which, nothing is
// fetched for the list), appearing once the note has drawn it, with the row keeping its height.
// A note without one keeps its whole width; a picture that cannot be had is never a broken one.
import zlib from "node:zlib";
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
// Ink on white, as Apple's fallback image of a drawing is.
const png = (() => {
  const { deflateSync, crc32 } = zlib;
  const w = 120, h = 96;
  const raw = Buffer.alloc((w * 3 + 1) * h, 255);
  for (let y = 0; y < h; y++) {
    raw[y * (w * 3 + 1)] = 0;
    if (y > 40 && y < 56) raw.fill(0, y * (w * 3 + 1) + 1, (y + 1) * (w * 3 + 1));
  }
  const chunk = (type, data) => {
    const len = Buffer.alloc(4); len.writeUInt32BE(data.length);
    const td = Buffer.concat([Buffer.from(type), data]);
    const crc = Buffer.alloc(4); crc.writeUInt32BE(crc32(td));
    return Buffer.concat([len, td, crc]);
  };
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 2;
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", ihdr), chunk("IDAT", deflateSync(raw)), chunk("IEND", Buffer.alloc(0))]);
})();
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(() => { window.__drawNote = true; window.__mediaNote = true; window.__picBase = "http://pics.test"; });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
const asked = [];
// The first note opens by itself, and its drawing with it. Fetching a drawing is what keeps it
// on this computer, so the backend holds it from the moment it is served.
let thumbsAskedBefore = null;
const page = await ctx.newPage();
await ctx.route("http://pics.test/**", async (r) => {
  const url = decodeURIComponent(r.request().url());
  asked.push(url);
  // A picture this computer no longer holds: the route has nothing for it.
  if (url.includes("note/GONE/")) return r.fulfill({ status: 404, body: "" });
  if (url.includes("note/D1/drawing") && thumbsAskedBefore === null) {
    thumbsAskedBefore = asked.filter((u) => u.includes("/thumb")).length;
    await page.evaluate(() => { window.__notePictures = { nD: "D1" }; });
  }
  return r.fulfill({ status: 200, contentType: "image/png", body: png });
});
// What the list shows at the moment the note has drawn its picture, before it asks again.
await ctx.addInitScript(() => {
  addEventListener("quince:note-picture-kept", () => {
    if (window.__thumbsAtKept !== undefined) return;
    window.__thumbsAtKept = document.querySelectorAll(".note-row-thumb").length;
    const r = document.querySelector('.note-row[data-guid="nD"]')?.getBoundingClientRect();
    window.__rowAtKept = r && { top: r.top, height: r.height, right: r.right };
  });
});
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.locator(".note-row", { hasText: "Sketch" }).first().waitFor();

const sketch = page.locator(".note-row", { hasText: "Sketch" }).first();
const trip = page.locator(".note-row", { hasText: "Trip" }).first();
const box = (l) => l.evaluate((el) => { const r = el.getBoundingClientRect(); return { top: r.top, height: r.height, right: r.right }; });
await sketch.click();
await page.locator(".ProseMirror img.nf-drawing").first().waitFor();
await page.waitForFunction(() => window.__thumbsAtKept !== undefined);
ok((await page.evaluate(() => window.__thumbsAtKept)) === 0, "with no picture kept, no row shows one");
ok(thumbsAskedBefore === 0, "and none is asked for");
const before = await page.evaluate(() => window.__rowAtKept);

// The note has drawn its drawing — which the backend now holds, and says so.
const thumb = sketch.locator(".note-row-thumb");
await thumb.waitFor({ state: "visible", timeout: 3000 }).catch(() => {});
ok(await thumb.isVisible(), "once the note has drawn its picture, the row shows it, unasked");
ok((await thumb.getAttribute("src") ?? "").includes("note%2FD1%2Fthumb"), "from the kept-pictures route, never one that fetches");
const after = await box(sketch);
ok(Math.abs(after.height - before.height) < 0.5 && Math.abs(after.top - before.top) < 0.5,
  `the row keeps its height and place  ${before.height.toFixed(1)} → ${after.height.toFixed(1)}`);
const t = await box(thumb);
ok(t.height <= after.height && Math.abs(after.right - t.right) <= 14, `the picture sits at the row's end, inside its height  ${JSON.stringify({ row: after, thumb: t })}`);
// The row is still one element: the picture is inside it, not a box around it.
ok(await thumb.evaluate((img) => img.parentElement.classList.contains("note-row")), "the picture is part of the row itself");
// A row without a picture keeps its whole width for its words.
const words = await trip.evaluate((row) => {
  const snip = row.querySelector(".note-row-meta").getBoundingClientRect();
  return Math.round(row.getBoundingClientRect().right - snip.right);
});
ok(words <= 14 && (await trip.locator(".note-row-thumb").count()) === 0, `a note without a picture keeps its whole width  ${words}`);

// A picture the computer no longer holds: nothing drawn, no broken frame, the row unchanged.
const tripBefore = await box(trip);
await page.evaluate(() => {
  window.__notePictures = { nD: "D1", nM: "GONE" };
  window.dispatchEvent(new Event("quince:note-picture-kept"));
});
await page.waitForFunction(() => document.querySelectorAll(".note-row-thumb").length !== 1 || window.__pictureAsks > 2, null, { timeout: 2000 }).catch(() => {});
await page.waitForFunction(() => [...document.querySelectorAll('.note-row')].every((r) => !r.textContent.includes("Trip") || !r.querySelector(".note-row-thumb[hidden]")), null, { timeout: 2000 }).catch(() => {});
const shownInTrip = await trip.locator(".note-row-thumb").evaluateAll((els) => els.filter((e) => !e.hidden).length);
const tripAfter = await box(trip);
ok(shownInTrip === 0 && Math.abs(tripAfter.height - tripBefore.height) < 0.5, "a picture that cannot be had is not shown, and the row stays as it was");
await page.screenshot({ path: `${process.env.TMPDIR ?? "/tmp"}/notes-list-pictures-${process.env.ENGINE ?? "chromium"}.png` });
await b.close();
process.exitCode = failures ? 1 : 0;
