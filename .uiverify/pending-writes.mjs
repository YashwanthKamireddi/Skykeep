// A change on its way to Apple stays drawn when a refresh lands mid-way — Apple pushes the
// change straight back, and the refresh read the library from before it: a note moved to
// another folder came back in the old one for a moment, then went again.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(() => {
  window.__noteFolders = [
    { guid: "f1", title: "Notes", sortOrder: 0 },
    { guid: "f2", title: "Recently Deleted", sortOrder: 1 },
    { guid: "f3", title: "Work", sortOrder: 2 },
  ];
});
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

await page.locator(".notes-folder", { hasText: /^Notes/ }).first().click();
await page.waitForTimeout(300);
await page.locator(".note-row", { hasText: "Curtains" }).first().click();
await page.waitForTimeout(300);
await page.evaluate(() => { window.__slowMove = 900; });
await page.getByRole("button", { name: "Folder for Curtains" }).click();
// The move is out (Apple slow to answer, 900 ms) before anything is watched: a refresh is made
// to land while it is out — timed from the move, not from opening the picker, which on a slow
// run let the refresh land before the move began and the check measured nothing.
await page.getByRole("option", { name: "Work" }).click();
await page.evaluate(() => {
  window.__back = 0; window.__gone = false; window.__watching = true;
  const tick = () => {
    const there = [...document.querySelectorAll(".note-row")].some((r) => r.textContent.includes("Curtains"));
    if (!there) window.__gone = true;
    else if (window.__gone) window.__back++;
    if (window.__watching) requestAnimationFrame(tick);
  };
  tick();
  setTimeout(() => window.__emit?.("quince://synced", { services: [], asOf: Math.floor(Date.now() / 1000), tookMs: 1 }), 300);
});
await page.waitForTimeout(800);
const r = await page.evaluate(() => { window.__watching = false; return { gone: window.__gone, back: window.__back }; });
ok(r.gone && r.back === 0, `a moved note leaves at once and does not come back while the move is out  [${JSON.stringify(r)}]`);
await b.close();
process.exitCode = failures ? 1 : 0;
