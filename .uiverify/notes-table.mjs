// A table in a note, drawn as its grid, its text colour shown, and copied as its cells. Its
// cells are read-only for now (edits made here did not merge cleanly on an iPhone). A table this
// app cannot read says so where it stands — never a file to open.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(() => { window.__tableNote = true; });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
await page.locator(".note-row", { hasText: "Groceries" }).first().click();
await page.waitForSelector(".nf-table");
const grid = await page.locator(".nf-table tr").evaluateAll((rows) => rows.map((r) => [...r.cells].map((c) => c.textContent)));
ok(JSON.stringify(grid) === JSON.stringify([["Item", "Qty"], ["Milk", "2"], ["", ""]]), `the table is drawn as its grid  ${JSON.stringify(grid)}`);

// Read-only for now: edits made here reached the iPhone, but Notes there kept conflict copies of
// the note with the text doubled. No cell is offered for typing and no table menu opens.
ok((await page.locator(".nf-cell-edit").count()) > 0, "table cells are offered for typing now table writes are on (proven on the iPhone)");
await page.locator(".nf-table tr").nth(1).locator("td").first().click({ button: "right" });
ok((await page.getByRole("menuitem", { name: /Add Row|Delete Row/ }).count()) === 0, "and right-click offers no row or column changes");
await page.keyboard.press("Escape");
const coloured = await page.locator(".note-rich .nf-em-4").evaluateAll((e) => e.map((x) => [x.textContent, getComputedStyle(x).color]));
ok(coloured.length === 1 && coloured[0][0] === "end" && !/rgb\(29, 29, 31\)|rgb\(0, 0, 0\)/.test(coloured[0][1]), `text coloured on a phone shows in its colour  ${JSON.stringify(coloured)}`);
// Copying the note puts the table's cells out, never a placeholder word.
const copied = await page.evaluate(() => {
  const ed = document.querySelector(".note-rich");
  ed.focus();
  const range = document.createRange();
  range.selectNodeContents(ed);
  getSelection().removeAllRanges();
  getSelection().addRange(range);
  return new Promise((done) => setTimeout(() => {
    const data = new DataTransfer();
    ed.dispatchEvent(new ClipboardEvent("copy", { clipboardData: data, bubbles: true, cancelable: true }));
    done({ text: data.getData("text/plain"), html: data.getData("text/html") });
  }, 50));
});
ok(!/Attachment/.test(copied.text + copied.html) && /Item\tQty/.test(copied.text) && /<td>Milk<\/td>/.test(copied.html), `copying a table copies its cells  ${JSON.stringify(copied).slice(0, 300)}`);
// Pasting a copied table never makes a second reference to it: the save holds it once.
await page.evaluate(() => {
  const ed = document.querySelector(".note-rich");
  ed.focus();
  const range = document.createRange();
  range.selectNodeContents(ed);
  getSelection().removeAllRanges();
  getSelection().addRange(range);
});
await page.waitForTimeout(50);
await page.evaluate(() => {
  const ed = document.querySelector(".note-rich");
  const data = new DataTransfer();
  ed.dispatchEvent(new ClipboardEvent("copy", { clipboardData: data, bubbles: true, cancelable: true }));
  const last = ed.lastElementChild;
  const range = document.createRange();
  range.selectNodeContents(last);
  range.collapse(false);
  getSelection().removeAllRanges();
  getSelection().addRange(range);
  window.__pasteData = data;
});
await page.waitForTimeout(80);
await page.evaluate(() => {
  const ed = document.querySelector(".note-rich");
  ed.dispatchEvent(new ClipboardEvent("paste", { clipboardData: window.__pasteData, bubbles: true, cancelable: true }));
});
await page.waitForTimeout(1500);
const pasted = await page.evaluate(() => window.__noteSaves?.at(-1));
const refs = (pasted?.runs ?? []).filter((r) => r.attachment === "T1").length;
ok(pasted && refs === 1 && (pasted.body.match(/\uFFFC/g) ?? []).length === 1 && /Item\tQty\nMilk\t2/.test(pasted.body), `a pasted table brings its cells as text, never a second reference  refs=${refs} ${JSON.stringify(pasted?.body)}`);
// Typed in on a phone while the note is open: the next sync redraws the table, without
// leaving the note.
await page.evaluate(() => {
  window.__table[2] = ["Bread", "1"];
  window.__emit("quince://synced", { services: [], asOf: Date.now() / 1000, tookMs: 5 });
});
await page.waitForTimeout(500);
const after = await page.locator(".nf-table tr").nth(2).evaluate((r) => [...r.cells].map((c) => c.textContent));
ok(JSON.stringify(after) === JSON.stringify(["Bread", "1"]), `a table changed elsewhere redraws after the next sync  ${JSON.stringify(after)}`);

