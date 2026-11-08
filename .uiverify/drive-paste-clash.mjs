// Pasting onto a name the folder already has, as Finder: a question first — Keep Both names the
// one pasted "Name 2", Replace sends the one there to Recently Deleted, Cancel leaves both as
// they were — for a copy (Ctrl-V) and for a move (Ctrl-Alt-V). Never overwritten silently, never
// refused. Duplicate (Ctrl-D) and a paste where it was copied name the copy alike: "Name copy".
//
//   URL=http://localhost:4173 ENGINE=webkit node .uiverify/drive-paste-clash.mjs
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, driveFixture: true });
await ctx.addInitScript(() => {
  const ROOT = "FOLDER::com.apple.CloudDocs::root";
  const DL = "FOLDER::com.apple.CloudDocs::dl";
  const item = (id, name, kind, extension = null, parentId = ROOT) => ({
    drivewsid: `${kind}::com.apple.CloudDocs::${id}`, parentId, name, extension, kind,
    size: kind === "FILE" ? 2048 : null, childCount: null, createdAt: 1787000000, modifiedAt: 1788600000, etag: "e",
  });
  window.__driveIn = {
    [ROOT]: [item("dl", "Downloads", "FOLDER"), item("plan", "Semester Plan", "FILE", "pdf"), item("notes", "Notes", "FILE", "txt")],
    [DL]: [item("old", "Semester Plan", "FILE", "pdf", DL), item("budget", "Budget", "FILE", "xlsx", DL)],
  };
});
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Drive");
await page.locator("button.drive-row-hit", { hasText: "Semester Plan" }).waitFor();
let failures = 0;
const ok = (pass, what) => { console.log(`${pass ? "PASS" : "FAIL"} ${what}`); if (!pass) failures++; };
const ROOT = "FOLDER::com.apple.CloudDocs::root";
const DL = "FOLDER::com.apple.CloudDocs::dl";
const PLAN = "FILE::com.apple.CloudDocs::plan";
const row = (name) => page.locator("button.drive-row-hit", { hasText: name }).first();
const names = () => page.locator(".drive-rows .drive-name").allInnerTexts();
const standIn = (name) => page.locator(".drive-rows li.is-busy", { hasText: name });
const settled = () => page.waitForFunction(() => !document.querySelector(".drive-rows li.is-busy"), null, { timeout: 4000 });
const got = (key) => page.evaluate((k) => window[k] ?? [], key);
const sheet = page.getByRole("alertdialog");
const answer = (name) => sheet.getByRole("button", { name, exact: true }).click();
const asked = () => sheet.waitFor({ timeout: 1500 }).then(() => true, () => false);

await row("Semester Plan").click();
await page.keyboard.press("Control+c");
await row("Downloads").dblclick();
await row("Budget").waitFor();
await page.waitForTimeout(100);

// Ctrl-V onto "Semester Plan.pdf": asked first, nothing done until answered.
await page.keyboard.press("Control+v");
ok(await asked(), "a paste onto a name already here asks first");
const title = await sheet.locator("h2").innerText().catch(() => "");
ok(/Semester Plan\.pdf/.test(title), `naming the item  (${title})`);
const buttons = await sheet.getByRole("button").allInnerTexts();
ok(["Keep Both", "Replace", "Cancel"].every((n) => buttons.includes(n)), `with Keep Both, Replace and Cancel  ${JSON.stringify(buttons)}`);
ok((await got("__driveCopies")).length === 0, "and nothing is copied while it asks");

// Cancel: nothing.
await answer("Cancel");
await page.waitForTimeout(200);
ok((await got("__driveCopies")).length === 0 && (await names()).filter((n) => n === "Semester Plan.pdf").length === 1, "Cancel leaves the folder as it was");

