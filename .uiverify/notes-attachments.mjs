// A note's pictures and files shown as Notes shows them: a picture inline at its own
// proportions — its place held at that shape before a byte of it arrives — a film or PDF as
// Apple's rendering of it, any other file as a card with its kind, name and size that opens on
// a click. One whose lookup fails is asked about again, never left a blank gap.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAIAAAADCAIAAAA2iEnWAAAAFklEQVR4nGP4z8DAwMDAxMDAwMAAAA8bAQPsc2MxAAAAAElFTkSuQmCC",
  "base64",
);
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(() => {
  window.__attachNote = true;
  window.__picBase = "http://pics.test";
  // The film's lookup fails twice before it answers; the pictures arrive slowly.
  window.__flakyInfo = { A3: 2 };
});
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
let release;
const held = new Promise((r) => { release = r; });
await ctx.route("http://pics.test/**", async (route) => {
  await held;
  await route.fulfill({ status: 200, contentType: "image/png", body: PNG });
});
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
await page.locator(".note-row", { hasText: "Attachments" }).first().click();
await page.waitForSelector(".nf-table");
await page.waitForTimeout(500);

// Pictures not yet arrived already hold their place at their own shape, no wider than the note.
const boxes = await page.locator(".note-rich .nf-attachment.is-image img").evaluateAll((imgs) =>
  imgs.map((i) => { const r = i.getBoundingClientRect(); return [Math.round(r.width), Math.round(r.height), i.complete && i.naturalWidth > 0]; }));
ok(boxes.length === 4 && boxes.every(([w, h, done]) => !done && w > 100 && h > 100 && w <= 640),
  `a picture holds its place at its proportions before it arrives  ${JSON.stringify(boxes)}`);
const portrait = boxes.find(([w, h]) => h > w);
ok(portrait && Math.abs(portrait[0] / portrait[1] - 3024 / 4032) < 0.02, "a portrait picture is drawn portrait");
release();
await page.waitForTimeout(400);

// The film whose lookup failed: a frame holds its place, then it is drawn once asked again.
const film = page.locator(".nf-doc.is-video");
const pendingNow = await page.evaluate(() => [...document.querySelectorAll(".note-rich .nf-attachment.is-pending")].map((a) => Math.round(a.getBoundingClientRect().height)));
ok((await film.count()) === 1 || (pendingNow.length === 1 && pendingNow[0] > 20), `an attachment not yet known holds its place, never a blank gap  ${JSON.stringify(pendingNow)}`);
await film.waitFor({ timeout: 15000 }).catch(() => {});
ok((await film.count()) === 1, "an attachment whose lookup failed is asked about again and drawn");

// A file opens on a click.
await page.locator(".nf-file", { hasText: "servers.json" }).click();
await page.waitForTimeout(150);
ok(JSON.stringify(await page.evaluate(() => window.__opened)) === JSON.stringify(["A2"]), "clicking a file card opens it");
ok((await page.evaluate(() => (window.__noteSaves ?? []).length)) === 0, "and the note is left alone");
await b.close();
process.exitCode = failures ? 1 : 0;
