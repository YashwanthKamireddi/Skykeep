// What a drag shows under the pointer (lib/dragPicture): a reminder, a contact or a file is
// dragged as its name in a small capsule, a photo as a small square — never the whole row,
// which WebKitGTK drew as wide as the pane and, on a screen at 2×, twice that. In WebKitGTK at
// 2× the picture is painted at half, which WebKitGTK's own doubling brings back to its size.
//
//   BASE=http://localhost:4173 node .uiverify/drag-picture.mjs
//   BASE=http://localhost:4173 ENGINE=webkit node .uiverify/drag-picture.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

/** Every picture handed to setDragImage, measured as it is handed over. */
const spy = () => {
  window.__pictures = [];
  const set = DataTransfer.prototype.setDragImage;
  DataTransfer.prototype.setDragImage = function (el, x, y) {
    const box = el.getBoundingClientRect();
    window.__pictures.push({ w: box.width, h: box.height, transform: el.style.transform, text: el.textContent, cls: el.className });
    return set.call(this, el, x, y);
  };
};
const last = (page) => page.evaluate(() => window.__pictures.at(-1) ?? null);

const fixtures = { remindersFixture: true, contactsFixture: true, driveFixture: true };
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, ...fixtures });
await ctx.addInitScript(spy);
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(BASE, { waitUntil: "networkidle" });

// Reminders: a row onto another row of its list.
await openScreen(page, "Reminders");
await page.locator(".rx-list", { hasText: "Daily" }).click();
const rows = page.locator('.rx-row[draggable="true"]');
await rows.nth(1).waitFor();
const rowBox = await rows.first().boundingBox();
await rows.first().dragTo(rows.nth(1));
let pic = await last(page);
ok(pic && pic.cls.includes("drag-label") && pic.text.includes("Plan tomorrow"), `a reminder is dragged as its name (${pic?.text})`);
ok(pic && pic.w <= 280 && pic.h <= 32 && pic.w < rowBox.width / 2, `small beside its ${Math.round(rowBox.width)}-pixel row: ${pic && `${Math.round(pic.w)}×${Math.round(pic.h)}`}`);
ok(!(await page.locator(".drag-picture").count()), "and gone from the page once the drag has its picture");

// Contacts: a card onto a list.
await openScreen(page, "Contacts");
const card = page.locator('.contact-rows li[draggable="true"]').first();
await card.waitFor();
const list = page.locator(".source-row").last();
await card.dragTo(list);
pic = await last(page);
ok(pic && pic.cls.includes("drag-label") && pic.w <= 280 && pic.h <= 32, `a contact is dragged as its name: ${pic && `${pic.text}, ${Math.round(pic.w)}×${Math.round(pic.h)}`}`);

// Drive: a file onto a folder.
await openScreen(page, "Drive");
const file = page.locator('.drive-row[draggable="true"]:not(.is-folder)').first();
await file.waitFor();
const folder = page.locator(".drive-row.is-folder").first();
await file.dragTo(folder);
pic = await last(page);
ok(pic && pic.cls.includes("drag-label") && pic.w <= 280 && pic.h <= 32, `a file is dragged as its name: ${pic && `${pic.text}, ${Math.round(pic.w)}×${Math.round(pic.h)}`}`);
await ctx.close();

// WebKitGTK on a screen at 2×: the picture painted at half. Marked as WebKitGTK here whatever
// the engine, since only there is it scaled.
const hi = await b.newContext({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 2 });
await hi.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, ...fixtures });
await hi.addInitScript(spy);
await hi.addInitScript(() => document.addEventListener("DOMContentLoaded", () => { document.documentElement.dataset.engine = "gtk"; }));
const p2 = await hi.newPage();
p2.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await p2.goto(BASE, { waitUntil: "networkidle" });
await openScreen(p2, "Reminders");
await p2.locator(".rx-list", { hasText: "Daily" }).click();
const r2 = p2.locator('.rx-row[draggable="true"]');
await r2.nth(1).waitFor();
await r2.first().dragTo(r2.nth(1));
pic = await last(p2);
ok(pic?.transform === "scale(0.5)", `in WebKitGTK at 2× it is painted at half (${pic?.transform || "not scaled"})`);
await hi.close();

await b.close();
console.log(`${engineName}: ${failures ? `${failures} FAILED` : "all passed"}`);
process.exit(failures ? 1 : 0);
