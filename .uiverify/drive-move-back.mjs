// An item moved out of a folder and back again shows in it again, and so does one deleted
// and recovered from Recently Deleted. Each was hidden in the folder it left, by folder and
// item, and that key was never let go: moved back, or recovered, it stayed missing from the
// folder until the app restarted.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, driveFixture: true });
await ctx.addInitScript(() => {
  const ROOT = "FOLDER::com.apple.CloudDocs::root";
  const item = (id, name, kind, extension = null, parentId = ROOT) => ({
    drivewsid: `${kind}::com.apple.CloudDocs::${id}`,
    parentId, name, extension, kind, size: kind === "FILE" ? 2048 : null, childCount: null,
    createdAt: 1787000000, modifiedAt: 1788600000, etag: "e",
  });
  window.__driveIn = {
    [ROOT]: [item("dl", "Downloads", "FOLDER"), item("plan", "Semester Plan", "FILE", "pdf"), item("memo", "Budget", "FILE", "xlsx")],
    "FOLDER::com.apple.CloudDocs::dl": [],
  };
  // Apple's side, as the library has it after each answer: a move takes the item from its
  // folder into the other, a delete puts it in Recently Deleted, a recovery puts it back.
  const wrap = () => {
    const t = window.__TAURI_INTERNALS__;
    if (!t || t.__moving) return;
    const inner = t.invoke;
    t.invoke = async (cmd, args) => {
      const all = () => Object.values(window.__driveIn).flat();
      if (cmd === "drive_move") {
        await new Promise((r) => setTimeout(r, 200));
        const it = all().find((i) => i.drivewsid === args.drivewsid);
        for (const k of Object.keys(window.__driveIn)) window.__driveIn[k] = window.__driveIn[k].filter((i) => i.drivewsid !== args.drivewsid);
        window.__driveIn[args.destinationId].push({ ...it, parentId: args.destinationId });
        return null;
      }
      if (cmd === "drive_delete") {
        const it = all().find((i) => i.drivewsid === args.drivewsid);
        window.__trash = [{ ...it, expiresAt: Math.floor(Date.now() / 1000) + 86400 * 20, restorePath: `${it.name}.${it.extension}` }];
      }
      if (cmd === "drive_trash_out" && args.recover) {
        for (const i of args.items) {
          const { expiresAt: _e, restorePath: _r, ...back } = i;
          window.__driveIn[back.parentId].push(back);
        }
      }
      return inner(cmd, args);
    };
    t.__moving = true;
  };
  wrap(); document.addEventListener("DOMContentLoaded", wrap);
});
const page = await ctx.newPage();
await page.goto(process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Drive");
await page.locator("button.drive-row-hit", { hasText: "Semester Plan" }).waitFor();
let failures = 0;
const ok = (pass, what) => { console.log(`${pass ? "PASS" : "FAIL"} ${what}`); if (!pass) failures++; };
const row = (name) => page.locator("button.drive-row-hit", { hasText: name }).first();
const names = () => page.locator(".drive-rows .drive-name").allInnerTexts();
const inFolder = (name) => page.waitForFunction((n) => [...document.querySelectorAll(".pane-head h1, .drive-title")].some((h) => h.textContent.includes(n)), name, { timeout: 5000 });
const up = async () => {
  await page.locator(".drive-rows").click({ position: { x: 5, y: 5 } }).catch(() => {});
  await page.keyboard.press("Control+ArrowUp");
  await row("Downloads").waitFor();
  await page.waitForTimeout(300);
};
const settled = () => page.waitForFunction(() => !document.querySelector(".drive-rows li.is-busy"), null, { timeout: 4000 }).then(() => page.waitForTimeout(400));

// Out: cut from the top into Downloads (Ctrl-C there, Ctrl-Alt-V here moves it).
await row("Semester Plan").click();
await page.keyboard.press("Control+c");
await row("Downloads").dblclick();
await inFolder("Downloads");
await page.keyboard.press("Control+Alt+v");
await settled();
ok((await names()).includes("Semester Plan.pdf"), `moved into Downloads  ${JSON.stringify(await names())}`);

// And back to where it was.
await row("Semester Plan").click();
await page.keyboard.press("Control+c");
await up();
await page.keyboard.press("Control+Alt+v");
await settled();
ok((await names()).includes("Semester Plan.pdf"), `moved back, it shows in the folder it first left  ${JSON.stringify(await names())}`);
ok((await page.locator(".drive-count").innerText()).startsWith("3 items"), `and is counted there  "${await page.locator(".drive-count").innerText()}"`);

// Deleted, then recovered from Recently Deleted: back in its folder.
await row("Budget").click();
await page.keyboard.press("Delete");
await page.waitForFunction(() => ![...document.querySelectorAll(".drive-rows .drive-name")].some((n) => n.textContent.includes("Budget")), null, { timeout: 3000 });
await page.waitForTimeout(400);
await page.getByRole("button", { name: /Recently Deleted/ }).first().click();
await page.locator(".drive-trash .drive-name", { hasText: "Budget" }).first().waitFor();
await page.locator(".drive-trash li", { hasText: "Budget" }).getByRole("button", { name: "Recover" }).click();
await page.waitForTimeout(300);
await page.locator(".drive-trash .crumb", { hasText: "iCloud Drive" }).click();
await row("Downloads").waitFor();
await page.waitForTimeout(600);
ok((await names()).includes("Budget.xlsx"), `recovered, it shows in its folder again  ${JSON.stringify(await names())}`);

await b.close();
process.exitCode = failures ? 1 : 0;
