// Drive's Recently Deleted: listed, Recover and Delete Now each leave the list at once,
// Delete Now asks first and does nothing if declined.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, driveFixture: true });
const page = await ctx.newPage();
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Drive");
await page.waitForTimeout(300);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

await page.getByRole("button", { name: "Recently Deleted" }).click();
await page.waitForTimeout(300);
const names = () => page.locator(".drive-rows .drive-name").allInnerTexts();
ok(JSON.stringify(await names()) === JSON.stringify(["Old notes.pdf", "Wallpaper.jpg"]), `Recently Deleted lists what was deleted  ${JSON.stringify(await names())}`);

const notes = await page.locator("li", { hasText: "Old notes" }).innerText();
ok(/21 days/.test(notes) && /From Downloads/.test(notes), `each shows its time left and where it came from  [${notes.replace(/\s+/g, " ")}]`);
ok((await page.locator("li", { hasText: "Wallpaper" }).locator(".is-soon").count()) === 1, "and one nearly gone is marked so");
await page.locator("li", { hasText: "Old notes" }).getByRole("button", { name: "Recover" }).click();
await page.waitForTimeout(100);
ok(!(await names()).includes("Old notes.pdf"), "Recover takes it off the list at once");

await page.locator("li", { hasText: "Wallpaper" }).getByRole("button", { name: "Delete Now" }).click();
await page.locator(".confirm").getByRole("button", { name: "Cancel" }).click();
await page.waitForTimeout(200);
ok((await names()).includes("Wallpaper.jpg"), "declining Delete Now keeps it");
await page.locator("li", { hasText: "Wallpaper" }).getByRole("button", { name: "Delete Now" }).click();
await page.locator(".confirm .confirm-answer").click();
await page.waitForTimeout(200);
const calls = await page.evaluate(() => window.__trashCalls);
ok(JSON.stringify(calls) === JSON.stringify([["drive_put_back", "FILE::t1"], ["drive_delete_forever", "FILE::t2"]]),
  `each went to Apple once, as asked  ${JSON.stringify(calls)}`);
ok((await page.getByText("Nothing in Recently Deleted.").count()) === 1, "and an empty Recently Deleted says so");
await b.close();
process.exitCode = failures ? 1 : 0;
