// A link in a shared note is whoever wrote the note's to choose. Only a web page, an email, a
// call or a place is followed; a link to a file on a share, a search query or a page made of
// data never leaves the window, and says, calmly, that it can't be opened from Skykeep. Every
// click is taken the one way (lib/follow.ts): the engine never navigates by itself.
//
// The note here is invented, shaped as a phone stores one: text with link runs.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const LINKS = [
  ["Agenda", "file://attacker.example@SSL/DavWWWRoot/agenda.lnk"],
  ["Search", "search-ms:query=x&crumb=location:%5C%5Cattacker.example%5Cshare"],
  ["Sign in", "data:text/html,<h1>Sign in to iCloud</h1>"],
  ["Run", "javascript:document.title='ran'"],
  ["Site", "https://example.org/site"],
];
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
await ctx.addInitScript((links) => {
  const t = window.__TAURI_INTERNALS__;
  const inner = t.invoke;
  const title = "Shared plans";
  const body = [title, ...links.map(([w]) => w)].join("\n");
  const note = { guid: "nS", folderGuid: "f1", title, snippet: links[0][0], body, locked: false, editable: true, pinned: false,
    createdAt: 1789000000, modifiedAt: 1789900000, etag: null };
  const plain = (length) => ({ length, style: null, indent: 0, checked: null, bold: false, italic: false, underline: false, strike: false, link: null });
  const runs = [{ ...plain(title.length + 1), style: 0 }];
  links.forEach(([w, href], i) => {
    runs.push({ ...plain(w.length), link: href });
    if (i < links.length - 1) runs.push(plain(1));
  });
  t.invoke = async (cmd, args) => {
    switch (cmd) {
      case "notes_in": return [{ ...note, body: null }, ...(await inner(cmd, args))];
      case "note_body": if (args?.guid === "nS") return body; break;
      case "note_runs": if (args?.guid === "nS") return runs; break;
      case "note_row": if (args?.guid === "nS") return { ...note, body: null }; break;
    }
    return inner(cmd, args);
  };
}, LINKS);
await ctx.route("https://example.org/**", (r) => r.fulfill({ status: 200, body: "opened" }));
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
const base = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
await page.goto(base, { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

await page.locator(".note-row", { hasText: "Shared plans" }).first().click();
await page.waitForTimeout(700);
for (const [word] of LINKS.slice(0, 4)) {
  const link = page.locator(".ProseMirror a", { hasText: word });
  ok(await link.evaluate((a) => a.classList.contains("nf-link-refused") && !!a.title), `"${word}" is drawn as text, saying why`);
  const navs = [];
  const seen = (f) => { if (f === page.mainFrame()) navs.push(f.url()); };
  page.on("framenavigated", seen);
  await link.click();
  await page.waitForTimeout(400);
  page.off("framenavigated", seen);
  ok(navs.length === 0 && page.url().startsWith(base), `a click on "${word}" goes nowhere  ${JSON.stringify(navs)}`);
  ok(await page.getByText("This kind of link can't be opened from Skykeep.").first().isVisible(), `and says it can't be opened from Skykeep`);
}
ok((await page.title()) !== "ran", "a javascript: link never runs");
// A web page is still followed, the one way out.
const [nav] = await Promise.all([
  page.waitForEvent("framenavigated", { timeout: 3000 }).catch(() => null),
  page.locator(".ProseMirror a", { hasText: "Site" }).click(),
]);
ok(nav?.url() === "https://example.org/site", `a web page is still opened  ${nav?.url()}`);
await b.close();
process.exitCode = failures ? 1 : 0;
