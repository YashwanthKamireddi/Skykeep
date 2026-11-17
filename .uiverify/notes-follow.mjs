// The list and the editor always agree: the note highlighted is the note shown, by its id.
//
// A note pasted into and saved, then another opened and typed in — it goes to the top, as an
// edited note does — then a third opened: the editor shows the third note, never the one just
// left, and what is typed there is saved to the third note alone. The same when switching
// straight after typing, before the pause that saves, and with a sync landing in between.
// Titles stay as Apple holds them while an edit leaves the title line alone. Only the edited
// note moves in the list; opening notes moves nothing.
//
// The library here is a small stand-in for the Rust one (notes.rs, outbox.rs): a save keeps
// the text, keeps Apple's title unless the title line changed, and moves the time edited on
// only when something changed. Names and text are invented.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
await ctx.addInitScript(() => {
  const t = window.__TAURI_INTERNALS__;
  const inner = t.invoke;
  const hour = 3600;
  // Three days ago, an hour apart: one date group, newest first A … F.
  const at = Math.floor(Date.now() / 1000) - 3 * 86400;
  const words = (l) => l.replace(/[\uFFFC\u2028]/g, " ").split(/\s+/).filter(Boolean).join(" ");
  const lines = (body) => body.split("\n").map((l) => words(l.replace(/\r$/, ""))).filter((l) => /[\p{L}\p{N}]/u.test(l));
  // Everything up to the end of the title line: an edit that leaves it alone leaves the title.
  const head = (body) => {
    let from = 0;
    for (const l of body.split("\n")) {
      const end = from + l.length;
      if (/[\p{L}\p{N}]/u.test(words(l))) return body.slice(0, end);
      from = end + 1;
    }
    return body;
  };
  const store = [
    ["nA", "Alpine trip\nPack the alpine boots"],
    ["nB", "Birch notes\nBirch bark sample"],
    ["nC", "Cedar list\nCedar planks"],
    ["nD", "Dahlia ideas\nDahlia bulbs"],
    ["nE", "Elm street\nElm leaves"],
    // Apple's own title, as the phone set it, which is not the first line with words on it.
    ["nF", "\u{1F3D5}\u{1F3D5}\u{1F3D5}\nTent\nStove", "\u{1F3D5}\u{1F3D5}\u{1F3D5}"],
  ].map(([guid, body, title], i) => ({
    guid, folderGuid: "f1", title: title ?? lines(body)[0], snippet: lines(body)[1] ?? null, body,
    locked: false, editable: true, pinned: false, createdAt: at - 10 * hour, modifiedAt: at - i * hour, etag: null,
  }));
  window.__fx = store;
  const runsOf = (body) => {
    const ls = body.split("\n");
    return ls.map((l, i) => ({ length: l.length + (i < ls.length - 1 ? 1 : 0), style: i === 0 ? 0 : null, indent: 0,
      checked: null, bold: false, italic: false, underline: false, strike: false, link: null }));
  };
  const savedRuns = {};
  const row = (n) => ({ ...n, body: null });
  t.invoke = async (cmd, args) => {
    const n = store.find((x) => x.guid === args?.guid);
    switch (cmd) {
      case "notes_in": return store.map(row);
      case "note_counts": return [store.length, [["f1", store.length]]];
      case "note_body": await new Promise((r) => setTimeout(r, 15)); return n ? n.body : null;
      case "note_runs": await new Promise((r) => setTimeout(r, 25)); return n ? (savedRuns[n.guid] ?? runsOf(n.body)) : [];
      case "note_row": return n ? row(n) : null;
      case "notes_search": return [];
      case "note_save": {
        (window.__fxSaves ??= []).push({ guid: args.guid, body: args.body });
        if (!n) throw "That note is no longer in the local library.";
        const before = n.body;
        if (args.body !== before || args.runs) {
          if (head(before) !== head(args.body)) n.title = lines(args.body)[0] ?? "";
          n.snippet = lines(args.body)[1] ?? null;
          n.body = args.body;
          if (args.runs) savedRuns[n.guid] = args.runs;
          if (args.body !== before) n.modifiedAt = Math.max(n.modifiedAt ?? 0, Math.floor(Date.now() / 1000));
        }
        return n.body;
      }
      default: return inner(cmd, args);
    }
  };
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.waitForTimeout(500);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

const order = () => page.$$eval(".note-row", (rs) => rs.map((r) => r.getAttribute("data-guid")));
const selected = () => page.$$eval('.note-row[aria-selected="true"]', (rs) => rs.map((r) => r.getAttribute("data-guid")));
const shown = () => page.evaluate(() => document.querySelector(".notes .note-rich")?.textContent ?? "");
const body = (guid) => page.evaluate((g) => window.__fx.find((n) => n.guid === g).body, guid);
const row = (guid) => page.locator(`.note-row[data-guid="${guid}"]`);
const toEnd = async () => {
  await page.locator(".notes .note-rich p").last().click();
  await page.keyboard.press("Control+End");
};
const paste = async (text) => {
  // The editor takes in where the cursor went before the paste lands.
  await page.waitForTimeout(150);
  const handled = await page.evaluate((t) => {
    const el = document.querySelector(".notes .note-rich");
    const dt = new DataTransfer();
    dt.setData("text/plain", t);
    const e = new ClipboardEvent("paste", { clipboardData: dt, bubbles: true, cancelable: true });
    if (!e.clipboardData) return false;
    el.dispatchEvent(e);
    return true;
  }, text);
  if (!handled) await page.keyboard.insertText(text);
};
/** The note on screen is `guid`'s: highlighted, and its own words in the editor. */
const agrees = async (guid, others, label) => {
  const sel = await selected();
  const text = await shown();
  const own = (await body(guid)).split("\n")[1];
  const foreign = [];
  for (const o of others) if (o !== guid && text.includes((await body(o)).split("\n")[1])) foreign.push(o);
  ok(sel.length === 1 && sel[0] === guid && text.includes(own) && foreign.length === 0,
    `${label}: highlighted ${JSON.stringify(sel)}, editor shows ${guid}'s text${foreign.length ? ` — and ${foreign.join(", ")}'s` : ""}  ${JSON.stringify(text.slice(0, 80))}`);
};
const ALL = ["nA", "nB", "nC", "nD", "nE", "nF"];

// 1. Paste into C and let it save; open E and type, let it save (E goes to the top); then the
//    note under E — read ahead as its neighbour — is opened.
await row("nC").click();
await page.waitForTimeout(400);
await toEnd();
await paste("\nPasted line one\nPasted line two");
await page.waitForTimeout(1400);
await row("nE").click();
await page.waitForTimeout(400);
await toEnd();
await page.keyboard.type(" and oaks", { delay: 20 });
await page.waitForTimeout(1400);
const afterE = await order();
ok(afterE[0] === "nE", `the edited note goes to the top  ${JSON.stringify(afterE)}`);
const next = afterE[1];
await row(next).click();
await page.waitForTimeout(400);
await agrees(next, ALL, `opened ${next} after E moved`);
await toEnd();
await page.keyboard.type(" kept", { delay: 20 });
await page.waitForTimeout(1400);
const lastSave = await page.evaluate(() => (window.__fxSaves ?? []).at(-1));
ok(lastSave?.guid === next && !lastSave.body.includes("Elm") && lastSave.body.endsWith(" kept"),
  `typing there saves to ${next} alone, with its own text  ${JSON.stringify(lastSave)}`);
ok((await body("nE")).includes("and oaks") && !(await body("nE")).includes("kept"), "E keeps what was typed in it, and nothing else");
ok((await body("nC")).includes("Pasted line two"), "the paste was saved to C");

// 2. Quickly: typing, then at once a neighbour — before the pause that saves.
await row("nD").click();
await page.waitForTimeout(500);
let o = await order();
const besideD = o[o.indexOf("nD") + 1] ?? o[o.indexOf("nD") - 1];
await toEnd();
await page.keyboard.type(" fast", { delay: 15 });
await row(besideD).click();
await page.waitForTimeout(300);
await agrees(besideD, ALL, `switched to ${besideD} at once after typing in D`);
await page.waitForTimeout(1300);
ok((await body("nD")).endsWith(" fast"), "what was typed in D just before leaving it is saved to D");
ok(!(await body(besideD)).includes("fast"), `and not to ${besideD}`);

// 3. With a sync landing between the typing and the switch.
await row("nB").click();
await page.waitForTimeout(500);
o = await order();
const besideB = o[o.indexOf("nB") + 1] ?? o[o.indexOf("nB") - 1];
await toEnd();
await page.keyboard.type(" synced", { delay: 15 });
await page.evaluate(() => window.__emit("quince://synced", { services: [], asOf: Math.floor(Date.now() / 1000), tookMs: 5 }));
await page.waitForTimeout(60);
await row(besideB).click();
await page.waitForTimeout(80);
await page.evaluate(() => window.__emit("quince://synced", { services: [], asOf: Math.floor(Date.now() / 1000), tookMs: 5 }));
await page.waitForTimeout(400);
await agrees(besideB, ALL, `switched to ${besideB} with syncs landing`);
await page.waitForTimeout(1300);
ok((await body("nB")).endsWith(" synced") && !(await body(besideB)).includes("synced"), "B's typing saved to B alone");

// 4. Opening notes moves nothing, and saves nothing.
const saves0 = await page.evaluate(() => (window.__fxSaves ?? []).length);
const still = await order();
for (const g of [still[3], still[1], still[4], still[2]]) {
  await row(g).click();
  await page.waitForTimeout(250);
}
await page.waitForTimeout(1200);
ok(JSON.stringify(await order()) === JSON.stringify(still), `opening notes moves none of them  ${JSON.stringify(await order())}`);
ok((await page.evaluate(() => (window.__fxSaves ?? []).length)) === saves0, "and saves none of them");

// 5. An edit moves only the edited note, to the top; selection and editor stay with it; a sync
//    after changes nothing.
const lastRow = still.at(-1);
await row(lastRow).click();
await page.waitForTimeout(400);
await toEnd();
await page.keyboard.type(" moved", { delay: 15 });
await page.waitForTimeout(1400);
const moved = await order();
ok(JSON.stringify(moved) === JSON.stringify([lastRow, ...still.filter((g) => g !== lastRow)]),
  `only ${lastRow} moved, to the top; every other row kept its place  ${JSON.stringify(still)} → ${JSON.stringify(moved)}`);
await agrees(lastRow, ALL, "after its move the edited note is still the one highlighted and shown");
await page.evaluate(() => window.__emit("quince://synced", { services: [], asOf: Math.floor(Date.now() / 1000), tookMs: 5 }));
await page.waitForTimeout(500);
ok(JSON.stringify(await order()) === JSON.stringify(moved), "a sync after it moves nothing");

// 6. Titles: an edit below the title line keeps the title Apple holds (the phone's), and an
//    edit of the title line takes the new first line.
await row("nF").click();
await page.waitForTimeout(400);
await toEnd();
await page.keyboard.type(" and lamp", { delay: 15 });
await page.waitForTimeout(1400);
const fTitle = await row("nF").locator(".note-row-name").innerText();
ok(fTitle === "\u{1F3D5}\u{1F3D5}\u{1F3D5}", `an edit below the title keeps Apple's title  (${JSON.stringify(fTitle)})`);
await row("nA").click();
await page.waitForTimeout(400);
await page.locator(".notes .note-rich p").first().click();
await page.keyboard.press("Control+Home");
await page.keyboard.press("Enter");
await page.keyboard.press("Enter");
await page.keyboard.press("ArrowUp");
await page.keyboard.type("Summit plan", { delay: 15 });
// Until the save has landed in the library, however busy the machine: a fixed wait failed under load.
await page.waitForFunction(() => window.__fx.find((n) => n.guid === "nA")?.title === "Summit plan", null, { timeout: 8000 }).catch(() => {});
await page.waitForTimeout(200);
const aTitle = await row("nA").locator(".note-row-name").innerText();
const aStored = await page.evaluate(() => window.__fx.find((n) => n.guid === "nA").title);
ok(aTitle === "Summit plan" && aTitle === aStored, `a new first line is the title, as the library keeps it  (row ${JSON.stringify(aTitle)}, library ${JSON.stringify(aStored)})`);

await b.close();
process.exitCode = failures ? 1 : 0;
