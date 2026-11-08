// Recents: the files opened lately, under Today, Yesterday and on back, as icloud.com and
// Finder group them; a double-click saves one — even one in a folder never opened here.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, driveFixture: true });
await ctx.addInitScript(() => {
  const t = setInterval(() => {
    const i = window.__TAURI_INTERNALS__;
    if (!i || i.__r) return;
    i.__r = true;
    const inner = i.invoke;
    i.invoke = async (cmd, args) => {
      if (cmd === "drive_download") { (window.__downloads ??= []).push(args); return `/home/test/Downloads/${args.item?.name ?? "x"}.pdf`; }
      return inner(cmd, args);
    };
    clearInterval(t);
  }, 0);
});
const page = await ctx.newPage();
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Drive");
await page.waitForTimeout(300);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
await page.getByRole("button", { name: "Recents" }).click();
await page.waitForTimeout(300);
const headings = await page.locator(".drive-group").allInnerTexts();
ok(JSON.stringify(headings.slice(0, 2)) === JSON.stringify(["Today", "Yesterday"]) && headings.length === 3, `grouped by when they were opened  ${JSON.stringify(headings)}`);
await page.locator("button.drive-row-hit", { hasText: "Invoice" }).dblclick();
await page.waitForTimeout(200);
const sent = await page.evaluate(() => window.__downloads?.at(-1));
ok(sent?.drivewsid === "FILE::r1" && sent?.item?.name === "Invoice-0001", "a double-click saves it, with the file's details sent along");
ok(/Saved to Downloads/.test(await page.locator(".toast").innerText()) && (await page.locator(".toast").getByRole("button", { name: "Show in Folder" }).count()) === 1, "and says where, with a way to see it");
await page.locator(".crumb", { hasText: "Drive" }).click();
ok((await page.locator(".drive-group").count()) === 0, "the breadcrumb goes back to iCloud Drive");
await b.close();
process.exitCode = failures ? 1 : 0;
