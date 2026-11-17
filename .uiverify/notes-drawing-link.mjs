// A phone's drawing and a link to another note, as Notes shows them: the drawing as its ink,
// made for the page's look (light, or dark with its lightness turned as a phone turns it),
// opening large on a click; a link to a note opens that note here, whatever folder holds it.
import zlib from "node:zlib";
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
// Ink on white, as Apple's fallback image is: a white 300×240 PNG with a black band.
const png = (() => {
  const { deflateSync, crc32 } = zlib;
  const w = 300, h = 240;
  const raw = Buffer.alloc((w * 3 + 1) * h, 255);
  for (let y = 0; y < h; y++) {
    raw[y * (w * 3 + 1)] = 0;
    if (y > 100 && y < 120) raw.fill(0, y * (w * 3 + 1) + 1, (y + 1) * (w * 3 + 1));
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
for (const scheme of ["light", "dark"]) {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 800 }, colorScheme: scheme });
  await ctx.addInitScript(() => { window.__drawNote = true; window.__picBase = "http://pics.test"; });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
  await ctx.route("http://pics.test/**", (r) => r.fulfill({ status: 200, contentType: "image/png", body: png }));
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
  await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await openScreen(page, "Notes");
  await page.waitForTimeout(400);
  await page.locator(".note-row", { hasText: "Sketch" }).first().click();
  await page.waitForTimeout(700);
  const drawing = page.locator(`.ProseMirror img.nf-drawing.nf-ink-${scheme}`);
  const other = page.locator(`.ProseMirror img.nf-drawing.nf-ink-${scheme === "light" ? "dark" : "light"}`);
  ok((await drawing.count()) === 1 && (await drawing.getAttribute("src")).includes(scheme === "dark" ? "note%2FD1%2Fdrawing-dark" : "note%2FD1%2Fdrawing"),
    `${scheme}: the drawing is drawn as its ink for this look, not shown as a file`);
  ok((await page.locator(".ProseMirror .nf-file").count()) === 0, `${scheme}: no file chip for it`);
  ok((await drawing.isVisible()) && !(await other.isVisible()), `${scheme}: the ink for this look shows, the other does not`);
  await page.screenshot({ path: `${process.env.TMPDIR ?? "/tmp"}/notes-drawing-${scheme}-${process.env.ENGINE}.png` });
  await drawing.click();
  await page.waitForTimeout(150);
  ok(await page.getByRole("dialog", { name: "Drawing" }).isVisible(), `${scheme}: a click shows it large`);
  await page.keyboard.press("Escape");
  // Chosen in the note, its ring is the drawing's own edges, not the width of the line.
  const ring = await drawing.evaluate((img) => {
    const box = img.closest(".nf-attachment");
    return { selected: box.classList.contains("ProseMirror-selectednode"), img: getComputedStyle(img).outlineStyle, box: getComputedStyle(box).outlineStyle };
  });
  ok(!ring.selected || (ring.img === "solid" && ring.box === "none"), `${scheme}: chosen, the ring sits on the drawing itself  ${JSON.stringify(ring)}`);
  ok((await page.locator(".ProseMirror a[target]").count()) === 0, `${scheme}: no link opens a window of its own`);
  // The link to another note: that note opens, in Quince.
  const link = page.locator(".ProseMirror a", { hasText: "The plan" });
  ok((await link.count()) === 1, `${scheme}: the link to a note shows as a link`);
  await link.click();
  await page.waitForTimeout(600);
  ok(page.url().startsWith("http://localhost:4173"), `${scheme}: following it stays in Skykeep`);
  const opened = (await page.locator(".note-rich .nf-p").first().innerText()).trim();
  ok(opened === "The plan" && /Step one/.test(await page.locator(".note-rich").first().innerText()),
    `${scheme}: and opens the note it links to  (the note open is "${opened}")`);
  await ctx.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
