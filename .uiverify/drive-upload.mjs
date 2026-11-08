import { openScreen } from "./stub.mjs";
// Uploading from Drive: each file named, with its own progress, and a verdict per file.
//
// "Uploading…" on the button said nothing about which of forty photos had gone. The tray must
// show each one moving, call it uploaded only when the backend has proven it intact, and name
// the reason for any that did not make it — while the window stays usable.
import { engine as chromium } from "./engine.mjs";

const b = await chromium.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 820 } });
await ctx.addInitScript(() => {
  const handlers = {};
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  window.__uploadCalls = [];
  window.__TAURI_INTERNALS__ = {
    transformCallback: (cb) => cb,
    invoke: async (cmd, args) => {
      if (cmd === "auth_status") return { hasStoredSession: true, signedIn: true };
      if (cmd === "auth_resume") return "library";
      if (cmd === "account_services") return ["drivews", "docws"];
      if (cmd === "plugin:event|listen") {
        (handlers[args.event] ??= []).push(args.handler);
        return handlers[args.event].length;
      }
      if (cmd === "pick_open")
        return args.directory ? ["/home/me/Holiday"] : ["/home/me/a.jpg", "/home/me/b.jpg"];
      if (cmd === "plugin:dialog|open")
        return args.options?.directory ? ["/home/me/Holiday"] : ["/home/me/a.jpg", "/home/me/b.jpg"];
      if (cmd === "drive_children") return [];
      if (cmd === "drive_item") return null;
      if (cmd === "drive_sync_folder") return 0;
      if (cmd === "drive_upload") {
        window.__uploadCalls.push(args);
        const say = (e) => (handlers["quince://upload"] ?? []).forEach((h) =>
          h({ event: "quince://upload", id: 1, payload: { batch: args.batch, within: "", error: null, ...e } }));
        const a = { id: `${args.batch}:0`, name: "a.jpg", total: 1000 };
        const c = { id: `${args.batch}:1`, name: "b.jpg", total: 500 };
        say({ ...a, sent: 0, state: "waiting" });
        say({ ...c, sent: 0, state: "waiting" });
        await wait(150);
        say({ ...a, sent: 500, state: "sending" });
        await wait(400);
        window.__midway = document.querySelector(".upload-tray")?.innerText ?? "";
        window.__midwayBars = [...document.querySelectorAll(".upload-bar:not(.is-waiting) > span")].map((b) => [b.style.transform, b.style.width]);
        say({ ...a, sent: 1000, state: "verifying" });
        await wait(100);
        say({ ...a, sent: 1000, state: "done" });
        say({ ...c, sent: 500, state: "failed", error: "the bytes Apple received for b.jpg are not the file's, so it was not filed" });
        return ["b.jpg: not filed"];
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
const ok = (pass, yes, no) => {
  console.log(pass ? `PASS ${yes}` : `FAIL ${no}`);
  if (!pass) failures++;
};

await page.getByRole("button", { name: "Upload", exact: true }).click();
const items = await page.getByRole("menuitem").allInnerTexts();
ok(
  JSON.stringify(items) === JSON.stringify(["Files", "Folder"]),
  "Upload offers files or a whole folder",
  `Upload offered ${JSON.stringify(items)}`
);
await page.getByRole("menuitem", { name: "Files" }).click();
await page.waitForTimeout(1200);

const midway = await page.evaluate(() => window.__midway);
ok(/a\.jpg/.test(midway) && /50%/.test(midway) && /b\.jpg/.test(midway) && /Waiting/.test(midway),
  "while going up, each file shows by name with its own progress",
  `midway the tray said ${JSON.stringify(midway)}`);

// The bars grow by scale on the compositor, never by width: a width laid the tray out on every
// step, and coming back to the window replayed every step at once as a glitch.
const bars = await page.evaluate(() => window.__midwayBars ?? []);
ok(bars.length > 0 && bars.every(([t, w]) => /^scaleX\(/.test(t) && !w), "progress bars grow by scale, not width", `bars were ${JSON.stringify(bars)}`);

const rows = await page.locator(".upload-row").evaluateAll((els) =>
  els.map((e) => ({ cls: e.className, text: e.innerText.replace(/\s+/g, " ") })));
ok(rows.length === 2 && /is-done/.test(rows[0].cls) && /is-failed/.test(rows[1].cls) &&
  /not filed/.test(rows[1].text),
  "each file ends with its own verdict, and a failure says why",
  `rows ended as ${JSON.stringify(rows)}`);
const head = await page.locator(".upload-tray-title").innerText();
ok(head === "Some uploads didn’t finish", "the tray sums it up honestly", `the tray said "${head}"`);

const usable = await page.getByRole("button", { name: "Upload", exact: true }).isEnabled();
ok(usable, "the window stays usable, and another upload can start", "Upload was left disabled");

await page.getByRole("button", { name: "Upload", exact: true }).click();
await page.getByRole("menuitem", { name: "Folder" }).click();
await page.waitForTimeout(1000);
const calls = await page.evaluate(() => window.__uploadCalls.map((c) => c.paths));
ok(JSON.stringify(calls[1]) === JSON.stringify(["/home/me/Holiday"]),
  "a chosen folder is handed over whole", `the folder upload sent ${JSON.stringify(calls)}`);

// An app's library (Obsidian's vault, Shortcuts') takes files from this computer too, filed in
// its own zone by the backend (rclone/rclone#9729): Upload is offered there, and sends into it.
{
  const VAULT = "FOLDER::iCloud.md.obsidian::documents";
  const appCtx = await b.newContext({ viewport: { width: 1280, height: 820 } });
  await appCtx.addInitScript((vault) => {
    window.__uploadCalls = [];
    window.__TAURI_INTERNALS__ = {
      transformCallback: (cb) => cb,
      invoke: async (cmd, args) => {
        if (cmd === "auth_status") return { hasStoredSession: true, signedIn: true };
        if (cmd === "auth_resume") return "library";
        if (cmd === "account_services") return ["drivews", "docws"];
        if (cmd === "plugin:event|listen") return 0;
        if (cmd === "pick_open" || cmd === "plugin:dialog|open") return ["/home/me/Note.md"];
        if (cmd === "drive_children")
          return (args?.folderId ?? args?.parentId ?? args?.folder ?? null) === vault
            ? []
            : [{ drivewsid: vault, parentId: null, name: "Obsidian", extension: null,
                 kind: "APP_LIBRARY", size: null, childCount: 0, createdAt: 1780000000,
                 modifiedAt: null, etag: null }];
        if (cmd === "drive_item") return null;
        if (cmd === "drive_sync_folder") return 0;
        if (cmd === "drive_upload") { window.__uploadCalls.push(args); return []; }
        if (cmd === "sync_all" || cmd === "sync_status") return { services: [], asOf: 1, tookMs: 0 };
        return [];
      },
    };
  }, VAULT);
  const app = await appCtx.newPage();
  await app.goto("http://localhost:4173", { waitUntil: "networkidle" });
  await openScreen(app, "Drive");
  await app.locator(".drive-row-hit", { hasText: "Obsidian" }).dblclick();
  await app.waitForTimeout(400);
  const button = app.getByRole("button", { name: "Upload", exact: true });
  const offered = await button.isEnabled();
  ok(offered, "an app's library offers Upload", "Upload was disabled in an app's library");
  if (offered) {
    await button.click();
    await app.getByRole("menuitem", { name: "Files" }).click();
    await app.waitForTimeout(500);
  }
  const into = await app.evaluate(() => window.__uploadCalls.map((c) => c.folderId));
  ok(JSON.stringify(into) === JSON.stringify([VAULT]),
    "and the file goes into that library", `uploads went to ${JSON.stringify(into)}`);
  await appCtx.close();
}

await b.close();
process.exitCode = failures === 0 ? 0 : 1;