// Keep Both: copied, and named "Semester Plan 2.pdf".
await page.evaluate(() => { window.__driveCopyDelay = 400; });
await page.keyboard.press("Control+v");
await asked();
await answer("Keep Both");
await standIn("Semester Plan 2.pdf").waitFor({ timeout: 1000 }).then(() => ok(true, "Keep Both shows the copy at once as “Semester Plan 2.pdf”"), () => ok(false, "Keep Both shows the copy at once as “Semester Plan 2.pdf”"));
await settled();
await page.waitForTimeout(200);
const renames = await got("__driveNames");
ok(renames.at(-1)?.name === "Semester Plan 2.pdf", `and names the real copy so  ${JSON.stringify(renames.at(-1))}`);
const kept = await names();
ok(kept.includes("Semester Plan.pdf") && kept.includes("Semester Plan 2.pdf"), `both are in the folder  ${JSON.stringify(kept)}`);
ok(!(await got("__driveDeleted")).length, "and nothing was deleted");

// Replace: the one here to Recently Deleted, the copy takes its name.
await page.keyboard.press("Control+v");
await asked();
await answer("Replace");
await standIn("Semester Plan.pdf").waitFor({ timeout: 1000 }).then(() => ok(true, "Replace shows the copy at once where the one there was"), () => ok(false, "Replace shows the copy at once where the one there was"));
await settled();
await page.waitForTimeout(200);
ok(JSON.stringify(await got("__driveDeleted")) === JSON.stringify(["FILE::com.apple.CloudDocs::old"]), `Replace sends the one here to Recently Deleted  ${JSON.stringify(await got("__driveDeleted"))}`);
ok((await got("__driveCopies")).length === 2, "and copies the pasted one");
const replaced = await names();
ok(replaced.filter((n) => n === "Semester Plan.pdf").length === 1 && !replaced.includes("Semester Plan copy.pdf"), `under its own name  ${JSON.stringify(replaced)}`);

// Ctrl-Alt-V onto a name already here: asked too, and Keep Both moves it as "Semester Plan 3.pdf".
await page.keyboard.press("Control+Alt+v");
ok(await asked(), "a move onto a name already here asks first, rather than refusing");
await answer("Keep Both");
await page.waitForFunction(() => (window.__moves ?? []).length === 1, null, { timeout: 3000 }).catch(() => {});
const moved = (await got("__moves"))[0];
const named = (await got("__driveNames")).at(-1);
ok(moved?.drivewsid === PLAN && moved?.destinationId === DL, `and moves it here  ${JSON.stringify(moved)}`);
ok(named?.drivewsid === PLAN && named?.name === "Semester Plan 3.pdf", `named “Semester Plan 3.pdf”  ${JSON.stringify(named)}`);
await settled().catch(() => {});

// Duplicate and a paste where it was copied: the same name rule, "Notes copy.txt" then "copy 2".
await page.locator(".drive-rows").click({ position: { x: 5, y: 5 } }).catch(() => {});
await page.keyboard.press("Control+ArrowUp");
await row("Notes").waitFor();
await row("Notes").click();
await page.keyboard.press("Control+d");
await standIn("Notes copy.txt").waitFor({ timeout: 1000 }).then(() => ok(true, "Ctrl-D shows the copy as “Notes copy.txt”"), () => ok(false, "Ctrl-D shows the copy as “Notes copy.txt”"));
const dup = (await got("__driveCopies")).at(-1);
ok(dup?.drivewsid === "FILE::com.apple.CloudDocs::notes" && dup?.destinationId === ROOT, `made as a copy beside itself, the way Paste makes one  ${JSON.stringify(dup)}`);
await settled();
await page.waitForTimeout(150);
ok((await names()).includes("Notes copy.txt"), "and it is “Notes copy.txt” once made");
await row("Notes").click();
await page.keyboard.press("Control+c");
await page.keyboard.press("Control+v");
await standIn("Notes copy 2.txt").waitFor({ timeout: 1000 }).then(() => ok(true, "and is named “Notes copy 2.txt”, as a second duplicate would be"), () => ok(false, "and is named “Notes copy 2.txt”, as a second duplicate would be"));
ok(await sheet.count() === 0, "a paste where it was copied does not ask");
await settled();

await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
