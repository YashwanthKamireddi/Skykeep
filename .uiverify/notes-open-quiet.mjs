// Opening a note is not an edit. A note of pictures, files, a film, a recording and a table —
// with Apple's runs in pieces, as a phone leaves them — is opened, looked at, scrolled, clicked
// into, its pictures and table clicked, the cursor moved, the note left and opened again: no
// save may be queued. One written then would rewrite the note's formatting on iCloud without
// anyone touching it (it happened, on his "Attachments" note). A real edit still saves.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

// A real picture to draw, served where the stub's routes point.
const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAIAAAADCAIAAAA2iEnWAAAAFklEQVR4nGP4z8DAwMDAxMDAwMAAAA8bAQPsc2MxAAAAAElFTkSuQmCC",
  "base64",
);
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(() => {
  window.__attachNote = true;
  window.__picBase = "http://pics.test";
});
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
await ctx.route("http://pics.test/**", (route) => route.fulfill({ status: 200, contentType: "image/png", body: PNG }));
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const saves = () => page.evaluate(() => (window.__noteSaves ?? []).length);

await page.locator(".note-row", { hasText: "Attachments" }).first().click();
await page.waitForSelector(".nf-table");
await page.waitForTimeout(1500);
ok((await saves()) === 0, "opening a note with attachments and a table queues no save");

// Looking at it: scrolled, clicked into its text, the cursor moved, a picture and the table
// clicked, a word selected — nothing typed.
const editor = page.locator(".note-rich");
await editor.hover();
await page.mouse.wheel(0, 600);
await page.waitForTimeout(200);
await page.mouse.wheel(0, -600);
await editor.locator("p").first().click();
for (const k of ["End", "ArrowDown", "ArrowDown", "ArrowDown", "ArrowUp", "Home", "Shift+ArrowRight", "ArrowLeft"]) await page.keyboard.press(k);
await page.locator(".nf-table td").first().click();
await page.locator(".note-rich p", { hasText: "A few words" }).dblclick();
const pic = page.locator(".nf-image").first();
if (await pic.count()) {
  await pic.click();
  await page.waitForTimeout(200);
  await page.keyboard.press("Escape");
}
await page.waitForTimeout(1500);
ok((await saves()) === 0, `looking, scrolling, clicking and selecting queue no save  ${JSON.stringify(await page.evaluate(() => window.__noteSaves?.[0]?.runs?.slice(0, 3)))}`);

// A press that drifts a few pixels is a drag: of the table, a file card, a picture, or of
// selected text — let go where it began. That queued the save he found (the same text, the
// runs merged), and a table so dragged came back as its cells' text, the table gone.
const nudge = async (box) => {
  await page.mouse.move(box.x + 6, box.y + 6);
  await page.mouse.down();
  await page.mouse.move(box.x + 10, box.y + 8, { steps: 5 });
  await page.mouse.move(box.x + 7, box.y + 6, { steps: 4 });
  await page.mouse.up();
};
for (const sel of [".nf-table td", ".nf-file", ".nf-attachment.is-image img"]) {
  const box = await page.locator(sel).first().boundingBox();
  await page.mouse.click(box.x + 6, box.y + 6);
  await nudge(box);
}
const words = page.locator(".note-rich p", { hasText: "A few words" });
await words.dblclick({ position: { x: 30, y: 8 } });
await nudge(await words.boundingBox());
await page.waitForTimeout(1500);
ok((await saves()) === 0 && (await page.locator(".nf-table").count()) === 1,
  `a drag let go where it began queues no save, and the table stays a table  ${JSON.stringify(await page.evaluate(() => window.__noteSaves?.[0]?.body))}`);

// Left and opened again.
await page.locator(".note-row").filter({ hasNotText: "Attachments" }).first().click();
await page.waitForTimeout(400);
await page.locator(".note-row", { hasText: "Attachments" }).first().click();
await page.waitForSelector(".nf-table");
await page.waitForTimeout(1500);
ok((await saves()) === 0, "leaving the note and opening it again queues no save");

// Attachments are shown: pictures as pictures, files as cards with their name and size.
const shown = await page.evaluate(() => ({
  images: document.querySelectorAll(".note-rich .nf-attachment.is-image img").length,
  cards: [...document.querySelectorAll(".note-rich .nf-file")].map((c) => c.textContent),
  docs: document.querySelectorAll(".note-rich .nf-doc").length,
  blank: [...document.querySelectorAll(".note-rich .nf-attachment")].filter((a) => a.getBoundingClientRect().height < 8).length,
}));
ok(shown.images === 4 && shown.docs === 2 && shown.cards.length === 2 && shown.blank === 0,
  `every attachment is drawn — 4 pictures, 2 previews, 2 file cards, none blank  ${JSON.stringify(shown)}`);
ok(shown.cards.some((c) => /servers\.json/.test(c) && /KB/.test(c)) && shown.cards.some((c) => /recording\.m4a/.test(c)),
  "a file card names the file and its size");

// A real edit still saves — and keeps every untouched run as Apple has it.
await page.locator(".note-rich p", { hasText: "A few words" }).click();
await page.keyboard.press("End");
await page.keyboard.type("!");
await page.waitForTimeout(1500);
const saved = await page.evaluate(() => window.__noteSaves ?? []);
ok(saved.length === 1 && saved[0].body.includes("!"), `a typed key saves once  ${saved.length}`);
const s = saved[0];
const ids = (rs) => (rs ?? []).filter((r) => r.attachment).map((r) => r.attachment).join(",");
ok(s && ids(s.runs) === ids(s.baseRuns ?? s.base_runs), "every attachment keeps its place in the saved runs");
await b.close();
process.exitCode = failures ? 1 : 0;
