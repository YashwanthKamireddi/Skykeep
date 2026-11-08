// A whole folder — or several things chosen together — downloaded from Drive into Downloads:
// offered in the menu and by the keys alone, shown as one calm line however many files are in
// it, cancellable, its failures named, and Show in Folder once it is there. A single file
// still downloads the way it always has.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, driveFixture: true });
const page = await ctx.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
await page.goto(process.env.BASE ?? process.env.URL ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Drive");
await page.waitForSelector(".drive-row-hit");
await page.waitForTimeout(300);

let failures = 0;
const ok = (pass, what) => { console.log(`${pass ? "PASS" : "FAIL"} ${what}`); if (!pass) failures++; };
const row = (name) => page.locator(".drive-row-hit", { hasText: name });
const tray = page.locator(".download-tray");
const got = (key) => page.evaluate((key) => window[key] ?? [], key);
const lastBatch = async () => (await got("__downloadItems")).at(-1)?.batch;
const say = (batch, e) => page.evaluate(([batch, e]) => window.__emit("quince://download", {
  batch, within: "Downloads", sent: 0, total: 1000, folder: false, error: null, ...e, id: `${batch}:${e.id}` }), [batch, e]);
const end = (batch, report) => page.evaluate(([batch, report]) => window.__downloadEnd[batch]({
  key: "k", saved: [], failed: [], moreFailed: 0, cancelled: false, problem: null, resumable: false, ...report }), [batch, report]);
const settle = () => page.waitForTimeout(250);

// ── The toolbar's Download ─────────────────────────────────────────────────────────────────
// Nothing selected, nothing to save; a file, saved as one; a folder, as its whole tree.
const toolbarDownload = page.locator(".drive-tools").getByRole("button", { name: "Download" });
ok(await toolbarDownload.isDisabled(), "the toolbar's Download waits for something to be selected");
const firstFile = page.locator(".drive-row-hit").filter({ hasNotText: "Downloads" }).last();
await firstFile.click();
await settle();
await toolbarDownload.click();
await settle();
ok((await got("__singleDownloads")).length === 1, `a selected file is saved from the toolbar  ${JSON.stringify(await got("__singleDownloads"))}`);
await row("Downloads").click();
await settle();
const before = (await got("__downloadItems")).length;
await toolbarDownload.click();
await settle();
const fromBar = (await got("__downloadItems")).at(-1);
ok((await got("__downloadItems")).length === before + 1 && fromBar?.items?.[0]?.drivewsid === "FOLDER::a", "a selected folder is saved as its whole tree");
await end(await lastBatch(), { saved: [] });
await settle();
// What follows starts as it always has: a fresh window, counting its own downloads.
await page.reload({ waitUntil: "networkidle" });
await openScreen(page, "Drive");
await page.waitForSelector(".drive-row-hit");
await page.waitForTimeout(300);
await page.evaluate(() => { window.__singleDownloads = []; window.__downloadItems = []; });

// ── A folder, from its menu ────────────────────────────────────────────────────────────────
await row("Downloads").click({ button: "right" });
const entries = await page.getByRole("menuitem").allInnerTexts();
ok(entries.some((t) => t.includes("Open")) && entries.some((t) => t.includes("Download a Copy")),
  `a folder's menu offers to download it  ${JSON.stringify(entries)}`);
await page.getByRole("menuitem", { name: "Download a Copy" }).click();
await settle();
let asked = (await got("__downloadItems")).at(-1);
ok(asked?.items?.length === 1 && asked.items[0].drivewsid === "FOLDER::a", `the folder itself is asked for  ${JSON.stringify(asked?.items?.map((i) => i.drivewsid))}`);
ok(asked?.title === "Downloads", `under its own name, to resume by  [${asked?.title}]`);
ok((await got("__singleDownloads")).length === 0, "a folder is not sent to the one-file download");
ok(await tray.isVisible(), "the downloads tray appears at once");
ok((await tray.locator(".upload-tray-title").innerText()) === "Downloading Downloads", `its head names the folder  [${await tray.locator(".upload-tray-title").innerText()}]`);
ok(/Preparing…/.test(await tray.locator(".upload-row").innerText()), "before anything is found it is preparing");

let batch = asked.batch;
for (const n of [1, 2, 3]) await say(batch, { id: "found", name: "", folder: true, sent: n, total: n * 1000, state: "found" });
await settle();
ok(/Preparing… 3 found/.test(await tray.locator(".upload-row").innerText()), `while the tree is counted it says what it has found  [${await tray.locator(".upload-row").innerText()}]`);
await say(batch, { id: "planned", name: "", folder: true, sent: 3, total: 3000, state: "planned" });
await say(batch, { id: 0, name: "photo 0.jpg", state: "downloading", sent: 500 });
await settle();
const rows = await tray.locator(".upload-row").count();
ok(rows === 1, `a folder of many files is one line, not a row per file  [${rows}]`);
ok(/0 of 3/.test(await tray.locator(".upload-row").innerText()), "once everything is found it counts to the total");
ok((await tray.locator(".upload-tray-percent").innerText()) === "16%", `the head says how far, by bytes  [${await tray.locator(".upload-tray-percent").innerText()}]`);
await say(batch, { id: 0, name: "photo 0.jpg", state: "done", sent: 1000 });
// A report overtaken on its way: the bar never slides back.
await say(batch, { id: 0, name: "photo 0.jpg", state: "downloading", sent: 100 });
await settle();
ok(/1 of 3/.test(await tray.locator(".upload-row").innerText()), "each file that lands is counted");
ok((await tray.locator(".upload-tray-percent").innerText()) === "33%", "a late report does not take it backwards");
for (const n of [1, 2]) await say(batch, { id: n, name: `photo ${n}.jpg`, state: "done", sent: 1000 });
await end(batch, { saved: ["/home/test/Downloads/Downloads 2"] });
await settle();
ok((await tray.locator(".upload-tray-title").innerText()) === "Downloads downloaded", `finished, the head says so  [${await tray.locator(".upload-tray-title").innerText()}]`);
ok((await tray.getByLabel("Downloaded").count()) === 1, "and the line is ticked");
const show = tray.getByRole("button", { name: "Show in Folder" });
ok((await show.count()) === 1, "it offers Show in Folder");
await show.click();
await settle();
ok(JSON.stringify(await got("__revealed")) === JSON.stringify(["/home/test/Downloads/Downloads 2"]), `Show in Folder shows the folder it made  ${JSON.stringify(await got("__revealed"))}`);
await page.waitForTimeout(4500);
ok(await tray.isVisible(), "it stays until it is closed: Show in Folder is what it is for");
await tray.getByRole("button", { name: "Close downloads" }).click();
await settle();
ok((await tray.count()) === 0, "Done closes it");

// ── A selection of files and folders ────────────────────────────────────────────────────────
const mod = process.platform === "darwin" ? "Meta" : "Control";
await row("Semester Plan").click();
await row("Downloads").click({ modifiers: [mod] });
await row("Downloads").click({ button: "right" });
ok((await page.getByRole("menuitem", { name: "Download 2 Copies" }).count()) === 1, "a selection with a folder in it can be downloaded");
await page.getByRole("menuitem", { name: "Download 2 Copies" }).click();
await settle();
asked = (await got("__downloadItems")).at(-1);
ok(asked.items.map((i) => i.drivewsid).sort().join() === "FILE::d,FOLDER::a", `every item chosen goes, in one download  ${JSON.stringify(asked.items.map((i) => i.drivewsid))}`);
ok((await tray.locator(".upload-tray-title").innerText()) === "Downloading 2 items", "one line for the whole selection");
batch = asked.batch;
await say(batch, { id: 0, name: "Semester Plan.pdf", within: "", state: "downloading", sent: 10 });
await say(batch, { id: 1, name: "secret.key", within: "Downloads", state: "failed", error: "its path is too long for this computer" });
await say(batch, { id: "x:0", name: "Broken", within: "Downloads", folder: true, state: "failed", error: "iCloud Drive is busy right now — try this one again in a minute." });
await say(batch, { id: "planned", name: "", folder: true, sent: 2, total: 2000, state: "planned" });
await say(batch, { id: 0, name: "Semester Plan.pdf", within: "", state: "done", sent: 1000 });
await end(batch, { saved: ["/home/test/Downloads/Semester Plan 2.pdf", "/home/test/Downloads/Downloads 3"], failed: ["Downloads/secret.key: its path is too long for this computer", "Downloads/Broken: busy"] });
await settle();
ok(/2 didn’t download/.test(await tray.locator(".upload-row").innerText()), `what did not arrive is counted  [${await tray.locator(".upload-row").innerText()}]`);
ok((await tray.locator(".upload-tray-title").innerText()) === "Some downloads didn’t finish", "and the head says not all of it finished");
await tray.getByRole("button", { name: "Show what didn’t download" }).click();
const why = await tray.locator(".upload-failures").innerText();
ok(/Downloads\/secret\.key/.test(why) && /too long/.test(why) && /Downloads\/Broken/.test(why), `each is named with why  [${why.replace(/\s+/g, " ")}]`);
ok((await tray.getByRole("button", { name: "Show in Folder" }).count()) === 1, "what did arrive can still be shown");
await tray.getByRole("button", { name: "Close downloads" }).click();
await settle();

// ── Cancel ──────────────────────────────────────────────────────────────────────────────────
await row("Downloads").click();
await row("Downloads").click({ button: "right" });
await page.getByRole("menuitem", { name: "Download a Copy" }).click();
await settle();
batch = await lastBatch();
await say(batch, { id: 0, name: "a.jpg", state: "done", sent: 1000 });
await say(batch, { id: 1, name: "b.jpg", state: "downloading", sent: 10 });
await settle();
ok((await tray.getByRole("button", { name: /^Cancel / }).count()) === 0, "one download has one Cancel, not two");
await tray.getByRole("button", { name: "Cancel", exact: true }).click();
await settle();
const cancels = await got("__downloadCancels");
ok(cancels.length === 1 && cancels[0].batch === batch, `Cancel stops that download  ${JSON.stringify(cancels)}`);
ok(/Cancelled after 1/.test(await tray.locator(".upload-row").innerText()), `then it says it was cancelled, and after how many  [${await tray.locator(".upload-row").innerText()}]`);
// Cancelled is not the end of it: it can be gone on with, where it was.
const resume = tray.getByRole("button", { name: "Resume" });
ok((await resume.count()) === 1, "a cancelled download offers to Resume");
await resume.click();
await settle();
const again = (await got("__downloadItems")).at(-1);
ok(again.batch !== batch && again.items[0].drivewsid === "FOLDER::a" && again.title === "Downloads", "Resume asks for the same download again — the backend goes on where it was");
ok((await tray.locator(".upload-row").count()) === 1 && /Downloading Downloads/.test(await tray.locator(".upload-tray-title").innerText()), "in place of the cancelled line, not beside it");
await end(again.batch, { saved: ["/home/test/Downloads/Downloads 2"] });
await settle();
ok((await tray.locator(".upload-tray-title").innerText()) === "Downloads downloaded", "and then finishes");
await tray.getByRole("button", { name: "Close downloads" }).click();
await settle();

// ── By the keys alone ───────────────────────────────────────────────────────────────────────
for (const key of ["Shift+F10", "ContextMenu"]) {
  await row("Downloads").click();
  const before = (await got("__downloadItems")).length;
  await page.keyboard.press(key);
  await settle();
  ok((await page.locator(".row-menu").count()) === 1, `${key} opens the selection's menu`);
  // Type-select, as a menu takes it, then Return.
  await page.keyboard.type("Dow");
  await page.keyboard.press("Enter");
  await settle();
  const after = await got("__downloadItems");
  ok(after.length === before + 1 && after.at(-1).items[0].drivewsid === "FOLDER::a", `${key}, then Download a Copy, downloads the folder`);
  await page.evaluate((b) => window.__downloadEnd[b]({ saved: [], failed: [], moreFailed: 0, cancelled: true, problem: null }), after.at(-1).batch);
  await settle();
  if (await tray.count()) await tray.getByRole("button", { name: "Close downloads" }).click();
}

// ── One file, as before ─────────────────────────────────────────────────────────────────────
await row("Semester Plan").click({ button: "right" });
await page.getByRole("menuitem", { name: "Download a Copy" }).click();
await settle();
ok(JSON.stringify(await got("__singleDownloads")) === JSON.stringify(["FILE::d"]), "one file still goes the one-file way");
ok((await tray.count()) === 0, "with its notice, not the tray");

// ── Downloads and uploads at once: stacked, never over each other ──────────────────────────
await row("Downloads").click();
await row("Downloads").click({ button: "right" });
await page.getByRole("menuitem", { name: "Download a Copy" }).click();
await settle();
await page.evaluate(() => window.__emit("quince://upload", { id: "u:0", batch: "u", name: "notes.txt", within: "", sent: 10, total: 100, state: "sending", folder: false, error: null }));
await settle();
const boxes = await page.locator(".upload-tray").evaluateAll((els) => els.map((e) => e.getBoundingClientRect().toJSON()));
const apart = boxes.length === 2 && (boxes[0].bottom <= boxes[1].top || boxes[1].bottom <= boxes[0].top);
const inside = boxes.every((r) => r.top >= 0 && r.bottom <= 800 && r.right <= 1280);
ok(apart && inside, `the two trays sit one above the other, on screen  ${JSON.stringify(boxes.map((r) => [Math.round(r.top), Math.round(r.bottom)]))}`);

// ── Left unfinished last time: offered again when Drive opens ──────────────────────────────
const later = await b.newContext({ viewport: { width: 1280, height: 800 } });
await later.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, driveFixture: true });
await later.addInitScript(() => {
  window.__unfinished = [{ key: "0123456789abcdef0123456789abcdef", title: "Archive",
    items: [{ drivewsid: "FOLDER::archive", parentId: null, name: "Archive", extension: null, kind: "FOLDER",
      size: null, childCount: null, createdAt: null, modifiedAt: null, etag: null }],
    saved: ["/home/test/Downloads/Archive"] }];
});
const next = await later.newPage();
next.on("pageerror", (e) => errors.push(String(e)));
await next.goto(process.env.BASE ?? process.env.URL ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(next, "Drive");
await next.waitForTimeout(500);
const offered = next.locator(".download-tray");
ok((await offered.count()) === 1 && /Archive/.test(await offered.innerText()) && /Paused/.test(await offered.innerText()),
  `a download left unfinished is offered again  [${(await offered.innerText().catch(() => "")).replace(/\s+/g, " ")}]`);
ok((await offered.locator(".upload-tray-title").innerText()) === "A download didn’t finish", "its head says so plainly");
ok((await offered.getByRole("button", { name: "Show in Folder" }).count()) === 1, "what of it is there can be shown");
await offered.getByRole("button", { name: "Forget Archive" }).click();
await next.waitForTimeout(250);
ok(JSON.stringify(await next.evaluate(() => window.__forgotten ?? [])) === JSON.stringify(["0123456789abcdef0123456789abcdef"]), "it can be let go of");
ok((await offered.count()) === 0, "and then it is no longer offered");
await later.close();

ok(errors.length === 0, `no page errors  ${JSON.stringify(errors)}`);
await b.close();
process.exitCode = failures ? 1 : 0;
