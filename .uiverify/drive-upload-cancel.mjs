import { openScreen } from "./stub.mjs";
// A folder upload shows what it is doing from the first moment — the folder being made, then
// each file as it is found — and every file still waiting or going up can be cancelled, one
// at a time or all at once, ending as "cancelled" rather than vanishing or failing.
import { engine as chromium } from "./engine.mjs";

const b = await chromium.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 820 } });
await ctx.addInitScript(() => {
  const handlers = {};
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  window.__cancels = [];
  window.__TAURI_INTERNALS__ = {
    transformCallback: (cb) => cb,
    convertFileSrc: (p, proto = "asset") => `${proto}://localhost/${encodeURIComponent(p)}`,
    invoke: async (cmd, args) => {
      if (cmd === "auth_status") return { hasStoredSession: true, signedIn: true };
      if (cmd === "auth_resume") return "library";
      if (cmd === "account_services") return ["drivews", "docws"];
      if (cmd === "plugin:event|listen") {
        (handlers[args.event] ??= []).push(args.handler);
        return handlers[args.event].length;
      }
      if (cmd === "plugin:dialog|open") return ["/home/me/Holiday"];
      if (cmd === "pick_open") return ["/home/me/Holiday"];
      if (cmd === "drive_children") return [];
      if (cmd === "drive_item") return null;
      if (cmd === "drive_sync_folder") return 0;
      const say = (e) => (handlers["quince://upload"] ?? []).forEach((h) =>
        h({ event: "quince://upload", id: 1, payload: { within: "", error: null, sent: 0, folder: false, ...e } }));
      if (cmd === "drive_upload_cancel") {
        window.__cancels.push(args);
        for (const id of window.__live ?? []) {
          if (args.ids.includes(id.id) || (args.batch && id.id.startsWith(`${args.batch}:`))) say({ ...id, state: "cancelled" });
        }
        return null;
      }
      if (cmd === "drive_upload") {
        const batch = args.batch;
        say({ id: `${batch}:dir:0`, batch, name: "Holiday", total: 0, state: "preparing", folder: true });
        await wait(300);
        window.__early = document.querySelector(".upload-tray")?.innerText ?? "";
        say({ id: `${batch}:dir:0`, batch, name: "Holiday", total: 0, state: "done", folder: true });
        window.__live = [0, 1, 2].map((n) => ({ id: `${batch}:${n}`, batch, name: `photo ${n}.jpg`, within: "Holiday", total: 1000 }));
        for (const f of window.__live) say({ ...f, state: "waiting" });
        await wait(200);
        window.__found = document.querySelector(".upload-row")?.innerText ?? "";
        // Everything found: from here there is a total.
        say({ id: `${batch}:planned`, batch, name: "", total: 3000, state: "planned", folder: true });
        say({ ...window.__live[0], sent: 300, state: "sending" });
        await wait(150);
        say({ ...window.__live[0], sent: 900, state: "sending" });
        await wait(100);
        window.__ahead = document.querySelector(".upload-tray-percent")?.textContent ?? "";
        // A report for the same file that was overtaken on its way here.
        say({ ...window.__live[0], sent: 300, state: "sending" });
        await wait(100);
        window.__after = document.querySelector(".upload-tray-percent")?.textContent ?? "";
        // Waits for the person: the backend's uploads run until they finish or are stopped.
        await new Promise((r) => (window.__finish = r));
        return [];
      }
      if (cmd === "sync_all" || cmd === "sync_status") return { services: [], asOf: 1, tookMs: 0 };
      return [];
    },
  };
});

const page = await ctx.newPage();
await page.goto("http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Drive");
await page.waitForSelector(".drive-rows", { state: "attached" });
await page.waitForTimeout(300);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

await page.getByRole("button", { name: "Upload", exact: true }).click();
await page.getByRole("menuitem", { name: "Folder" }).click();
await page.waitForTimeout(700);
const early = await page.evaluate(() => window.__early);
ok(/Holiday/.test(early) && /Preparing/.test(early), `a folder upload says what it is doing from the start  [${early.replace(/\s+/g, " ")}]`);
const found = await page.evaluate(() => window.__found);
ok(/Preparing… 3 found/.test(found), `while a folder is still being walked it counts what it has found, not a total  [${found.replace(/\s+/g, " ")}]`);
// One line for the folder, everything in it counted together — not a row per file.
const lines = await page.locator(".upload-row").allInnerTexts();
ok(lines.length === 1 && /Holiday/.test(lines[0]) && /0 of 3/.test(lines[0]), `a folder is one line, its files counted together  ${JSON.stringify(lines)}`);
await page.waitForFunction(() => window.__after !== undefined);
const [ahead, after] = await page.evaluate(() => [window.__ahead, window.__after]);
ok(ahead === "30%" && after === "30%", `progress never slides back when a late report arrives  [${ahead} → ${after}]`);
ok(/Uploading Holiday/.test(await page.locator(".upload-tray-title").innerText()), "the head names the folder going up");

ok((await page.getByRole("button", { name: "Cancel Holiday" }).count()) === 0, "one upload has one Cancel, not two");
await page.locator(".upload-tray").getByRole("button", { name: "Cancel", exact: true }).click();
await page.waitForTimeout(200);
const sent = await page.evaluate(() => window.__cancels);
ok(sent.length === 1 && Boolean(sent[0].batch), `Cancel stops the whole upload  ${JSON.stringify(sent)}`);
ok(/Cancelled/.test(await page.locator(".upload-row").first().innerText()), `the folder then says it was cancelled  [${await page.locator(".upload-row").first().innerText()}]`);
await page.evaluate(() => window.__finish?.());
await b.close();
process.exitCode = failures ? 1 : 0;
