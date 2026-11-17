// Drive delete of a folder that changed on another device since it was shown: not deleted
// silently — asked, with what the folder holds now and what was added; kept on Cancel; on
// "Delete Anyway" deleted as the version just shown. An unchanged folder asks nothing.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

async function open(changed) {
  const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, driveFixture: true });
  if (changed) await ctx.addInitScript(() => {
    window.__driveChanged = { "FOLDER::a": { items: 12, added: 3, changed: 0, gone: 0, etag: "w9" } };
  });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
  await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
  await openScreen(page, "Drive");
  await page.waitForTimeout(300);
  return { ctx, page };
}
const del = async (page) => {
  await page.getByRole("button", { name: "Actions for Downloads" }).click();
  await page.getByRole("menuitem", { name: "Delete" }).click();
  await page.waitForTimeout(250);
};
const row = (page) => page.locator(".drive-rows .drive-name", { hasText: "Downloads" }).count();
const sheet = (page) => page.getByRole("alertdialog").or(page.getByRole("dialog"));

{
  const { ctx, page } = await open(false);
  await del(page);
  ok((await sheet(page).count()) === 0, "an unchanged folder is deleted with no question");
  ok((await page.evaluate(() => window.__driveDeleted ?? [])).includes("FOLDER::a"), "and it is deleted");
  await ctx.close();
}
{
  const { ctx, page } = await open(true);
  await del(page);
  const text = (await sheet(page).count()) ? await sheet(page).first().innerText() : "";
  ok(/now has 12 items, with 3 items added since it was shown here/.test(text), `a changed folder is asked about as it now stands  ${JSON.stringify(text)}`);
  ok(!(await page.evaluate(() => window.__driveDeleted ?? [])).includes("FOLDER::a"), "nothing deleted before the answer");
  await page.getByRole("button", { name: "Cancel" }).click();
  await page.waitForTimeout(250);
  ok((await row(page)) === 1, "Cancel keeps it, back in its place");
  ok(!(await page.locator(".drive-error, [role=alert]").filter({ hasText: /kept/ }).count()), "and says no error for it");

  await del(page);
  await page.getByRole("button", { name: "Delete Anyway" }).click();
  await page.waitForTimeout(300);
  ok((await page.evaluate(() => window.__driveDeleted ?? [])).includes("FOLDER::a"), "Delete Anyway deletes the version just shown");
  ok((await row(page)) === 0, "and it leaves");
  await ctx.close();
}
await b.close();
if (failures) process.exitCode = 1;
