// Copy and Paste in Drive, as Finder: Ctrl-C on what is chosen, Ctrl-V in another folder makes
// copies there — shown at once under the name Finder gives a copy, gone again with the reason if
// Apple refuses; Ctrl-Alt-V moves instead. Files copied in the desktop's file manager paste as an
// upload. The menus have Copy, and a right-click on nothing has Paste.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, driveFixture: true });
await ctx.addInitScript(() => {
  const ROOT = "FOLDER::com.apple.CloudDocs::root";
  const item = (id, name, kind, extension = null, parentId = ROOT) => ({
    drivewsid: `${kind === "APP_LIBRARY" ? "APPLIBRARY::com.apple.Keynote" : `${kind}::com.apple.CloudDocs`}::${id}`,
    parentId, name, extension, kind, size: kind === "FILE" ? 2048 : null, childCount: null,
    createdAt: 1787000000, modifiedAt: 1788600000, etag: "e",
  });
  window.__driveIn = {
    [ROOT]: [
      item("dl", "Downloads", "FOLDER"),
      item("pr", "Projects", "FOLDER"),
      item("plan", "Semester Plan", "FILE", "pdf"),
      item("kn", "Keynote", "APP_LIBRARY"),
    ],
    "FOLDER::com.apple.CloudDocs::dl": [],
    "FOLDER::com.apple.CloudDocs::pr": [],
  };
});
const page = await ctx.newPage();
await page.goto(process.env.BASE ?? process.env.URL ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Drive");
await page.locator("button.drive-row-hit", { hasText: "Semester Plan" }).waitFor();
let failures = 0;
const ok = (pass, what) => { console.log(`${pass ? "PASS" : "FAIL"} ${what}`); if (!pass) failures++; };
const PLAN = "FILE::com.apple.CloudDocs::plan";
const DL = "FOLDER::com.apple.CloudDocs::dl";
const PR = "FOLDER::com.apple.CloudDocs::pr";
const row = (name) => page.locator("button.drive-row-hit", { hasText: name }).first();
const names = () => page.locator(".drive-rows .drive-name").allInnerTexts();
const standIn = (name) => page.locator(".drive-rows li.is-busy", { hasText: name });
const copies = () => page.evaluate(() => window.__driveCopies ?? []);
const inFolder = (name) => page.waitForFunction((n) => [...document.querySelectorAll(".pane-head h1, .drive-title")].some((h) => h.textContent.includes(n)), name, { timeout: 5000 });
const errorShown = (text) => page.locator(".toast.is-error", { hasText: text }).waitFor({ timeout: 3000 }).then(() => true, () => false);
const dismissErrors = async () => {
  for (const close of await page.locator(".toast.is-error button").all()) await close.click().catch(() => {});
};
const open = async (name) => {
  await row(name).dblclick();
  await inFolder(name);
  await page.waitForTimeout(100);
};
const up = async () => {
  await page.locator(".drive-rows").click({ position: { x: 5, y: 5 } }).catch(() => {});
  await page.keyboard.press("Control+ArrowUp");
  await row("Semester Plan").waitFor();
};

// Copy in one folder, paste in another: the copy shows at once, under its own name.
await row("Semester Plan").click();
await page.keyboard.press("Control+c");
await open("Downloads");
await page.evaluate(() => { window.__driveCopyDelay = 500; });
const pressed = Date.now();
await page.keyboard.press("Control+v");
await standIn("Semester Plan.pdf").waitFor({ timeout: 1000 });
ok(Date.now() - pressed < 450, `the copy shows before Apple answers  ${Date.now() - pressed}ms`);
ok((await standIn("Semester Plan.pdf").innerText()).includes("Copying"), "and says it is being copied");
await page.waitForFunction(() => !document.querySelector(".drive-rows li.is-busy"), null, { timeout: 4000 });
const first = (await copies()).at(-1);
ok(first?.drivewsid === PLAN && first?.destinationId === DL, `Apple is asked to copy it here  ${JSON.stringify(first)}`);
ok((await names()).includes("Semester Plan.pdf"), "the copy is in the folder once Apple has it");

// Pasted again where its name is taken: Finder's question, and Keep Both names it "Semester
// Plan 2.pdf", then "Semester Plan 3.pdf" (drive-paste-clash.mjs has the rest of it).
const keepBoth = () => page.getByRole("alertdialog").getByRole("button", { name: "Keep Both" }).click();
await page.keyboard.press("Control+v");
await keepBoth();
await standIn("Semester Plan 2.pdf").waitFor({ timeout: 1000 }).then(() => ok(true, "a second paste, both kept, is named “Semester Plan 2.pdf”"), () => ok(false, "a second paste, both kept, is named “Semester Plan 2.pdf”"));
await page.waitForFunction(() => !document.querySelector(".drive-rows li.is-busy"), null, { timeout: 4000 });
await page.keyboard.press("Control+v");
await keepBoth();
await standIn("Semester Plan 3.pdf").waitFor({ timeout: 1000 }).then(() => ok(true, "a third, “Semester Plan 3.pdf”"), () => ok(false, "a third, “Semester Plan 3.pdf”"));
await page.waitForFunction(() => !document.querySelector(".drive-rows li.is-busy"), null, { timeout: 4000 });
ok((await copies()).length === 3, "each paste is one copy");

// Refused: the row that showed goes again, and the reason is said.
await page.evaluate(() => { window.__driveCopyFail = "iCloud Drive turned this down."; });
await page.keyboard.press("Control+v");
await keepBoth();
await standIn("Semester Plan 4.pdf").waitFor({ timeout: 1000 }).catch(() => {});
ok(await errorShown("turned this down"), "a copy Apple refuses says why");
ok(!(await names()).includes("Semester Plan 4.pdf"), "and its row is gone again");
await page.evaluate(() => { window.__driveCopyFail = null; window.__driveCopyDelay = 0; });
await dismissErrors();

// Files copied in the file manager paste as an upload into the folder open.
await page.evaluate(() => { window.__clipFiles = ["/home/alex/Pictures/Lake.jpg", "/home/alex/Notes.txt"]; });
await page.keyboard.press("Control+v");
await page.waitForFunction(() => (window.__driveUploads ?? []).length === 1, null, { timeout: 3000 }).catch(() => {});
const sent = await page.evaluate(() => window.__driveUploads?.[0]);
ok(sent?.folderId === DL && sent?.paths?.length === 2, `the file manager's files go up into this folder  ${JSON.stringify(sent)}`);
ok((await copies()).length === 4, "and nothing of Drive's is copied for them");
await page.evaluate(() => { window.__clipFiles = []; });

// Ctrl-Alt-V moves what was copied here instead: gone from where it was, shown here at once.
await up();
await row("Semester Plan").click();
await page.keyboard.press("Control+c");
await open("Projects");
await page.keyboard.press("Control+Alt+v");
await standIn("Semester Plan.pdf").waitFor({ timeout: 400 }).then(() => ok(true, "Ctrl-Alt-V shows it here at once"), () => ok(false, "Ctrl-Alt-V shows it here at once"));
ok((await standIn("Semester Plan.pdf").innerText().catch(() => "")).includes("Moving"), "as being moved");
await page.waitForFunction(() => (window.__moves ?? []).length === 1, null, { timeout: 3000 }).catch(() => {});
const moved = await page.evaluate(() => window.__moves?.[0]);
ok(moved?.drivewsid === PLAN && moved?.destinationId === PR, `Apple is asked to move it here  ${JSON.stringify(moved)}`);
ok((await copies()).length === 4, "and nothing is copied");
await page.waitForFunction(() => !document.querySelector(".drive-rows li.is-busy"), null, { timeout: 4000 });
await page.keyboard.press("Control+ArrowUp");
await row("Downloads").waitFor();
await page.waitForTimeout(150);
ok(!(await names()).includes("Semester Plan.pdf"), "and it has left the folder it was in");
// Moved, it is not on the clipboard to paste again.
await open("Downloads");
await page.keyboard.press("Control+Alt+v");
await page.waitForTimeout(300);
ok((await page.evaluate(() => (window.__moves ?? []).length)) === 1, "a move pastes once");
await up().catch(() => {});

// A folder is never pasted into itself.
await page.locator("button.drive-row-hit", { hasText: "Downloads" }).waitFor();
await row("Downloads").click();
await page.keyboard.press("Control+c");
await open("Downloads");
await page.keyboard.press("Control+v");
ok(await errorShown("inside itself"), "a folder pasted into itself is refused, as Finder refuses it");
ok((await copies()).length === 4, "and nothing is asked of Apple");
await dismissErrors();
await page.keyboard.press("Control+ArrowUp");
await row("Projects").waitFor();

// The menus: Copy on an item (not on an app's library), Paste on a right-click on nothing.
await row("Keynote").click({ button: "right" });
ok(!(await page.locator(".row-menu [role='menuitem']", { hasText: /^Copy$/ }).count()), "an app's library has no Copy");
await page.keyboard.press("Escape");
await row("Projects").click({ button: "right" });
await page.locator(".row-menu [role='menuitem']", { hasText: /^Copy$/ }).click();
await page.evaluate(() => { window.__driveCopyDelay = 600; window.__driveCopyName = "Projects copy"; });
const box = await page.locator(".drive-rows").boundingBox();
await page.mouse.click(box.x + box.width / 2, Math.min(780, box.y + box.height + 60), { button: "right" });
const paste = page.locator(".row-menu [role='menuitem']", { hasText: /^Paste$/ });
await paste.waitFor({ timeout: 2000 }).catch(() => {});
ok((await paste.count()) === 1, "a right-click on nothing offers Paste");
await paste.click().catch(() => {});
await page.waitForFunction(() => (window.__driveCopies ?? []).length === 5, null, { timeout: 3000 }).catch(() => {});
const viaMenu = (await copies()).at(-1);
ok(viaMenu?.drivewsid === PR && viaMenu?.destinationId === "FOLDER::com.apple.CloudDocs::root", `Paste from the menu copies the folder here  ${JSON.stringify(viaMenu)}`);
await standIn("Projects copy").waitFor({ timeout: 1000 }).then(() => ok(true, "named “Projects copy” beside itself"), () => ok(false, "named “Projects copy” beside itself"));

await b.close();
process.exitCode = failures ? 1 : 0;