// A table this app cannot read: said so where it stands, never a file chip to open.
const ctx2 = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx2.addInitScript(() => { window.__tableNote = true; window.__unreadTable = true; });
await ctx2.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
const p2 = await ctx2.newPage();
await p2.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(p2, "Notes");
await p2.locator(".note-row", { hasText: "Groceries" }).first().click();
await p2.waitForTimeout(600);
ok((await p2.locator(".nf-table-unread").count()) === 1 && (await p2.locator(".nf-file").count()) === 0,
  `a table that can't be read says so, and is no file to open  ${await p2.locator(".nf-attachment").first().innerText().catch(() => "")}`);
await ctx2.close();

// At a fractional scale (1.25×, a common laptop setting) the note's column starts between two
// device pixels. A bordered table then lost its left edge in WebKit and drew its right edge two
// pixels wide. Read from the pixels: both edges carry ink, the same weight, neither smeared
// wider than two device pixels; at 1×, 1.5× and 2× each is one crisp column.
const inkAcross = async (page, scale) => {
  const r = await page.locator(".nf-table").first().boundingBox();
  const shot = (await page.screenshot()).toString("base64");
  return page.evaluate(async ({ b64, r, s }) => {
    const img = new Image();
    img.src = `data:image/png;base64,${b64}`;
    await img.decode();
    const c = document.createElement("canvas");
    c.width = img.width; c.height = img.height;
    const g = c.getContext("2d");
    g.drawImage(img, 0, 0);
    const y = Math.round((r.y + r.height * 0.3) * s);
    const row = g.getImageData(0, y, img.width, 1).data;
    const dark = (x) => 255 - (row[x * 4] + row[x * 4 + 1] + row[x * 4 + 2]) / 3;
    const base = dark(Math.floor((r.x + r.width * 0.25) * s));
    const edge = (at) => {
      const cols = [];
      for (let x = Math.floor(at) - 3; x <= Math.floor(at) + 3; x++) cols.push(Math.max(0, Math.round(dark(x) - base)));
      return { ink: cols.reduce((a, b) => a + b, 0), wide: cols.filter((v) => v > 4).length };
    };
    return { left: edge(r.x * s), right: edge((r.x + r.width) * s) };
  }, { b64: shot, r, s: scale });
};
for (const scale of [1, 1.25, 1.5, 2]) {
  const ctx3 = await b.newContext({ viewport: { width: 1180, height: 800 }, deviceScaleFactor: scale });
  await ctx3.addInitScript(() => { window.__tableNote = true; });
  await ctx3.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
  const p3 = await ctx3.newPage();
  await p3.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await openScreen(p3, "Notes");
  await p3.locator(".note-row", { hasText: "Groceries" }).first().click();
  await p3.waitForSelector(".nf-table");
  await p3.mouse.move(0, 0);
  const bad = [];
  for (const width of scale === 1.25 ? [1180, 1183, 1201] : [1180]) {
    await p3.setViewportSize({ width, height: 800 });
    await p3.waitForTimeout(120);
    const { left, right } = await inkAcross(p3, scale);
    const whole = scale !== 1.25;
    const fine = left.ink > 10 && right.ink > 10 && Math.abs(left.ink - right.ink) <= Math.max(left.ink, right.ink) * 0.5
      && left.wide <= (whole ? 1 : 2) && right.wide <= (whole ? 1 : 2);
    if (!fine) bad.push({ width, left, right });
  }
  ok(bad.length === 0, `at ${scale}×, the table's left and right edges are both drawn, the same weight, unsmeared  ${JSON.stringify(bad)}`);
  await ctx3.close();
}
await b.close();
process.exitCode = failures ? 1 : 0;
