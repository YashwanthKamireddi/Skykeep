import { openScreen } from "./stub.mjs";
// Writing a new note the way a person does: New Note, type, pause, keep typing.
//
// The pause is where it went wrong. The box being typed into was swapped for another when
// the note was created, so for a moment nothing was open ("Select a note"), focus fell onto
// the window, and the box that went away could create the note a second time — three
// "Untitled" notes from one. This runs it in a real browser against a slow create.
import { engine as chromium } from "./engine.mjs";

const b = await chromium.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 820 } });
await ctx.addInitScript(() => {
  // WebKit, which the app runs on, blurs a focused field as it is removed; Chromium does not.
  const remove = Node.prototype.removeChild;
  Node.prototype.removeChild = function (child) {
    const focused = document.activeElement;
    if (focused && child.contains(focused)) {
      focused.dispatchEvent(new FocusEvent("focusout", { bubbles: true }));
    }
    return remove.call(this, child);
  };
  const notes = [];
  const bodies = {};
  window.__creates = [];
  window.__saves = [];
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  window.__TAURI_INTERNALS__ = {
    transformCallback: (cb) => cb,
    invoke: async (cmd, args) => {
      if (cmd === "auth_status") return { hasStoredSession: true, signedIn: true };
      if (cmd === "auth_resume") return "library";
      if (cmd === "account_services") return ["notes", "ckdatabasews"];
      if (cmd === "note_folders") return [];
      if (cmd === "notes_in") return notes.slice();
      if (cmd === "note_body") return bodies[args.guid] ?? null;
      if (cmd === "note_create") {
        window.__creates.push(args.body);
        await wait(400);
        const guid = `N${window.__creates.length}`;
        const [title, ...rest] = args.body.split("\n");
        notes.unshift({ guid, folderGuid: null, title, snippet: rest.join(" "), body: null,
          locked: false, pinned: false, createdAt: 1, modifiedAt: 1, editable: true, etag: "e" });
        bodies[guid] = args.body;
        return guid;
      }
      if (cmd === "note_save") {
        window.__saves.push([args.guid, args.body]);
        bodies[args.guid] = args.body;
        return null;
      }
      if (cmd === "sync_all" || cmd === "sync_status") return { services: [], asOf: 1, tookMs: 0 };
      if (cmd === "plugin:event|listen") return 0;
      return [];
    },
  };
});

const page = await ctx.newPage();
await page.goto(process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.waitForTimeout(300);

// Watch for the moment nothing is open, and for focus leaving the editor.
await page.evaluate(() => {
  window.__blank = 0;
  window.__lostFocus = 0;
  new MutationObserver(() => {
    if (document.querySelector(".note-body .note-pick")?.textContent?.includes("Select a note")) {
      window.__blank++;
    }
  }).observe(document.body, { subtree: true, childList: true, characterData: true });
});

await page.getByRole("button", { name: "New note" }).click();
await page.waitForTimeout(150);
await page.keyboard.type("Shopping");
await page.waitForTimeout(1600); // the pause: long enough to create the note
// Return after the title goes on to the body, as in Notes.
await page.keyboard.type("\nmilk and bread");
await page.waitForTimeout(1600); // and long enough to save what followed

// The editor is one document; its first line is the title.
const text = () => page.evaluate(() => {
  const lines = [...document.querySelectorAll(".note-editor .note-rich .nf-p")].map((p) => p.textContent);
  return { title: lines[0] ?? null, rest: lines.length ? lines.slice(1).join("\n") : null,
    focused: Boolean(document.activeElement?.closest(".note-editor")) };
});
const after = await text();
const result = await page.evaluate(() => ({
  creates: window.__creates.length,
  blank: window.__blank,
  saves: window.__saves,
}));

let failures = 0;
const ok = (pass, yes, no) => {
  console.log(pass ? `PASS ${yes}` : `FAIL ${no}`);
  if (!pass) failures++;
};
ok(result.creates === 1, "one note is created, not several", `${result.creates} notes were created`);
ok(after.focused, "typing stays in the note throughout", "focus fell out of the note");
ok(result.blank === 0, "the note never disappears into “Select a note”", `“Select a note” showed ${result.blank} time(s)`);
ok(after.title === "Shopping" && after.rest === "milk and bread",
  "the first line is the title and Enter carries on into the body",
  `title ${JSON.stringify(after.title)}, body ${JSON.stringify(after.rest)}`);
ok(result.saves.some(([g, body]) => g === "N1" && body === "Shopping\nmilk and bread"),
  "what was typed after the pause is saved onto that note", `saves ${JSON.stringify(result.saves)}`);

// Backspace at the start of the body joins it back onto the title, as in Notes.
await page.locator(".note-rich .nf-p").nth(1).click();
await page.waitForTimeout(50);
await page.keyboard.press("Home");
await page.keyboard.press("Backspace");
await page.waitForTimeout(150);
const joined = await text();
ok(joined.title === "Shoppingmilk and bread" && joined.rest === "",
  "Backspace at the start of the body joins it to the title", `after Backspace: ${JSON.stringify(joined)}`);

// Enter in the middle of the title splits it there.
await page.locator(".note-rich .nf-p").first().click();
await page.evaluate(() => {
  // The caret after "Shopping", placed exactly.
  const text = document.querySelector(".note-rich .nf-p").firstChild;
  const r = document.createRange();
  r.setStart(text, 8);
  r.collapse(true);
  const sel = window.getSelection();
  sel.removeAllRanges();
  sel.addRange(r);
});
await page.waitForTimeout(80);
await page.keyboard.press("Enter");
await page.waitForTimeout(150);
const split = await text();
ok(split.title === "Shopping" && split.rest === "milk and bread",
  "Enter in the middle of the title splits the line there", `after Enter: ${JSON.stringify(split)}`);
await b.close();
process.exitCode = failures === 0 ? 0 : 1;
