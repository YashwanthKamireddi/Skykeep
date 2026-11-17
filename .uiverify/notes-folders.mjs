// Notes folders, as Notes manages them: New Folder appears ready to be named, ⋯ and a
// right-click offer Rename / New Folder / Delete Folder, and Delete asks Apple's question.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, notesFixture: true });
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Notes");
await page.waitForTimeout(300);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const names = () => page.locator(".notes-folders .notes-folder-name").allInnerTexts();

await page.getByRole("button", { name: "New Folder" }).click();
await page.waitForTimeout(300);
const field = page.getByRole("textbox", { name: "Rename New Folder" });
ok((await field.count()) === 1, "New Folder appears, ready to be named");
await field.fill("Recipes");
await field.press("Enter");
await page.waitForTimeout(250);
ok((await names()).includes("Recipes"), `it takes its name  ${JSON.stringify(await names())}`);

await page.locator(".notes-folder-row", { hasText: "Recipes" }).click({ button: "right" });
await page.waitForTimeout(150);
const offered = await page.getByRole("menuitem").allInnerTexts();
ok(JSON.stringify(offered) === JSON.stringify(["Rename Folder", "New Folder", "Delete Folder"]), `a folder's menu  ${JSON.stringify(offered)}`);
await page.getByRole("menuitem", { name: "Delete Folder" }).click();
const question = await page.locator(".confirm").innerText();
await page.locator(".confirm .confirm-answer").click();
await page.waitForTimeout(250);
ok(question.includes("All notes and any subfolders will be deleted."), "Delete Folder asks Apple's question");
ok(!(await names()).includes("Recipes"), "and the folder is gone");
const calls = await page.evaluate(() => window.__folderCalls);
ok(calls?.[0]?.[0] === "create" && calls?.[1]?.[0] === "rename" && calls?.[1]?.[2] === "Recipes" && calls?.[2]?.[0] === "delete",
  `each went to Apple once  ${JSON.stringify(calls)}`);
await b.close();
process.exitCode = failures ? 1 : 0;
