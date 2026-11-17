// Notes as Notes on the Mac shows them: a web address in the text is a link — found, not
// written into the note — and a click on it opens it; a picture in a note shows large in the
// window on a click, at once, from the picture already drawn, with a way to open it elsewhere.
import zlib from "node:zlib";
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(() => { window.__mediaNote = true; window.__linkNote = true; window.__picBase = "http://pics.test"; });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
// A real picture behind the picture route: a 600×400 gradient, made here as a PNG.
const png = (() => {
  const { deflateSync, crc32 } = zlib;
  const w = 600, h = 400;
  const raw = Buffer.alloc((w * 3 + 1) * h);
  for (let y = 0; y < h; y++) {
    raw[y * (w * 3 + 1)] = 0;
    for (let x = 0; x < w; x++) {
      const o = y * (w * 3 + 1) + 1 + x * 3;
      raw[o] = 74 + (y * 171) / h; raw[o + 1] = 144 + (y * 71) / h; raw[o + 2] = 217 - (y * 107) / h;
    }
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
await ctx.route("http://pics.test/**", (r) => r.fulfill({ status: 200, contentType: "image/png", body: png }));
await ctx.route("https://www.youtube.com/**", (r) => r.fulfill({ status: 200, body: "opened" }));
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

await page.locator(".note-row", { hasText: "Links" }).first().click();
await page.waitForTimeout(600);
const links = await page.locator(".ProseMirror a.nf-autolink").evaluateAll((as) => as.map((a) => [a.textContent, a.getAttribute("href")]));
ok(links.some(([t, h]) => t === "www.youtube.com/watch?v=5vVh0ZGx8aU" && h === "https://www.youtube.com/watch?v=5vVh0ZGx8aU"),
  `an address without its scheme is a link to it  ${JSON.stringify(links)}`);
ok(links.some(([t]) => t === "https://example.org/a"), "a sentence's full stop is not part of the address");
const saves = await page.evaluate(() => window.__noteSaves?.length ?? 0);
ok(saves === 0, "found links change nothing in the note");
// A click on it opens it (the app hands any address outside it to the browser).
const [nav] = await Promise.all([
  page.waitForEvent("framenavigated", { timeout: 3000 }).catch(() => null),
  page.locator(".ProseMirror a.nf-autolink", { hasText: "youtube" }).click(),
]);
ok(nav && nav.url().startsWith("https://www.youtube.com/watch"), `a click opens the address  ${nav?.url()}`);
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.waitForTimeout(400);

await page.locator(".note-row", { hasText: "Trip" }).first().click();
await page.waitForTimeout(800);
const img = page.locator(".ProseMirror img.nf-image");
ok((await img.count()) === 1 && (await img.evaluate((i) => i.complete && i.naturalWidth > 0)), "the picture is drawn in the note");
await img.click();
await page.waitForTimeout(150);
const viewer = page.getByRole("dialog", { name: "beach.jpeg" });
ok(await viewer.isVisible(), "a click shows it large, in the window");
ok(/note%2FM1%2Finline$/.test(await img.getAttribute("src")), "the note draws the picture at the size it needs, not the whole photo");
// At once from the picture already drawn, then whole once that is decoded — never a blank.
const shownSrc = await viewer.locator("img").getAttribute("src");
ok(shownSrc === (await img.getAttribute("src")) || /note%2FM1%2Ffile$/.test(shownSrc), `shown from the picture already drawn, or already whole  ${shownSrc}`);
await page.waitForTimeout(400);
ok(/note%2FM1%2Ffile$/.test(await viewer.locator("img").getAttribute("src")) && (await viewer.locator("img").evaluate((i) => i.complete && i.naturalWidth > 0)),
  "and then the whole picture, drawn");
ok((await page.evaluate(() => window.__opened ?? [])).length === 0, "and nothing handed to another app");
await viewer.getByRole("button", { name: "Open in Default App" }).click();
await page.waitForTimeout(100);
ok(JSON.stringify(await page.evaluate(() => window.__opened)) === JSON.stringify(["M1"]), "Open in Default App hands it over");
await page.keyboard.press("Escape");
await page.waitForTimeout(100);
ok(!(await viewer.isVisible()), "Escape puts it away");
await img.click();
await page.waitForTimeout(150);
ok(await viewer.isVisible(), "shown again");
await page.mouse.click(10, 10);
await page.waitForTimeout(100);
ok(!(await viewer.isVisible()), "and so does a click outside");
await page.screenshot({ path: `${process.env.TMPDIR ?? "/tmp"}/notes-links-${process.env.ENGINE}.png` });
await b.close();
process.exitCode = failures ? 1 : 0;
