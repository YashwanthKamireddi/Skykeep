// A numbered list with one nested in it, as a phone numbers it: the nested items count from 1
// on their own, and the outer list carries on after them — 1, (1, 2), 2 — in the editor and in a
// note shown read-only. One count for every depth had the outer list go on from 4.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

const lines = [["Steps", 0, null], ["Pack", 0, 102], ["Socks", 1, 102], ["Boots", 1, 102], ["Drive", 0, 102], ["Done", 0, null]];
const body = lines.map(([t]) => t).join("\n");
const runs = lines.map(([t, indent, style], i) => ({ length: t.length + (i < lines.length - 1 ? 1 : 0), style: i === 0 ? 0 : style, indent,
  checked: null, bold: false, italic: false, underline: false, strike: false, link: null }));
const note = (guid, editable) => ({ guid, folderGuid: "f1", title: "Steps", snippet: "", body, locked: false, editable, pinned: false,
  createdAt: null, modifiedAt: 1789900000 + (editable ? 1 : 0), etag: null });

const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(([notes, runs]) => {
  window.__notesExtra = notes;
  window.__savedRuns = Object.fromEntries(notes.map((n) => [n.guid, runs]));
}, [[note("nNum", true), note("nNumRO", false)], runs]);
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); failures++; });
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Notes");

await page.locator('.note-row[data-guid="nNum"]').click();
await page.waitForTimeout(500);
const shown = await page.locator('.note-rich .nf-p[data-style="102"]').evaluateAll((ps) => ps.map((p) => `${p.getAttribute("data-n")}. ${p.textContent}`));
ok(JSON.stringify(shown) === JSON.stringify(["1. Pack", "1. Socks", "2. Boots", "2. Drive"]), `editor: ${JSON.stringify(shown)}`);
const drawn = await page.locator('.note-rich .nf-p[data-style="102"]').first().evaluate((p) => getComputedStyle(p, "::before").content);
ok(/attr\(data-n\)|"1\."/.test(drawn) || drawn.includes("1"), `and the number is what is drawn  ${drawn}`);

// A new item typed under the nested one counts on from it.
await page.waitForTimeout(600);
const boots = page.locator('.note-rich .nf-p', { hasText: "Boots" });
const bb = await boots.boundingBox();
// At the end of its words, past the number's gutter.
await page.mouse.click(bb.x + bb.width - 4, bb.y + bb.height / 2);
await page.waitForTimeout(150);
await page.keyboard.press("End");
await page.keyboard.press("Enter");
await page.keyboard.type("Hat");
await page.waitForTimeout(200);
const typed = await page.locator('.note-rich .nf-p[data-style="102"]').evaluateAll((ps) => ps.map((p) => `${p.getAttribute("data-n")}. ${p.textContent}`));
ok(JSON.stringify(typed) === JSON.stringify(["1. Pack", "1. Socks", "2. Boots", "3. Hat", "2. Drive"]), `a new nested item: ${JSON.stringify(typed)}`);

await page.waitForTimeout(1200);
await page.locator('.note-row[data-guid="nNumRO"]').click();
await page.waitForTimeout(500);
await page.locator(".note-formatted").waitFor({ timeout: 3000 }).catch(async () => console.log(await page.locator(".note-detail, .note-editor").first().evaluate((e) => e.outerHTML.slice(0, 400)).catch(() => "none")));
const read = await page.locator(".note-formatted .nf-item").evaluateAll((ps) => ps.map((p) => p.innerText.replace(/\s+/g, " ").trim()));
ok(JSON.stringify(read) === JSON.stringify(["1. Pack", "1. Socks", "2. Boots", "2. Drive"]), `read-only: ${JSON.stringify(read)}`);

await b.close();
process.exitCode = failures ? 1 : 0;
