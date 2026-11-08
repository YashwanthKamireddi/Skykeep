// Adding a file to a note never looks stuck: at once a card shows the picture and its name with
// "Adding to the note", the note waits under it (so nothing typed meanwhile is lost), and when
// Apple has it the card goes and the note is editable again.
//
//   BASE=http://localhost:4173 ENGINE=webkit node .uiverify/notes-attach-feedback.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };

const ctx = await b.newContext({ viewport: { width: 1300, height: 850 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
await ctx.addInitScript(() => { window.__attachOn = true; window.__pick = "/home/someone/Pictures/beach.jpg"; window.__attachDelay = 1500; });
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.locator(".note-row", { hasText: "Curtains" }).click();
await page.locator(".note-rich .nf-p", { hasText: "Washing Machine cover" }).waitFor();

const t0 = Date.now();
await page.getByRole("button", { name: "Add a picture or file" }).click();
const card = page.locator(".note-adding-card");
await card.waitFor({ timeout: 1000 });
ok(Date.now() - t0 < 800, `the card is there at once (${Date.now() - t0} ms)`);
ok((await card.innerText()).includes("beach.jpg") && (await card.innerText()).includes("Adding to the note"), "it names the file and says it is being added");
await page.locator(".note-adding-thumb[src^='blob:']").waitFor({ timeout: 1000 });
ok(true, "its picture shows while it uploads");
const covered = await page.evaluate(() => {
  const r = document.querySelector(".note-rich")?.getBoundingClientRect();
  const top = r && document.elementFromPoint(r.left + 20, r.top + 20);
  return !!top?.closest(".note-adding");
});
ok(covered, "the note waits under it, so nothing is typed into it meanwhile");

await card.waitFor({ state: "detached", timeout: 5000 });
ok((await page.locator(".note-adding").count()) === 0, "when Apple has it, the card goes");
const sent = await page.evaluate(() => window.__attaches?.length ?? 0);
ok(sent === 1, "the file was sent once");
const editable = await page.locator(".note-rich [contenteditable='true'], .note-rich[contenteditable='true']").count();
ok(editable > 0, "and the note is editable again");

// Several files at once, one refused: the others still go in, and the refused one is named.
await page.evaluate(() => {
  window.__attaches = [];
  window.__attachDelay = 0;
  window.__pick = ["/home/someone/a.txt", "/home/someone/desktop.ini", "/home/someone/b.mp4"];
});
await page.getByRole("button", { name: "Add a picture or file" }).click();
await page.locator(".toast-host", { hasText: "Added 2 of 3 files" }).waitFor({ timeout: 5000 });
ok(true, "the files around the refused one still go in");
const tried = await page.evaluate(() => window.__attaches.map((a) => a.path.split("/").pop()));
ok(JSON.stringify(tried) === JSON.stringify(["a.txt", "desktop.ini", "b.mp4"]), `each file tried once, in order (${tried})`);
await page.locator(".toast-host", { hasText: "desktop.ini is a system file" }).waitFor({ timeout: 5000 });
ok(true, "and the one left out is named");

await b.close();
process.exitCode = failures ? 1 : 0;
